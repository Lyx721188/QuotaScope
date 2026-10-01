//! v0's billing allowance and request rate limit, read with a v0 Platform
//! API key from `GET https://api.v0.dev/v1/user/billing` and
//! `GET https://api.v0.dev/v1/rate-limits`.
//!
//! Billing comes in two shapes, named by `billingType`: `token`, a balance
//! with a total and a remainder and a cycle end, and `legacy`, a limit and a
//! remainder. Either way the units are v0's own; nothing here calls them
//! dollars, and the on-demand balance beside the token allowance is left out
//! rather than labelled with a guess. A remainder that is not reported leaves
//! that window off — the size alone is not a percentage.
//!
//! The rate limit is best-effort: the billing allowance is the reading, and
//! a rate limit that cannot be had leaves it standing. The reply shapes are
//! second-hand (taken from CodexBar's v0 provider and the API reference it
//! cites, not from a captured reply).

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, State, Unavailability, UsageWindow};
use std::sync::Arc;

const BILLING_ENDPOINT: &str = "https://api.v0.dev/v1/user/billing";
const RATE_LIMIT_ENDPOINT: &str = "https://api.v0.dev/v1/rate-limits";

pub struct V0Service {
    http: Arc<HttpClient>,
}

impl V0Service {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for V0Service {
    fn provider(&self) -> Provider {
        Provider::V0
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::V0);
        let Some(key) = pasted_or_none(keys.api_key(Provider::V0)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let billing =
            match self
                .http
                .fetch_json(crate::http::Method::Get, BILLING_ENDPOINT, &refs, None)
            {
                Ok(value) => value,
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            };
        // A rate limit that cannot be had leaves the billing allowance
        // standing, so every failure here — refused included — is ignored.
        let rate_limit = self
            .http
            .fetch_json(crate::http::Method::Get, RATE_LIMIT_ENDPOINT, &refs, None)
            .ok();
        let mut usage = reading(&billing, rate_limit.as_ref(), account);
        if matches!(usage.state, State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

/// Used is the size less what is left, both as reported. A remainder above
/// the size reads as nothing used, not as a negative share. Both the billing
/// allowance and the rate limit are a size with a remainder and no stated
/// length, so the seconds are sort keys only.
fn quota_window(
    id: &str,
    kind: Kind,
    limit: Option<f64>,
    remaining: Option<f64>,
    reset: Option<f64>,
    seconds: i64,
) -> Option<UsageWindow> {
    let limit = limit.filter(|limit| limit.is_finite() && *limit > 0.0)?;
    let remaining = remaining.filter(|remaining| remaining.is_finite())?;
    let mut window = UsageWindow::new(
        id,
        kind,
        None,
        (limit - remaining).max(0.0) / limit,
        seconds,
        date(reset),
    );
    window.reports_length = false;
    window.is_exhausted = remaining <= 0.0;
    Some(window)
}

/// Unix time, in seconds or in milliseconds; CodexBar accepts either, and so
/// does this. Zero or less is no reset.
fn date(stamp: Option<f64>) -> Option<i64> {
    let stamp = stamp.filter(|stamp| stamp.is_finite() && *stamp > 0.0)?;
    Some(if stamp >= 1_000_000_000_000.0 {
        stamp as i64
    } else {
        (stamp * 1000.0) as i64
    })
}

pub fn reading(
    billing: &serde_json::Value,
    rate_limit: Option<&serde_json::Value>,
    account: AccountKey,
) -> ProviderUsage {
    let Some(kind) = crate::http::string_field(billing, "billingType").map(str::to_string) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    // The envelope is the shape's promise: `data` that is not an object is a
    // reply this reader does not understand, not an empty allowance.
    let (total, remaining, reset) = match kind.as_str() {
        "token" => {
            let Some(data) = crate::http::object_field(billing, "data") else {
                return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
            };
            let balance = data.get("balance");
            (
                balance.and_then(|balance| crate::http::number_field(balance, "total")),
                balance.and_then(|balance| crate::http::number_field(balance, "remaining")),
                data.get("billingCycle")
                    .and_then(|cycle| crate::http::number_field(cycle, "end")),
            )
        }
        "legacy" => {
            let Some(data) = crate::http::object_field(billing, "data") else {
                return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
            };
            (
                crate::http::number_field(data, "limit"),
                crate::http::number_field(data, "remaining"),
                crate::http::number_field(data, "reset"),
            )
        }
        _ => return ProviderUsage::unavailable(account, Unavailability::UnreadableReply),
    };

    let mut windows: Vec<UsageWindow> = Vec::new();
    // v0's own credits over a billing cycle whose length is not stated.
    // Credits, with no invented billing-cycle length.
    if let Some(window) = quota_window(
        "v0.billing",
        Kind::Credits,
        total,
        remaining,
        reset,
        30 * 86_400,
    ) {
        windows.push(window);
    }
    // Counted in requests, with a reset and no stated length. A day is only
    // where it sorts; the allowance is counted in messages.
    if let Some(data) = rate_limit {
        if let Some(window) = quota_window(
            "v0.rate_limit",
            Kind::Messages,
            crate::http::number_field(data, "limit"),
            crate::http::number_field(data, "remaining"),
            crate::http::number_field(data, "reset"),
            86_400,
        ) {
            windows.push(window);
        }
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    windows.sort_by_key(|window| window.window_seconds);
    ProviderUsage::live_now(account, windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(
        billing: serde_json::Value,
        rate_limit: Option<serde_json::Value>,
    ) -> ProviderUsage {
        reading(
            &billing,
            rate_limit.as_ref(),
            AccountKey::primary(Provider::V0),
        )
    }

    fn live_windows(usage: &ProviderUsage) -> &[UsageWindow] {
        match &usage.state {
            State::Live => &usage.windows,
            other => panic!("expected a live reading, got {other:?}"),
        }
    }

    #[test]
    fn token_billing_reads_balance_and_cycle_end() {
        let usage = reading_for(
            serde_json::json!({
                "billingType": "token",
                "data": {"balance": {"total": 100, "remaining": 25}, "billingCycle": {"end": 1_770_000_000_000i64}},
            }),
            None,
        );
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "v0.billing");
        assert!((windows[0].used_fraction - 0.75).abs() < 1e-9);
        assert!(!windows[0].reports_length);
        assert_eq!(windows[0].resets_at, Some(1_770_000_000_000));
    }

    #[test]
    fn legacy_billing_reads_limit_and_remainder() {
        let usage = reading_for(
            serde_json::json!({"billingType": "legacy", "data": {"limit": 10, "remaining": 4}}),
            None,
        );
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        assert!((windows[0].used_fraction - 0.6).abs() < 1e-9);
        assert!(!windows[0].is_exhausted);
    }

    #[test]
    fn spent_remainder_reads_as_exhausted() {
        let usage = reading_for(
            serde_json::json!({"billingType": "legacy", "data": {"limit": 10, "remaining": 0}}),
            None,
        );
        let windows = live_windows(&usage);
        assert!((windows[0].used_fraction - 1.0).abs() < 1e-9);
        assert!(windows[0].is_exhausted);
    }

    #[test]
    fn remainder_above_size_is_not_negative_use() {
        let usage = reading_for(
            serde_json::json!({"billingType": "legacy", "data": {"limit": 10, "remaining": 25}}),
            None,
        );
        let windows = live_windows(&usage);
        assert_eq!(windows[0].used_fraction, 0.0);
        assert!(!windows[0].is_exhausted);
    }

    #[test]
    fn unknown_or_missing_billing_type_is_unreadable() {
        for billing in [
            serde_json::json!({"billingType": "mystery", "data": {}}),
            serde_json::json!({}),
            serde_json::json!({"billingType": "token"}),
        ] {
            let usage = reading_for(billing, None);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn remainder_that_is_not_reported_leaves_the_window_off() {
        let usage = reading_for(
            serde_json::json!({"billingType": "legacy", "data": {"limit": 10}}),
            None,
        );
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn rate_limit_joins_the_reading_and_sorts_first() {
        let usage = reading_for(
            serde_json::json!({
                "billingType": "token",
                "data": {"balance": {"total": 100, "remaining": 25}},
            }),
            Some(serde_json::json!({"limit": 50, "remaining": 10, "reset": 1_700_000_000i64})),
        );
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 2);
        // A day sorts before the billing cycle, and the reset arrived in
        // epoch seconds.
        assert_eq!(windows[0].id, "v0.rate_limit");
        assert_eq!(windows[0].window_seconds, 86_400);
        assert_eq!(windows[0].resets_at, Some(1_700_000_000_000));
        assert_eq!(windows[1].id, "v0.billing");
    }

    #[test]
    fn failed_rate_limit_leaves_billing_standing() {
        // A rate-limit body that never parses (as the service's failure
        // paths produce) is dropped, not fatal.
        let usage = reading_for(
            serde_json::json!({
                "billingType": "legacy",
                "data": {"limit": 10, "remaining": 5},
            }),
            Some(serde_json::json!("gateway error page")),
        );
        assert_eq!(live_windows(&usage).len(), 1);
    }
}
