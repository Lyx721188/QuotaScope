//! Raycast AI: the account's AI credits — how many are left out of the
//! period's total, and when the next ones arrive.
//!
//! Read with the browser session the user imports, from the call Raycast's
//! own settings page makes:
//! `GET https://www.raycast.com/frontend_api/current_user/ai_credits`. The
//! shape is second-hand — taken from CodexBar's Raycast plugin, not from a
//! captured reply — and the fixture in the tests says so.
//!
//! **Credits, not a month.** `next_credits_at` is when the allowance renews;
//! nothing states how long the period is, so none is claimed.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button. Raycast signs in
/// at `www.raycast.com`, which keeps the session cookie and its CSRF token.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["www.raycast.com"],
    cookies: &["__raycast_session", "csrf_token"],
};

const ENDPOINT: &str = "https://www.raycast.com/frontend_api/current_user/ai_credits";

pub struct RaycastAiService {
    http: Arc<HttpClient>,
}

impl RaycastAiService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for RaycastAiService {
    fn provider(&self) -> Provider {
        Provider::RaycastAi
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::RaycastAi);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::RaycastAi)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        // The settings page's own request: same origin, its settings page as
        // the referrer.
        let headers = [
            ("Cookie", cookie),
            ("Accept", "application/json".to_string()),
            ("Origin", "https://www.raycast.com".to_string()),
            ("Referer", "https://www.raycast.com/settings".to_string()),
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
            Err(failure) => {
                return ProviderUsage::unavailable(account, session_failure(failure));
            }
        };
        match reading(&reply) {
            Ok((windows, plan)) => {
                let mut usage = ProviderUsage::live_now(account, windows);
                usage.origin = Some(self.origin_token().into());
                usage.plan = plan;
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
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

/// Raycast writes its amounts as numbers or as numeric strings; anything
/// else is no figure.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Amount {
    value: Option<f64>,
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(Self {
            value: match value {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.trim().parse::<f64>().ok(),
                _ => None,
            },
        })
    }
}

/// The tier on the reply's funding subscription, if one is named.
#[derive(Deserialize)]
struct Funding {
    #[serde(default)]
    tier: Option<String>,
}

/// The reply, decoded strictly: a field that arrived the wrong shape spoils
/// the whole reply, the way the upstream decode fails.
#[derive(Deserialize)]
struct Reply {
    #[serde(default, rename = "remaining_balance_credits")]
    remaining_balance_credits: Option<Amount>,
    #[serde(default, rename = "total_balance_credits")]
    total_balance_credits: Option<Amount>,
    #[serde(default, rename = "next_credits_at")]
    next_credits_at: Option<String>,
    #[serde(default, rename = "funding_subscription")]
    funding_subscription: Option<Funding>,
}

/// The reading, as the windows it makes and the plan's name. Both figures
/// from Raycast, or no ring: a total of zero is no allowance, and a
/// remainder alone has nothing to be a fraction of.
pub fn reading(reply: &Value) -> Result<(Vec<UsageWindow>, Option<String>), Unavailability> {
    // The decode reads an object and nothing else; a sequence would not
    // reach it.
    if !reply.is_object() {
        return Err(Unavailability::UnreadableReply);
    }
    let Ok(reply) = serde_json::from_value::<Reply>(reply.clone()) else {
        return Err(Unavailability::UnreadableReply);
    };

    let remaining = reply
        .remaining_balance_credits
        .and_then(|amount| amount.value);
    let total = reply.total_balance_credits.and_then(|amount| amount.value);

    let mut windows = Vec::new();
    if let Some(fraction) = fraction(remaining, total) {
        let mut window = UsageWindow::new(
            "raycast.ai_credits",
            // Upstream kinds the allowance `.credits`; Spend is the Windows
            // stand-in for it, as elsewhere in the port.
            Kind::Spend,
            None,
            fraction,
            30 * 86_400,
            reply
                .next_credits_at
                .as_deref()
                .and_then(crate::timeutil::parse_iso8601_ms),
        );
        // `next_credits_at` names the renewal, never the period's length.
        window.reports_length = false;
        window.is_exhausted = remaining.is_some_and(|remaining| remaining <= 0.0);
        windows.push(window);
    }

    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    let tier = reply.funding_subscription.and_then(|funding| funding.tier);
    Ok((windows, plan(tier.as_deref())))
}

/// The period's spend against its total. A negative remainder, or a total
/// that is zero, is not a ring at all.
fn fraction(remaining: Option<f64>, total: Option<f64>) -> Option<f64> {
    let remaining = remaining.filter(|value| value.is_finite() && *value >= 0.0)?;
    let total = total.filter(|value| value.is_finite() && *value > 0.0)?;
    Some(((total - remaining) / total).max(0.0))
}

/// Raycast's plan names, as its own pages write them.
pub fn plan(tier: Option<&str>) -> Option<String> {
    let tier = tier.map(str::trim).filter(|tier| !tier.is_empty())?;
    Some(match tier {
        "pro" => "Pro".to_string(),
        "pro_plus" => "Pro+".to_string(),
        "max" => "Max".to_string(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shape pinned by CodexBar's Raycast plugin — second-hand, not
    /// captured from a live account.
    #[test]
    fn the_credit_allowance_as_a_share_of_the_total_with_the_renewal_date() {
        let reply = json!({
            "remaining_balance_credits": 250,
            "total_balance_credits": 1000,
            "next_credits_at": "2026-10-01T00:00:00Z",
            "funding_subscription": {"tier": "pro_plus", "seats": 1}
        });
        let (windows, plan) = reading(&reply).unwrap();
        assert_eq!(windows.len(), 1);
        let window = &windows[0];
        assert_eq!(window.id, "raycast.ai_credits");
        // Upstream kinds this `.credits`; Spend is the Windows stand-in.
        assert_eq!(window.kind, Kind::Spend);
        assert_eq!(window.used_fraction, 0.75);
        assert_eq!(window.window_seconds, 30 * 86_400);
        assert!(!window.reports_length);
        assert_eq!(
            window.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-01T00:00:00Z")
        );
        assert_eq!(plan.as_deref(), Some("Pro+"));
    }

    #[test]
    fn amounts_may_arrive_as_strings_and_a_spent_allowance_is_exhausted() {
        let reply = json!({
            "remaining_balance_credits": " 0 ",
            "total_balance_credits": "500",
            "funding_subscription": {"tier": "pro"}
        });
        let (windows, plan) = reading(&reply).unwrap();
        assert_eq!(windows[0].used_fraction, 1.0);
        assert!(windows[0].is_exhausted);
        assert_eq!(windows[0].resets_at, None);
        assert_eq!(plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn a_used_share_is_never_negative_even_when_the_remainder_runs_past_the_total() {
        let reply = json!({"remaining_balance_credits": 600, "total_balance_credits": 500});
        let (windows, _) = reading(&reply).unwrap();
        assert_eq!(windows[0].used_fraction, 0.0);
    }

    #[test]
    fn no_total_or_a_zero_one_is_no_ring_never_zero_percent() {
        for reply in [
            json!({"remaining_balance_credits": 250}),
            json!({"remaining_balance_credits": 250, "total_balance_credits": 0}),
            json!({"remaining_balance_credits": -1, "total_balance_credits": 500}),
            json!({"remaining_balance_credits": true, "total_balance_credits": 500}),
            json!({"remaining_balance_credits": "abc", "total_balance_credits": 500}),
        ] {
            assert_eq!(
                reading(&reply),
                Err(Unavailability::NoLimitsReported),
                "{reply}"
            );
        }
    }

    #[test]
    fn a_reply_that_is_not_an_object_spoils_the_decode() {
        assert_eq!(
            reading(&json!([1, 2])),
            Err(Unavailability::UnreadableReply)
        );
        assert_eq!(reading(&json!("no")), Err(Unavailability::UnreadableReply));
        // So does a field that arrived the wrong shape: a renewal date that
        // is not a date is no reading at all, not one without a reset.
        assert_eq!(
            reading(
                &json!({"remaining_balance_credits": 1, "total_balance_credits": 2,
                            "next_credits_at": 5})
            ),
            Err(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn the_plan_names_as_raycast_writes_them() {
        assert_eq!(plan(Some("pro")).as_deref(), Some("Pro"));
        assert_eq!(plan(Some("pro_plus")).as_deref(), Some("Pro+"));
        assert_eq!(plan(Some("max")).as_deref(), Some("Max"));
        assert_eq!(plan(Some(" team ")).as_deref(), Some("team"));
        assert_eq!(plan(Some("  ")), None);
        assert_eq!(plan(None), None);
        // A subscription with no tier named draws no plan.
        let reply = json!({
            "remaining_balance_credits": 1, "total_balance_credits": 2,
            "funding_subscription": {}
        });
        assert_eq!(reading(&reply).unwrap().1, None);
    }
}
