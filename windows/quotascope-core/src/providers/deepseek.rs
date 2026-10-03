//! DeepSeek's prepaid balance, from the documented
//! `GET https://api.deepseek.com/user/balance`.
//!
//! This API reply has no allowance, reset or spend history. An explicitly
//! imported console session can supply balance as a fallback, and the
//! separate console history reader supplies account-wide daily billing. The
//! denominator behind the ring comes from somewhere else, and that somewhere
//! is the user's choice: something QuotaScope watched, a figure they typed, or
//! nothing at all.
//!
//! Every figure arrives as a **string**, including the money, and a field
//! that is absent or unparseable is **absent**, not zero — a balance read as
//! zero is a full red ring and a notification saying the account is spent.

use super::{pasted_or_none, DeepSeekBasis, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, UsageWindow};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const ENDPOINT: &str = "https://api.deepseek.com/user/balance";
static BASELINE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct DeepSeekService {
    http: Arc<HttpClient>,
}

pub struct Purse {
    pub currency: String,
    pub total: f64,
}

impl ProviderService for DeepSeekService {
    fn provider(&self) -> Provider {
        Provider::DeepSeek
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        self.fetch_with_basis(keys, &DeepSeekBasis::default())
    }
}

impl DeepSeekService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        DeepSeekService { http }
    }

    pub fn fetch_with_basis(&self, keys: &KeyRing, basis: &DeepSeekBasis) -> ProviderUsage {
        self.fetch_for_account(keys, basis, &AccountKey::primary(Provider::DeepSeek))
    }

    pub fn fetch_for_account(
        &self,
        keys: &KeyRing,
        basis: &DeepSeekBasis,
        account: &AccountKey,
    ) -> ProviderUsage {
        let key = pasted_or_none(keys.api_key(Provider::DeepSeek));
        let console = keys.deepseek_console.get(&account.id()).map(String::as_str);
        let (reply, origin) = balance_reply(
            key.as_deref(),
            console,
            |key| {
                let auth = format!("Bearer {key}");
                self.http.fetch_json(
                    crate::http::Method::Get,
                    ENDPOINT,
                    &[("Authorization", &auth), ("Accept", "application/json")],
                    None,
                )
            },
            |token| {
                let (result, _) = crate::deepseek_session::renewing(
                    token,
                    |current| {
                        let reply = crate::deepseek_session::get(
                            &self.http,
                            crate::deepseek_session::Route::Summary,
                            None,
                            current,
                        )?;
                        let balance = crate::deepseek_console::parse_summary(&reply)?;
                        if purse(&balance, basis.currency.as_deref()).is_none() {
                            return Err(crate::model::Unavailability::NoLimitsReported);
                        }
                        Ok(balance)
                    },
                    || crate::deepseek_session::renew_from_browser(account, token),
                );
                result
            },
        );
        let reply = match reply {
            Ok(reply) => reply,
            Err(reason) => return ProviderUsage::unavailable(account.clone(), reason),
        };

        let is_available = reply.get("is_available").and_then(|v| v.as_bool());
        let Some(purse) = purse(&reply, basis.currency.as_deref()) else {
            return ProviderUsage::unavailable(
                account.clone(),
                crate::model::Unavailability::NoLimitsReported,
            );
        };

        // The mark is advanced on every reading, whichever basis is in
        // force: switching to "since top-up" later should find a peak
        // already there rather than start over from whatever the balance
        // happens to be that afternoon.
        let mark = {
            let _write = BASELINE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut marks = Baseline::load();
            let mark_key = baseline_key(account, &purse.currency);
            let mark = marks.advance(&mark_key, purse.total, crate::timeutil::now_ms());
            marks.save();
            mark
        };

        let windows = windows_for(
            &purse,
            &basis.basis,
            basis.budget,
            mark.peak,
            mark.set_at,
            is_available,
        );

        let mut usage = ProviderUsage::live_now(account.clone(), windows);
        usage.origin = Some(origin.to_string());
        usage.credit_balance = Some(format!(
            "{:.2}{}",
            purse.total,
            currency_suffix(&purse.currency)
        ));
        usage.credit_remaining = Some(CreditAmount {
            amount: purse.total,
            currency: purse.currency.clone(),
        });
        usage
    }
}

fn balance_reply(
    key: Option<&str>,
    console: Option<&str>,
    mut keyed: impl FnMut(&str) -> Result<serde_json::Value, crate::model::Unavailability>,
    mut from_console: impl FnMut(&str) -> Result<serde_json::Value, crate::model::Unavailability>,
) -> (
    Result<serde_json::Value, crate::model::Unavailability>,
    &'static str,
) {
    use crate::model::Unavailability as U;
    let Some(key) = key else {
        return (
            console
                .map(&mut from_console)
                .unwrap_or(Err(U::ApiKeyMissing)),
            "webSession",
        );
    };
    let first = keyed(key);
    if matches!(
        first,
        Err(U::ApiKeyRefused | U::Unreachable | U::ServerError | U::UnreadableReply)
    ) {
        if let Some(console) = console {
            if let Ok(reply) = from_console(console) {
                return (Ok(reply), "webSession");
            }
        }
    }
    (first, "endpoint")
}

/// One currency's money, with the strings turned into numbers. **Which
/// currency the ring follows**: the user's choice if they made one, else the
/// first entry with money in it, else the first entry at all — an account
/// can hold both CNY and USD and they cannot be added together.
fn purse(reply: &serde_json::Value, preferring: Option<&str>) -> Option<Purse> {
    let infos = crate::http::array_field(reply, "balance_infos");
    let purses: Vec<Purse> = infos
        .iter()
        .filter_map(|info| {
            let currency = crate::http::string_field(info, "currency")?;
            let total = money(crate::http::string_field(info, "total_balance"))?;
            Some(Purse {
                currency: currency.to_string(),
                total,
            })
        })
        .collect();
    if purses.is_empty() {
        return None;
    }

    if let Some(preferring) = preferring {
        if let Some(chosen) = purses.iter().find(|p| p.currency == preferring) {
            return Some(Purse {
                currency: chosen.currency.clone(),
                total: chosen.total,
            });
        }
    }
    let chosen = purses.iter().find(|p| p.total > 0.0).unwrap_or(&purses[0]);
    Some(Purse {
        currency: chosen.currency.clone(),
        total: chosen.total,
    })
}

/// Money arrives as a string. A field that is absent or unparseable is
/// **absent**, not zero.
fn money(text: Option<&str>) -> Option<f64> {
    let text = text?.trim();
    if text.is_empty() {
        return None;
    }
    text.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0 && *n < 1e15)
}

fn currency_suffix(code: &str) -> String {
    match code {
        "CNY" | "RMB" => " ¥".into(),
        "USD" => " $".into(),
        other => format!(" {other}"),
    }
}

/// At most one window, because there is at most one denominator.
/// `balanceOnly` produces none at all and the rail shows the money in place
/// of a percentage. The arithmetic is the balance ring's own — the same rule
/// every prepaid account draws by — and DeepSeek adds one thing of its own:
/// **its word** on whether the balance can still pay for a call.
pub fn windows_for(
    purse: &Purse,
    basis: &str,
    budget: Option<f64>,
    peak: f64,
    since: i64,
    is_available: Option<bool>,
) -> Vec<UsageWindow> {
    let balance = CreditAmount {
        amount: purse.total,
        currency: purse.currency.clone(),
    };
    let Some(mut window) = crate::balance_ring::window(
        &balance,
        crate::balance_ring::Basis::from_token(basis),
        budget,
        peak,
    ) else {
        return Vec::new();
    };
    // **DeepSeek's own word**, not the arithmetic: `is_available` is the
    // flag it sets when the balance can no longer pay for a call. A budget
    // the reader set low can reach 100% with money still in the account, and
    // that is not the account being spent.
    window.is_exhausted = is_available == Some(false);
    let _ = since;
    vec![window]
}

fn baseline_path() -> PathBuf {
    crate::data_dir().join("deepseek-baseline.json")
}

fn baseline_key(account: &AccountKey, currency: &str) -> String {
    if account.is_primary() {
        currency.to_string()
    } else {
        format!("{}|{currency}", account.id())
    }
}

/// The highest balance QuotaScope has watched, per currency. The one measurement
/// the "since top-up" denominator is allowed to rest on.
#[derive(Default, Clone)]
struct Baseline {
    marks: HashMap<String, Mark>,
}

#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Mark {
    peak: f64,
    set_at: i64,
}

impl Baseline {
    fn load() -> Baseline {
        std::fs::read_to_string(baseline_path())
            .ok()
            .and_then(|text| serde_json::from_str::<HashMap<String, Mark>>(&text).ok())
            .map(|marks| Baseline { marks })
            .unwrap_or_default()
    }

    fn advance(&mut self, currency: &str, total: f64, now: i64) -> Mark {
        let mark = self.marks.entry(currency.to_string()).or_default();
        if total > mark.peak {
            mark.peak = total;
            mark.set_at = now;
        } else if mark.set_at == 0 {
            mark.set_at = now;
        }
        *mark
    }

    fn save(&self) {
        let dir = crate::data_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(text) = serde_json::to_string(&self.marks) {
            let _ = std::fs::write(baseline_path(), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Unavailability as U;
    use serde_json::json;

    #[test]
    fn api_success_and_rate_limit_do_not_consult_console() {
        for result in [
            Ok(json!({"balance_infos": []})),
            Err(U::RateLimited),
            Err(U::NoLimitsReported),
        ] {
            let expected = result.clone();
            let (actual, origin) = balance_reply(
                Some("api"),
                Some("session"),
                |_| result.clone(),
                |_| panic!("console must not be read"),
            );
            assert_eq!(actual, expected);
            assert_eq!(origin, "endpoint");
        }
    }

    #[test]
    fn permitted_api_failures_use_console_only_when_it_succeeds() {
        for reason in [
            U::ApiKeyRefused,
            U::Unreachable,
            U::ServerError,
            U::UnreadableReply,
        ] {
            let (reply, origin) = balance_reply(
                Some("api"),
                Some("session"),
                |_| Err(reason),
                |token| {
                    assert_eq!(token, "session");
                    Ok(json!({"balance_infos": [{"currency": "CNY", "total_balance": "3"}]}))
                },
            );
            assert_eq!(purse(&reply.unwrap(), None).unwrap().total, 3.0);
            assert_eq!(origin, "webSession");
            let (reply, origin) = balance_reply(
                Some("api"),
                Some("session"),
                |_| Err(reason),
                |_| Err(U::SessionExpired),
            );
            assert_eq!(reply, Err(reason));
            assert_eq!(origin, "endpoint");
        }
    }

    #[test]
    fn console_only_preserves_session_errors_and_missing_session_is_not_zero() {
        let (reply, origin) = balance_reply(
            None,
            Some("session"),
            |_| panic!("no API key"),
            |_| Err(U::SessionExpired),
        );
        assert_eq!(reply, Err(U::SessionExpired));
        assert_eq!(origin, "webSession");
        let (reply, _) = balance_reply(
            None,
            None,
            |_| panic!("no API key"),
            |_| panic!("no session"),
        );
        assert_eq!(reply, Err(U::ApiKeyMissing));
        for text in ["", "NaN", "inf", "-1", "1000000000000000", "oops"] {
            assert!(money(Some(text)).is_none());
        }
        assert_eq!(money(Some("0")), Some(0.0));
        assert!(purse(
            &json!({"balance_infos": [{"currency": "USD", "total_balance": "NaN"}]}),
            None
        )
        .is_none());
    }

    #[test]
    fn baseline_preserves_primary_legacy_key_and_isolates_extra_accounts_and_currencies() {
        let primary = AccountKey::primary(Provider::DeepSeek);
        let extra = AccountKey::from_id("deepSeek#1").unwrap();
        let mut baseline = Baseline::default();
        assert_eq!(baseline_key(&primary, "USD"), "USD");
        baseline.advance(&baseline_key(&primary, "USD"), 100.0, 1);
        baseline.advance(&baseline_key(&extra, "USD"), 7.0, 2);
        baseline.advance(&baseline_key(&extra, "CNY"), 12.0, 3);
        assert_eq!(
            baseline.advance(&baseline_key(&extra, "USD"), 6.0, 4).peak,
            7.0
        );
        assert_eq!(
            baseline
                .advance(&baseline_key(&primary, "USD"), 99.0, 5)
                .peak,
            100.0
        );
        assert_eq!(baseline.marks.len(), 3);
    }
}
