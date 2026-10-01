//! xKiro (not AWS's Kiro): a plan's spend windows, the daily free-token
//! allowance, and the pay-as-you-go wallet, read with a pasted key from the
//! documented usage route, `GET https://api.xkiro.com/v1/usage`, which xKiro
//! says costs nothing and counts against no limit.
//!
//! The shape is second-hand — CodexBar's xKiro plugin and the example replies
//! in xKiro's own docs, not a captured reply — and the fixture in the tests
//! says so. The free-token reset is xKiro's documented rule, midnight UTC,
//! and not a figure in the reply.
//!
//! The reply decodes as a whole or not at all, the way Swift's decoder would:
//! a field whose type drifts from the documented shape unreadables the reply
//! instead of quietly dropping the figure.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, State, Unavailability, UsageWindow,
};
use serde::Deserialize;
use std::sync::Arc;

const ENDPOINT: &str = "https://api.xkiro.com/v1/usage";

pub struct XKiroService {
    http: Arc<HttpClient>,
}

impl XKiroService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for XKiroService {
    fn provider(&self) -> Provider {
        Provider::XKiro
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::XKiro);
        let Some(key) = pasted_or_none(keys.api_key(Provider::XKiro)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self
            .http
            .fetch_json(crate::http::Method::Get, ENDPOINT, &refs, None)
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = reading(&reply, account);
        if matches!(usage.state, State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

/// xKiro writes money as fixed-point strings: `"200.000000"`.
#[derive(Deserialize)]
#[serde(untagged)]
enum Money {
    Text(String),
    Number(f64),
}

impl Money {
    fn value(&self) -> Option<f64> {
        match self {
            Money::Text(text) => text.trim().parse::<f64>().ok().filter(|v| v.is_finite()),
            Money::Number(value) => value.is_finite().then_some(*value),
        }
    }
}

#[derive(Deserialize)]
struct Window {
    kind: Option<String>,
    #[serde(rename = "window_sec")]
    window_sec: Option<i64>,
    #[serde(rename = "spent_usd")]
    spent_usd: Option<Money>,
    #[serde(rename = "cap_usd")]
    cap_usd: Option<Money>,
    #[serde(rename = "resets_in_sec")]
    resets_in_sec: Option<f64>,
}

#[derive(Deserialize)]
struct FreeTokens {
    #[serde(rename = "used_today")]
    used_today: Option<f64>,
    #[serde(rename = "limit_per_day")]
    limit_per_day: Option<f64>,
}

#[derive(Deserialize)]
struct Wallet {
    #[serde(rename = "balance_usd")]
    balance_usd: Option<Money>,
}

#[derive(Deserialize)]
struct Reply {
    object: Option<String>,
    plan: Option<String>,
    windows: Option<Vec<Window>>,
    #[serde(rename = "free_tokens")]
    free_tokens: Option<FreeTokens>,
    wallet: Option<Wallet>,
}

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    reading_at(reply, account, crate::timeutil::now_ms())
}

pub fn reading_at(reply: &serde_json::Value, account: AccountKey, now_ms: i64) -> ProviderUsage {
    // The envelope names the object; anything else — including a reply whose
    // types drift from the documented shape — is one this reader does not
    // understand.
    let Ok(reply) = serde_json::from_value::<Reply>(reply.clone()) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    if reply.object.as_deref() != Some("usage") {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }

    let mut windows: Vec<UsageWindow> = Vec::new();
    for window in reply.windows.unwrap_or_default() {
        // A whole number of hours, or it can't be named without rounding.
        let Some(seconds) = window
            .window_sec
            .filter(|seconds| *seconds >= 3_600 && seconds % 3_600 == 0)
        else {
            continue;
        };
        let Some(spent) = window
            .spent_usd
            .as_ref()
            .and_then(Money::value)
            .filter(|spent| *spent >= 0.0)
        else {
            continue;
        };
        let Some(cap) = window
            .cap_usd
            .as_ref()
            .and_then(Money::value)
            .filter(|cap| *cap > 0.0)
        else {
            continue;
        };
        let resets_at = window
            .resets_in_sec
            .filter(|secs| secs.is_finite() && *secs >= 0.0)
            .map(|secs| now_ms + (secs * 1000.0) as i64);
        let id = format!(
            "xkiro.{}",
            window.kind.unwrap_or_else(|| seconds.to_string())
        );
        let mut row = UsageWindow::new(
            &id,
            kind_of_length(seconds),
            None,
            spent / cap,
            seconds,
            resets_at,
        );
        row.is_exhausted = spent >= cap;
        windows.push(row);
    }

    // A null daily limit is "unlimited": a statement, not a denominator.
    if let Some(free) = reply.free_tokens {
        let used = free
            .used_today
            .filter(|used| used.is_finite() && *used >= 0.0);
        let limit = free
            .limit_per_day
            .filter(|limit| limit.is_finite() && *limit > 0.0);
        if let (Some(used), Some(limit)) = (used, limit) {
            let mut row = UsageWindow::new(
                "xkiro.free_tokens",
                // Upstream names the day its own `.daily` kind; the Windows
                // Kind has no daily case, and the day as `other` is the
                // stand-in the crate already uses (see aixy.rs). The length
                // is real — xKiro's documented daily boundary — so it still
                // reports one.
                Kind::Other(86_400),
                None,
                used / limit,
                86_400,
                next_midnight_utc_ms(now_ms),
            );
            row.is_exhausted = used >= limit;
            windows.push(row);
        }
    }

    let balance = reply
        .wallet
        .and_then(|wallet| wallet.balance_usd)
        .and_then(|money| money.value());
    if windows.is_empty() && balance.is_none() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }

    let plan = reply
        .plan
        .map(|plan| plan.trim().to_string())
        .filter(|plan| !plan.is_empty())
        .map(|plan| capitalized(&plan));
    // Shortest first, so the row order follows the windows' own lengths.
    windows.sort_by_key(|window| window.window_seconds);
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.plan = plan;
    if let Some(balance) = balance {
        usage.credit_balance = Some(money_text(balance));
        usage.credit_remaining = Some(CreditAmount {
            amount: balance,
            currency: "USD".into(),
        });
    }
    usage
}

/// Named by the length xKiro states: five hours and a week are the two it
/// documents, a day is the crate's `.daily` stand-in, and anything else by
/// its number of seconds.
pub fn kind_of_length(seconds: i64) -> Kind {
    const FIVE_HOURS: i64 = 5 * 3_600;
    const A_DAY: i64 = 86_400;
    const A_WEEK: i64 = 7 * 86_400;
    match seconds {
        FIVE_HOURS => Kind::FiveHour,
        A_DAY => Kind::Other(A_DAY),
        A_WEEK => Kind::Weekly,
        other => Kind::Other(other),
    }
}

/// xKiro's documented free-token boundary: the next midnight UTC, strictly
/// after `now_ms`.
pub fn next_midnight_utc_ms(now_ms: i64) -> Option<i64> {
    use chrono::{TimeZone, Utc};
    let now = Utc.timestamp_millis_opt(now_ms).single()?;
    let tomorrow = now.date_naive().succ_opt()?;
    let midnight = tomorrow.and_hms_opt(0, 0, 0)?;
    Some(Utc.from_utc_datetime(&midnight).timestamp_millis())
}

/// Swift's `capitalized`: each word's first letter up, the rest down, word
/// starts after any non-alphanumeric — enough for the plan names the source
/// spells.
pub fn capitalized(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if at_word_start {
                out.extend(ch.to_uppercase());
            } else {
                out.extend(ch.to_lowercase());
            }
            at_word_start = false;
        } else {
            at_word_start = true;
            out.push(ch);
        }
    }
    out
}

/// The balance as money — symbol first, cents kept. A negative balance is
/// money owed, and the sign leads the figure the way a currency formatter
/// writes it.
fn money_text(amount: f64) -> String {
    if amount < 0.0 {
        format!("-${:.2}", -amount)
    } else {
        format!("${amount:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn reading_at_for(value: serde_json::Value, now_ms: i64) -> ProviderUsage {
        reading_at(&value, AccountKey::primary(Provider::XKiro), now_ms)
    }

    const NOW: i64 = 1_790_000_000_000; // a fixed "now" for the tests

    fn full_reply() -> serde_json::Value {
        json!({
            "object": "usage",
            "plan": "pro plus",
            "windows": [
                {"kind": "weekly", "window_sec": 604800, "spent_usd": "12.000000", "cap_usd": 40, "resets_in_sec": 3600},
                {"kind": "five_hour", "window_sec": 18000, "spent_usd": 2.5, "cap_usd": "10.000000", "resets_in_sec": 600},
            ],
            "free_tokens": {"used_today": 300, "limit_per_day": 1000},
            "wallet": {"balance_usd": "200.000000"},
        })
    }

    #[test]
    fn the_documented_reply_reads_in_full() {
        let usage = reading_at_for(full_reply(), NOW);
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.plan.as_deref(), Some("Pro Plus"));
        let windows = &usage.windows;
        // Shortest window first.
        assert_eq!(
            windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["xkiro.five_hour", "xkiro.free_tokens", "xkiro.weekly"]
        );
        let weekly = &windows[2];
        assert_eq!(weekly.kind, Kind::Weekly);
        assert!((weekly.used_fraction - 0.3).abs() < 1e-9);
        assert_eq!(weekly.resets_at, Some(NOW + 3_600_000));
        assert!(!weekly.is_exhausted);
        let free = &windows[1];
        assert_eq!(free.kind, Kind::Other(86_400));
        assert!((free.used_fraction - 0.3).abs() < 1e-9);
        assert_eq!(free.resets_at, next_midnight_utc_ms(NOW));
        assert_eq!(usage.credit_balance.as_deref(), Some("$200.00"));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 200.0,
                currency: "USD".into(),
            })
        );
    }

    #[test]
    fn window_lengths_name_their_kinds() {
        for (seconds, kind) in [
            (5 * 3_600, Kind::FiveHour),
            (86_400, Kind::Other(86_400)),
            (7 * 86_400, Kind::Weekly),
            (3 * 3_600, Kind::Other(3 * 3_600)),
        ] {
            assert_eq!(kind_of_length(seconds), kind, "{seconds}");
        }
    }

    #[test]
    fn a_window_that_cannot_be_named_without_rounding_is_skipped() {
        let mut reply = full_reply();
        reply["windows"] = json!([
            {"kind": "odd", "window_sec": 5400, "spent_usd": 1, "cap_usd": 2},
            {"kind": "short", "window_sec": 1800, "spent_usd": 1, "cap_usd": 2},
        ]);
        let usage = reading_at_for(reply, NOW);
        // Both gone, but the wallet and the free tokens still read.
        assert!(usage.windows.iter().all(|w| w.id != "xkiro.odd"));
        assert!(matches!(usage.state, State::Live));
    }

    #[test]
    fn a_drifting_type_unreadables_the_whole_reply() {
        // Upstream's decoder would throw on any of these; so does the port.
        for reply in [
            json!({"object": "usage", "windows": [{"window_sec": "18000", "spent_usd": 1, "cap_usd": 2}]}),
            json!({"object": "usage", "free_tokens": {"used_today": "300", "limit_per_day": 1000}}),
            json!({"object": "usage", "wallet": {"balance_usd": true}}),
            json!({"object": "usage", "windows": "none"}),
        ] {
            let usage = reading_at_for(reply, NOW);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn the_envelope_must_name_the_object() {
        for object in [json!({"plan": "pro"}), json!({"object": "other"})] {
            let usage = reading_at_for(object, NOW);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn a_null_daily_limit_is_unlimited_not_zero() {
        let reply = json!({
            "object": "usage",
            "free_tokens": {"used_today": 300, "limit_per_day": null},
            "wallet": {"balance_usd": 5},
        });
        let usage = reading_at_for(reply, NOW);
        assert!(usage.windows.is_empty());
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 5.0,
                currency: "USD".into(),
            })
        );
    }

    #[test]
    fn exhausted_when_the_allowance_reaches_its_cap() {
        let reply = json!({
            "object": "usage",
            "windows": [{"kind": "five_hour", "window_sec": 18000, "spent_usd": 10, "cap_usd": 10}],
            "free_tokens": {"used_today": 1000, "limit_per_day": 1000},
            "wallet": {"balance_usd": 0},
        });
        let usage = reading_at_for(reply, NOW);
        assert!(usage.windows.iter().all(|w| w.is_exhausted));
    }

    #[test]
    fn nothing_understood_reports_no_limits() {
        let usage = reading_at_for(json!({"object": "usage"}), NOW);
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn a_negative_wallet_reads_as_money_owed() {
        let usage = reading_at_for(
            json!({"object": "usage", "wallet": {"balance_usd": "-1.25"}}),
            NOW,
        );
        assert_eq!(usage.credit_balance.as_deref(), Some("-$1.25"));
    }

    #[test]
    fn the_next_midnight_utc_is_strictly_after_now() {
        // 2026-09-30T23:59:59.999Z → 2026-10-01T00:00:00Z
        let before = 1_790_812_799_999;
        assert_eq!(next_midnight_utc_ms(before), Some(1_790_812_800_000));
        // Exactly midnight rolls to the next day, the way "after" reads.
        assert_eq!(
            next_midnight_utc_ms(1_790_812_800_000),
            Some(1_790_899_200_000)
        );
    }

    #[test]
    fn capitalized_follows_swifts_word_rule() {
        assert_eq!(capitalized("pro plus"), "Pro Plus");
        assert_eq!(capitalized("PRO"), "Pro");
        assert_eq!(capitalized("max-pro"), "Max-Pro");
        assert_eq!(capitalized(""), "");
    }
}
