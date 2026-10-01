//! Perplexity: the API credit on the account, and the monthly credit that
//! comes with a subscription when it is the only credit there is.
//!
//! Read with the browser session the user imports, from the route the
//! account's usage page calls:
//! `GET https://www.perplexity.ai/rest/billing/credits?version=2.18&source=default`.
//! The shape is second-hand — taken from CodexBar's Perplexity plugin and its
//! tests, not from a captured reply — and the fixture in the tests says so.
//!
//! **Money, in cents.** Every figure is a count of US cents: the balance, each
//! grant, the total used. The balance is shown as a balance.
//!
//! **A ring only where the split is stated.** Perplexity reports how much each
//! grant was and how much was used in total — not which grant it was taken
//! from. CodexBar spends the total down the subscription's grant first, then
//! purchased, then bonus, and draws a ring for each; that order is its guess.
//! This draws the subscription's ring only when that grant is the only one on
//! the account, where the total can have come from nowhere else, and draws
//! none at all when there is no such grant — never a full ring standing in
//! for one that is missing.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["www.perplexity.ai"],
    cookies: &["__Secure-next-auth.session-token"],
};

const ENDPOINT: &str = "https://www.perplexity.ai/rest/billing/credits?version=2.18&source=default";

pub struct PerplexityService {
    http: Arc<HttpClient>,
}

impl PerplexityService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for PerplexityService {
    fn provider(&self) -> Provider {
        Provider::Perplexity
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Perplexity);
        let Some(header) = pasted_or_none(keys.api_key(Provider::Perplexity)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        // Only the sign-in cookie is ever sent, and a header without one is a
        // session that was never really imported.
        let Some(cookie) = keep(&header, SESSION.cookies) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let headers = [
            ("Cookie", cookie),
            ("Accept", "application/json".to_string()),
            // What the usage page's own request carries; the site sits behind
            // a bot screen that turns away a request that looks like no
            // browser.
            ("Origin", "https://www.perplexity.ai".to_string()),
            (
                "Referer",
                "https://www.perplexity.ai/account/usage".to_string(),
            ),
            (
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36"
                    .to_string(),
            ),
        ];
        let header_refs: Vec<(&str, &str)> =
            headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self.http.fetch_json_detailed(
            crate::http::Method::Get,
            ENDPOINT,
            &header_refs,
            None,
        ) {
            Ok(value) => value,
            // A redirect or a refusal is the session no longer working.
            Err(failure) => return ProviderUsage::unavailable(account, session_failure(failure)),
        };
        let usage = reading(&reply, crate::timeutil::now_ms());
        if matches!(usage.state, crate::model::State::Live) {
            return ProviderUsage {
                origin: Some(self.origin_token().into()),
                ..usage
            };
        }
        usage
    }
}

/// The shared fetch folds 401/403 into the refused key; a browser session is
/// refused the same way and means the same thing: import it again.
fn session_failure(failure: crate::http::HttpFailure) -> Unavailability {
    match failure {
        crate::http::HttpFailure::NotFound => Unavailability::ServerError,
        crate::http::HttpFailure::Unavailable(Unavailability::ApiKeyRefused) => {
            Unavailability::SessionExpired
        }
        crate::http::HttpFailure::Unavailable(reason) => reason,
    }
}

/// Keeps only the named cookies out of a browser's `name=value; …` header,
/// and nothing at all if the first name is missing. What is not kept never
/// leaves the process.
///
/// A name ending in `*` is a prefix, for a service whose session cookie
/// carries a suffix of its own — Mistral's is `ory_session_` and then the
/// deployment's id. The prefix has to be followed by something; `*` alone
/// would keep every cookie the browser holds for the host.
///
/// Names joined with `|` are alternatives, for a service whose session can
/// sit under any one of several names. In the first entry that means any one
/// of them is enough; every one of them is kept.
pub fn keep(header: &str, cookies: &[&str]) -> Option<String> {
    let first = cookies.first()?;
    let required: Vec<&str> = first.split('|').collect();
    let patterns: Vec<&str> = cookies.iter().flat_map(|c| c.split('|')).collect();
    let matches = |name: &str, pattern: &str| -> bool {
        match pattern.strip_suffix('*') {
            Some(prefix) => {
                !prefix.is_empty() && name.starts_with(prefix) && name.len() > prefix.len()
            }
            None => name == pattern,
        }
    };
    let pairs: Vec<(&str, &str)> = header
        .split(';')
        .filter_map(|part| {
            let trimmed = part.trim();
            let equals = trimmed.find('=')?;
            let (name, value) = (&trimmed[..equals], &trimmed[equals + 1..]);
            patterns
                .iter()
                .any(|pattern| matches(name, pattern))
                .then_some((name, value))
        })
        .collect();
    if !pairs
        .iter()
        .any(|(name, _)| required.iter().any(|want| matches(name, want)))
    {
        return None;
    }
    let mut seen: Vec<&str> = Vec::new();
    let kept: Vec<String> = pairs
        .into_iter()
        .filter(|(name, _)| {
            if seen.contains(name) {
                return false;
            }
            seen.push(name);
            true
        })
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    Some(kept.join("; "))
}

/// A figure: a finite number. Never a boolean, which a lenient reader would
/// otherwise take as 0 or 1.
fn figure(value: Option<&Value>) -> Option<f64> {
    value?.as_f64().filter(|v| v.is_finite())
}

/// Perplexity has answered with both spellings.
fn field<'a>(object: &'a Value, snake: &str, camel: &str) -> Option<&'a Value> {
    object.get(snake).or_else(|| object.get(camel))
}

pub fn reading(reply: &Value, now_ms: i64) -> ProviderUsage {
    let account = AccountKey::primary(Provider::Perplexity);
    let Some(object) = reply.as_object() else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let Some(grants) = field(reply, "credit_grants", "creditGrants")
        .and_then(|v| v.as_array())
        // A grant that is not an object reads no better than no grant.
        .filter(|grants| grants.iter().all(|grant| grant.is_object()))
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    // A grant that has lapsed is no longer part of anything.
    let live: Vec<&Value> = grants
        .iter()
        .filter(
            |grant| match figure(field(grant, "expires_at_ts", "expiresAtTs")) {
                Some(expiry) => (expiry * 1000.0) as i64 > now_ms,
                None => true,
            },
        )
        .collect();
    let amount = |grant: &Value| figure(field(grant, "amount_cents", "amountCents"));
    let recurring: f64 = live
        .iter()
        .filter(|grant| field(grant, "type", "type").and_then(|v| v.as_str()) == Some("recurring"))
        .filter_map(|grant| amount(grant))
        .sum();
    let others: f64 = live
        .iter()
        .filter(|grant| field(grant, "type", "type").and_then(|v| v.as_str()) != Some("recurring"))
        .filter_map(|grant| amount(grant))
        .sum();
    let purchased = figure(
        object
            .get("current_period_purchased_cents")
            .or_else(|| object.get("currentPeriodPurchasedCents")),
    )
    .unwrap_or(0.0);
    let used = figure(field(reply, "total_usage_cents", "totalUsageCents"));

    let mut windows: Vec<UsageWindow> = Vec::new();
    if recurring > 0.0 && others <= 0.0 && purchased <= 0.0 {
        if let Some(used) = used.filter(|used| *used >= 0.0) {
            // Upstream kinds this `.credits` — an allowance counted in
            // credits, no length it claims. The Windows kind set has no
            // credits case, so Spend stands in, the way LiteLLM's shared
            // pool does.
            let renewal = figure(field(reply, "renewal_date_ts", "renewalDateTs"))
                .filter(|at| *at > 0.0)
                .map(crate::timeutil::epoch_to_ms);
            let mut window = UsageWindow::new(
                "perplexity.recurring",
                Kind::Credits,
                None,
                used / recurring,
                // A sort key: the renewal is stated, the period's length is not.
                30 * 86_400,
                renewal,
            );
            window.reports_length = false;
            window.is_exhausted = used >= recurring;
            windows.push(window);
        }
    }

    let balance = figure(field(reply, "balance_cents", "balanceCents"))
        .filter(|cents| *cents >= 0.0)
        .map(|cents| cents / 100.0);
    if windows.is_empty() && balance.is_none() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let mut read = ProviderUsage::live_now(account, windows);
    read.credit_balance = balance.map(money);
    read.credit_remaining = balance.map(|amount| CreditAmount {
        amount,
        currency: "USD".into(),
    });
    read
}

/// Cents, as dollars.
pub fn money(amount: f64) -> String {
    format!("${amount:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    /// Before every expiry in the fixtures.
    const NOW_MS: i64 = 1_788_000_000_000;

    fn reading_at(value: Value) -> ProviderUsage {
        reading(&value, NOW_MS)
    }

    /// Upstream's `perplexity-credits` fixture — second-hand, written from
    /// CodexBar's Perplexity plugin, not captured from a live account.
    fn credits() -> Value {
        json!({
            "version": "2.18",
            "balance_cents": 320,
            "renewal_date_ts": 1790812800,
            "current_period_purchased_cents": 0,
            "credit_grants": [
                {"type": "recurring", "amount_cents": 500, "expires_at_ts": 1790812800}
            ],
            "total_usage_cents": 180
        })
    }

    #[test]
    fn the_subscriptions_credit_when_it_is_the_only_credit_and_the_balance_in_dollars() {
        let usage = reading_at(credits());
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.windows.len(), 1);
        let window = &usage.windows[0];
        assert_eq!(window.used_fraction, 180.0 / 500.0);
        assert!(!window.reports_length);
        assert_eq!(window.resets_at, Some(1_790_812_800_000));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 3.2,
                currency: "USD".into()
            })
        );
        assert_eq!(usage.credit_balance.as_deref(), Some("$3.20"));
    }

    #[test]
    fn beside_a_bonus_or_a_purchase_which_grant_was_spent_isnt_stated_no_ring() {
        for value in [
            json!({"balance_cents": 7250, "renewal_date_ts": 1790812800,
                   "current_period_purchased_cents": 0,
                   "credit_grants": [
                       {"type": "recurring", "amount_cents": 10000},
                       {"type": "promotional", "amount_cents": 20000, "expires_at_ts": 1800000000}],
                   "total_usage_cents": 2750}),
            json!({"balance_cents": 0, "renewal_date_ts": 1790812800,
                   "current_period_purchased_cents": 3000,
                   "credit_grants": [{"type": "recurring", "amount_cents": 5000}],
                   "total_usage_cents": 8000}),
        ] {
            let usage = reading_at(value);
            assert!(usage.windows.is_empty());
            assert!(usage.credit_remaining.is_some());
        }
    }

    #[test]
    fn a_bonus_that_has_lapsed_is_no_longer_beside_it() {
        let usage = reading_at(json!({
            "balance_cents": 100,
            "credit_grants": [
                {"type": "recurring", "amount_cents": 1000},
                {"type": "promotional", "amount_cents": 500, "expires_at_ts": 1700000000}],
            "total_usage_cents": 250
        }));
        assert_eq!(usage.windows[0].used_fraction, 0.25);
    }

    #[test]
    fn no_subscription_credit_no_ring_never_a_full_one_standing_in() {
        let usage = reading_at(json!({
            "balance_cents": 0, "renewal_date_ts": 1790812800,
            "current_period_purchased_cents": 0,
            "credit_grants": [], "total_usage_cents": 0
        }));
        assert!(usage.windows.is_empty());
        assert_eq!(usage.credit_remaining.map(|c| c.amount), Some(0.0));
    }

    #[test]
    fn camel_case_spellings_are_read_too() {
        let usage = reading_at(json!({
            "balanceCents": 500, "renewalDateTs": 1790812800,
            "creditGrants": [{"type": "recurring", "amountCents": 500}],
            "totalUsageCents": 100
        }));
        assert_eq!(usage.windows[0].used_fraction, 0.2);
        assert_eq!(usage.credit_remaining.map(|c| c.amount), Some(5.0));
    }

    #[test]
    fn a_reply_that_isnt_a_credits_reply_cant_be_read() {
        for value in [json!({}), json!({"balance_cents": 5})] {
            assert_eq!(
                reading_at(value).state,
                State::Unavailable(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn figures_that_arent_figures_are_left_off_and_nothing_left_is_no_limits() {
        let usage = reading_at(json!({
            "balance_cents": -4,
            "credit_grants": [{"type": "recurring", "amount_cents": 500}],
            "total_usage_cents": -1
        }));
        assert_eq!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn only_the_sign_in_cookie_is_kept_and_not_without_it() {
        assert_eq!(
            keep(
                "__cf_bm=x; __Secure-next-auth.session-token=good; theme=dark",
                SESSION.cookies
            ),
            Some("__Secure-next-auth.session-token=good".to_string())
        );
        // A header without the sign-in cookie keeps nothing at all.
        assert_eq!(keep("next-auth.csrf-token=abc", SESSION.cookies), None);
        assert_eq!(keep("", SESSION.cookies), None);
    }
}
