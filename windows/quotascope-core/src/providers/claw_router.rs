//! ClawRouter, OpenClaw's routing gateway: the monthly budget on the policy a
//! key belongs to, as an amount spent and a limit the service states.
//!
//! Read with a ClawRouter key the user pastes, from the hosted service's
//! `GET https://clawrouter.openclaw.ai/v1/usage`. The shape is second-hand —
//! taken from CodexBar's ClawRouter provider and its tests, not from a
//! captured reply — and the test here says so.
//!
//! Money is in micro-dollars. A policy with no budget (`configured: false`)
//! reports spend and requests and nothing to measure them against, so it is
//! "no limits reported" rather than a ring at zero. The budget's month is
//! named (`…/2026-07`) but not when, or in which zone, it turns over, so no
//! reset is inferred from it. The per-provider breakdown is spend with no
//! limit and is left out.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://clawrouter.openclaw.ai/v1/usage";

pub struct ClawRouterService {
    http: Arc<HttpClient>,
}

impl ClawRouterService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for ClawRouterService {
    fn provider(&self) -> Provider {
        Provider::ClawRouter
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::ClawRouter);
        let Some(key) = pasted_or_none(keys.api_key(Provider::ClawRouter)) else {
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
        match reading(&reply) {
            Ok(window) => {
                let mut usage = ProviderUsage::live_now(account, vec![window]);
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// The monthly budget window, when the policy has a budget to read.
pub fn reading(reply: &serde_json::Value) -> Result<UsageWindow, Unavailability> {
    let Some(budget) = crate::http::object_field(reply, "budget") else {
        return Err(Unavailability::UnreadableReply);
    };
    let Some(configured) = crate::http::bool_field(budget, "configured") else {
        return Err(Unavailability::UnreadableReply);
    };
    if !configured {
        return Err(Unavailability::NoLimitsReported);
    }
    let limit = crate::http::number_field(budget, "limitMicros");
    let spent = crate::http::number_field(budget, "spentMicros");
    let (Some(limit), Some(spent)) = (limit, spent) else {
        return Err(Unavailability::NoLimitsReported);
    };
    if limit <= 0.0 || spent < 0.0 {
        return Err(Unavailability::NoLimitsReported);
    }
    let fraction = spent / limit;
    let mut window = UsageWindow::new(
        "clawrouter.monthly",
        Kind::Monthly,
        None,
        fraction,
        // A calendar month: a sort key, not a stated length.
        30 * 86_400,
        None,
    );
    window.reports_length = false;
    window.is_exhausted = fraction >= 1.0;
    Ok(window)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micro_dollars_become_one_monthly_window() {
        let reply = serde_json::json!({
            "budget": {"configured": true, "limitMicros": 10_000_000, "spentMicros": 2_500_000},
            "providers": {"openai": {"spentMicros": 2_500_000}},
        });
        let window = reading(&reply).unwrap();
        assert_eq!(window.id, "clawrouter.monthly");
        assert_eq!(window.kind, Kind::Monthly);
        assert_eq!(window.used_fraction, 0.25);
        // A calendar month: a sort key, not a stated length.
        assert!(!window.reports_length);
        assert!(window.resets_at.is_none());
        assert!(!window.is_exhausted);
    }

    #[test]
    fn spent_past_the_limit_is_exhausted_and_drawn_over_one() {
        let reply = serde_json::json!({
            "budget": {"configured": true, "limitMicros": 100, "spentMicros": 125},
        });
        let window = reading(&reply).unwrap();
        assert_eq!(window.used_fraction, 1.25);
        assert!(window.is_exhausted);
    }

    #[test]
    fn a_policy_without_a_budget_reports_no_limits() {
        let reply = serde_json::json!({"budget": {"configured": false, "spentMicros": 500}});
        assert_eq!(
            reading(&reply).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn a_budget_with_no_usable_limit_reports_no_limits() {
        for budget in [
            serde_json::json!({"configured": true}),
            serde_json::json!({"configured": true, "limitMicros": 0, "spentMicros": 1}),
            serde_json::json!({"configured": true, "limitMicros": 100}),
            serde_json::json!({"configured": true, "limitMicros": 100, "spentMicros": -1}),
        ] {
            let reply = serde_json::json!({"budget": budget});
            assert_eq!(
                reading(&reply).unwrap_err(),
                Unavailability::NoLimitsReported,
                "budget: {budget}"
            );
        }
    }

    #[test]
    fn a_reply_without_a_budget_is_unreadable() {
        assert_eq!(
            reading(&serde_json::json!({})).unwrap_err(),
            Unavailability::UnreadableReply
        );
        let reply = serde_json::json!({"budget": {}});
        assert_eq!(
            reading(&reply).unwrap_err(),
            Unavailability::UnreadableReply
        );
    }
}
