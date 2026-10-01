//! DevPass, LLM Gateway's subscription: the billing cycle's plan credits, the
//! premium models' weekly allowance, and — where the key has one — the key's
//! own spending limit. Every allowance and every amount used is stated by the
//! service, in dollars of credit.
//!
//! Read with a regular LLM Gateway API key the user pastes, from
//! `GET https://api.llmgateway.io/v1/key`. The shape is second-hand — taken
//! from CodexBar's DevPass provider, its tests and the public usage API it
//! cites, not from a captured reply — and the tests here say so.
//!
//! The cycle's end is not in the reply, so that allowance has no reset and no
//! length; neither is inferred. An allowance of zero is no allowance and is
//! left off rather than drawn full or empty.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.llmgateway.io/v1/key";

pub struct DevPassService {
    http: Arc<HttpClient>,
}

impl DevPassService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for DevPassService {
    fn provider(&self) -> Provider {
        Provider::DevPass
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::DevPass);
        let Some(key) = pasted_or_none(keys.api_key(Provider::DevPass)) else {
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
        let (windows, plan) = match reading(&reply) {
            Ok(reading) => reading,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.origin = Some(self.origin_token().into());
        usage.plan = plan;
        usage
    }
}

/// The plan names the service sends; anything else is a shape this build
/// does not know.
const PLANS: [&str; 4] = ["none", "lite", "pro", "max"];

pub fn reading(
    reply: &serde_json::Value,
) -> Result<(Vec<UsageWindow>, Option<String>), Unavailability> {
    let Some(key) = crate::http::object_field(reply, "data") else {
        return Err(Unavailability::UnreadableReply);
    };
    let Some(plan) = crate::http::string_field(key, "devPlan") else {
        return Err(Unavailability::UnreadableReply);
    };
    if !PLANS.contains(&plan) {
        return Err(Unavailability::UnreadableReply);
    }

    let mut windows = Vec::new();
    if plan != "none" {
        // Seven days from the first premium request — a stated length.
        if let Some(window) = window(
            "devpass.premium_weekly",
            Kind::Weekly,
            amount(crate::http::string_field(key, "devPlanPremiumCreditsUsed")),
            amount(crate::http::string_field(key, "devPlanPremiumWeeklyLimit")),
            7 * 86_400,
            true,
            crate::http::string_field(key, "devPlanPremiumWeekResetsAt")
                .and_then(crate::timeutil::parse_iso8601_ms),
        ) {
            windows.push(window);
        }
        // The billing cycle's credits. No end is reported and no length is
        // claimed; thirty days only sorts it after the week. Upstream calls
        // this a credit allowance; the Windows model has no credits kind, so
        // the month is a sort key and the row says so.
        if let Some(window) = window(
            "devpass.cycle",
            Kind::Other(30 * 86_400),
            amount(crate::http::string_field(key, "devPlanCreditsUsed")),
            amount(crate::http::string_field(key, "devPlanCreditsLimit")),
            30 * 86_400,
            false,
            None,
        ) {
            windows.push(window);
        }
    }
    // The key's own spending limit, against everything it has ever spent.
    // Never resets, so it has no clock and sorts last.
    if let Some(window) = window(
        "devpass.key_limit",
        Kind::Spend,
        amount(crate::http::string_field(key, "usage")),
        amount(crate::http::string_field(key, "limit")),
        365 * 86_400,
        false,
        None,
    ) {
        windows.push(window);
    }

    if windows.is_empty() {
        // Pay as you go with no key limit: an answer, not an outage. Upstream
        // files it under its own "no plan" word; the Windows vocabulary has
        // no such case, so it shares this one.
        return Err(Unavailability::NoLimitsReported);
    }
    let name = (plan != "none").then(|| capitalize(plan));
    Ok((windows, name))
}

/// A window from an amount used and an allowance, both as DevPass wrote them.
/// Nil when either is missing, unreadable or negative, or when the allowance
/// is zero.
#[allow(clippy::too_many_arguments)]
fn window(
    id: &str,
    kind: Kind,
    used: Option<f64>,
    limit: Option<f64>,
    seconds: i64,
    reports_length: bool,
    resets_at: Option<i64>,
) -> Option<UsageWindow> {
    let used = used?;
    let limit = limit?;
    if limit <= 0.0 {
        return None;
    }
    let fraction = used / limit;
    let mut window = UsageWindow::new(id, kind, None, fraction, seconds, resets_at);
    window.reports_length = reports_length;
    window.is_exhausted = fraction >= 1.0;
    Some(window)
}

/// Amounts arrive as decimal strings, "212.00".
fn amount(text: Option<&str>) -> Option<f64> {
    text?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/// "lite" reads as "Lite".
fn capitalize(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paid_plan_reports_three_windows() {
        let reply = serde_json::json!({
            "data": {
                "devPlan": "pro",
                "devPlanCreditsUsed": "212.00",
                "devPlanCreditsLimit": "500.00",
                "devPlanPremiumCreditsUsed": "18.00",
                "devPlanPremiumWeeklyLimit": "50.00",
                "devPlanPremiumWeekResetsAt": "2026-10-05T00:00:00Z",
                "usage": "312.00",
                "limit": "1000.00",
            },
        });
        let (windows, plan) = reading(&reply).unwrap();
        assert_eq!(plan.as_deref(), Some("Pro"));
        assert_eq!(windows.len(), 3);

        let weekly = &windows[0];
        assert_eq!(weekly.id, "devpass.premium_weekly");
        assert_eq!(weekly.kind, Kind::Weekly);
        assert_eq!(weekly.used_fraction, 18.0 / 50.0);
        // Seven days from the first premium request — a stated length.
        assert!(weekly.reports_length);
        assert!(weekly.resets_at.is_some());

        let cycle = &windows[1];
        assert_eq!(cycle.id, "devpass.cycle");
        assert_eq!(cycle.used_fraction, 212.0 / 500.0);
        assert!(!cycle.reports_length);
        assert!(cycle.resets_at.is_none());

        let key_limit = &windows[2];
        assert_eq!(key_limit.id, "devpass.key_limit");
        assert_eq!(key_limit.kind, Kind::Spend);
        assert_eq!(key_limit.used_fraction, 312.0 / 1000.0);
        assert!(!key_limit.reports_length);
        assert!(key_limit.resets_at.is_none());
    }

    #[test]
    fn a_spent_allowance_is_marked_exhausted() {
        let reply = serde_json::json!({
            "data": {
                "devPlan": "max",
                "devPlanPremiumCreditsUsed": "50.00",
                "devPlanPremiumWeeklyLimit": "50.00",
            },
        });
        let (windows, plan) = reading(&reply).unwrap();
        assert_eq!(plan.as_deref(), Some("Max"));
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "devpass.premium_weekly");
        assert!(windows[0].is_exhausted);
    }

    #[test]
    fn an_allowance_of_zero_is_left_off() {
        let reply = serde_json::json!({
            "data": {
                "devPlan": "lite",
                "devPlanPremiumCreditsUsed": "0.00",
                "devPlanPremiumWeeklyLimit": "0.00",
                "usage": "5.00",
                "limit": "20.00",
            },
        });
        let (windows, plan) = reading(&reply).unwrap();
        assert_eq!(plan.as_deref(), Some("Lite"));
        assert_eq!(
            windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["devpass.key_limit"]
        );
    }

    #[test]
    fn pay_as_you_go_reports_only_the_key_limit() {
        let reply = serde_json::json!({
            "data": {"devPlan": "none", "usage": "5.00", "limit": "20.00"},
        });
        let (windows, plan) = reading(&reply).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "devpass.key_limit");
        assert!(plan.is_none());
    }

    #[test]
    fn nothing_to_show_is_an_answer_not_an_outage() {
        let reply = serde_json::json!({"data": {"devPlan": "none"}});
        assert_eq!(
            reading(&reply).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn an_unknown_plan_or_shape_is_unreadable() {
        for reply in [
            serde_json::json!({"data": {"devPlan": "ultra"}}),
            serde_json::json!({"data": {}}),
            serde_json::json!({}),
            serde_json::json!({"data": "nope"}),
        ] {
            assert_eq!(
                reading(&reply).unwrap_err(),
                Unavailability::UnreadableReply,
                "reply: {reply}"
            );
        }
    }

    #[test]
    fn a_negative_or_unreadable_amount_drops_its_window() {
        let reply = serde_json::json!({
            "data": {
                "devPlan": "pro",
                "devPlanPremiumCreditsUsed": "-1.00",
                "devPlanPremiumWeeklyLimit": "50.00",
                "usage": "not a number",
                "limit": "20.00",
            },
        });
        // Every allowance dropped, none left to show.
        assert_eq!(
            reading(&reply).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }
}
