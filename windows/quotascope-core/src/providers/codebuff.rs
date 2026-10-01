//! Codebuff, the coding agent: its credit allowance, the credits left, and —
//! for the CLI's own login — the weekly rate limit.
//!
//! - `POST https://www.codebuff.com/api/v1/usage` — the credits used, the
//!   quota, the balance and the next reset. A POST because that is how the
//!   CLI asks; it carries only a fingerprint id and changes nothing.
//! - `GET https://www.codebuff.com/api/user/subscription` — the tier and the
//!   weekly limit. Upstream asks it only with the CLI's saved login, as
//!   CodexBar does: an API key reads the credits alone. This port holds only
//!   pasted keys, so the subscription reply is parsed but never fetched.
//!
//! The shapes are second-hand — taken from CodexBar's Codebuff provider and
//! its tests, not from captured replies — and the tests here say so.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{HttpClient, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const USAGE_ENDPOINT: &str = "https://www.codebuff.com/api/v1/usage";
/// A sort key: no period is stated, only the next reset.
const THIRTY_DAYS: i64 = 30 * 86_400;

pub struct CodebuffService {
    http: Arc<HttpClient>,
}

impl CodebuffService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for CodebuffService {
    fn provider(&self) -> Provider {
        Provider::Codebuff
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Codebuff);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Codebuff)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // The pasted key first: it is the one somebody chose on purpose. It
        // reads the credits alone — no subscription call, as upstream.
        let body = serde_json::json!({"fingerprintId": "pulse-usage"});
        let usage = match self
            .http
            .fetch_json(Method::Post, USAGE_ENDPOINT, &refs, Some(&body))
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let (windows, plan, remaining) = match reading(&usage, None) {
            Ok(reading) => reading,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.origin = Some(self.origin_token().into());
        usage.plan = plan;
        if let Some(remaining) = remaining {
            usage.credit_balance = Some(balance_text(remaining));
        }
        usage
    }
}

/// The usage reply, plus the subscription reply when there is one. Returns
/// the windows, the plan's name, and the credits left.
pub fn reading(
    usage: &serde_json::Value,
    subscription: Option<&serde_json::Value>,
) -> Result<(Vec<UsageWindow>, Option<String>, Option<f64>), Unavailability> {
    if !usage.is_object() {
        return Err(Unavailability::UnreadableReply);
    }
    let mut windows: Vec<UsageWindow> = Vec::new();

    // Credits used out of the quota, both as Codebuff reports them. A
    // missing or zero quota is left off — never drawn as spent.
    let used = figure(usage, "usage").or_else(|| figure(usage, "used"));
    let quota = figure(usage, "quota").or_else(|| figure(usage, "limit"));
    if let (Some(used), Some(quota)) = (used, quota) {
        if quota > 0.0 {
            let mut window = UsageWindow::new(
                "codebuff.credits",
                // Upstream calls this a credit allowance; the Windows model
                // has no credits kind, so the month is a sort key and the
                // row says so.
                Kind::Other(THIRTY_DAYS),
                None,
                used / quota,
                THIRTY_DAYS,
                usage.get("next_quota_reset").and_then(date_field),
            );
            window.reports_length = false;
            window.is_exhausted = used >= quota;
            windows.push(window);
        }
    }

    // The weekly limit, when the subscription reply has one.
    let mut plan: Option<String> = None;
    if let Some(subscription) = subscription.filter(|reply| reply.is_object()) {
        let details = crate::http::object_field(subscription, "subscription");
        let limit = crate::http::object_field(subscription, "rateLimit");
        let weekly_used = limit
            .and_then(|limit| figure(limit, "weeklyUsed"))
            .or_else(|| limit.and_then(|limit| figure(limit, "used")));
        let weekly_limit = limit
            .and_then(|limit| figure(limit, "weeklyLimit"))
            .or_else(|| limit.and_then(|limit| figure(limit, "limit")));
        if let (Some(used), Some(total)) = (weekly_used, weekly_limit) {
            if total > 0.0 {
                let mut window = UsageWindow::new(
                    "codebuff.weekly",
                    Kind::Weekly,
                    None,
                    used / total,
                    7 * 86_400,
                    limit
                        .and_then(|limit| limit.get("weeklyResetsAt"))
                        .and_then(date_field),
                );
                window.is_exhausted = used >= total;
                windows.push(window);
            }
        }
        let candidates = [
            details.and_then(|details| crate::http::string_field(details, "displayName")),
            crate::http::string_field(subscription, "displayName"),
            details.and_then(|details| crate::http::string_field(details, "tier")),
            crate::http::string_field(subscription, "tier"),
        ];
        plan = candidates
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|name| !name.is_empty())
            .map(capitalize);
    }
    windows.sort_by_key(|window| window.window_seconds);

    // Credits, not money: the number alone, as Codex's balance is shown.
    let remaining = figure(usage, "remainingBalance").or_else(|| figure(usage, "remaining"));
    if windows.is_empty() && remaining.is_none() {
        return Err(Unavailability::NoLimitsReported);
    }
    Ok((windows, plan, remaining))
}

/// A figure: a finite, non-negative number, or one written as a string.
fn figure(value: &serde_json::Value, key: &str) -> Option<f64> {
    crate::http::number_field(value, key).filter(|figure| figure.is_finite() && *figure >= 0.0)
}

/// A figure of time: ISO 8601, or seconds (or milliseconds) since 1970.
fn date_field(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::String(text) => {
            let text = text.trim();
            if let Some(resets_at) = crate::timeutil::parse_iso8601_ms(text) {
                return Some(resets_at);
            }
            text.parse::<f64>()
                .ok()
                .filter(|figure| figure.is_finite() && *figure > 0.0)
                .map(crate::timeutil::epoch_to_ms)
        }
        other => crate::http::number(other)
            .filter(|figure| figure.is_finite() && *figure > 0.0)
            .map(crate::timeutil::epoch_to_ms),
    }
}

/// At most one decimal place, trailing zeros gone — "12", "12.5", "12.3".
fn balance_text(amount: f64) -> String {
    let mut text = format!("{amount:.1}");
    if text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

/// "pro" reads as "Pro"; a name already capitalised stands.
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
    fn credits_window_has_a_reset_but_no_length() {
        let usage = serde_json::json!({
            "usage": 120.5,
            "quota": 200,
            "next_quota_reset": "2026-10-05T00:00:00Z",
            "remainingBalance": 79.5,
        });
        let (windows, plan, remaining) = reading(&usage, None).unwrap();
        assert_eq!(windows.len(), 1);
        let window = &windows[0];
        assert_eq!(window.id, "codebuff.credits");
        assert_eq!(window.used_fraction, 120.5 / 200.0);
        assert!(!window.reports_length);
        assert!(window.resets_at.is_some());
        assert!(!window.is_exhausted);
        assert!(plan.is_none());
        assert_eq!(remaining, Some(79.5));
    }

    #[test]
    fn alternate_field_names_are_read() {
        let usage = serde_json::json!({"used": "200", "limit": "200", "remaining": 0});
        let (windows, _, remaining) = reading(&usage, None).unwrap();
        assert_eq!(windows[0].used_fraction, 1.0);
        assert!(windows[0].is_exhausted);
        assert_eq!(remaining, Some(0.0));
    }

    #[test]
    fn a_missing_or_zero_quota_is_left_off() {
        let usage = serde_json::json!({"usage": 10, "quota": 0, "remainingBalance": 55});
        let (windows, _, remaining) = reading(&usage, None).unwrap();
        assert!(windows.is_empty());
        assert_eq!(remaining, Some(55.0));
    }

    #[test]
    fn nothing_reported_is_an_answer() {
        let usage = serde_json::json!({"other": 1});
        assert_eq!(
            reading(&usage, None).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn subscription_adds_the_weekly_limit_and_the_plan() {
        let usage = serde_json::json!({"usage": 10, "quota": 100, "remainingBalance": 90});
        let subscription = serde_json::json!({
            "subscription": {"displayName": "Free", "tier": "free"},
            "rateLimit": {
                "weeklyUsed": 30,
                "weeklyLimit": 400,
                "weeklyResetsAt": 1_791_158_400,
            },
        });
        let (windows, plan, _) = reading(&usage, Some(&subscription)).unwrap();
        let weekly = windows.iter().find(|w| w.id == "codebuff.weekly").unwrap();
        assert_eq!(weekly.kind, Kind::Weekly);
        assert_eq!(weekly.used_fraction, 30.0 / 400.0);
        // The weekly window sorts first, before the monthly credits.
        assert_eq!(windows[0].id, "codebuff.weekly");
        assert_eq!(plan.as_deref(), Some("Free"));
    }

    #[test]
    fn plan_falls_through_display_name_to_tier() {
        let usage = serde_json::json!({"remainingBalance": 5});
        let subscription = serde_json::json!({"subscription": {"tier": "pro"}});
        let (_, plan, remaining) = reading(&usage, Some(&subscription)).unwrap();
        assert_eq!(plan.as_deref(), Some("Pro"));
        assert_eq!(remaining, Some(5.0));
    }

    #[test]
    fn dates_arrive_as_iso_or_epoch() {
        let iso = serde_json::json!("2026-10-05T00:00:00Z");
        let seconds = serde_json::json!("1791158400");
        let millis = serde_json::json!(1_791_158_400_000_u64);
        assert_eq!(date_field(&iso), date_field(&seconds));
        assert_eq!(date_field(&iso), date_field(&millis));
        assert_eq!(date_field(&serde_json::json!(0)), None);
    }

    #[test]
    fn a_non_object_reply_is_unreadable() {
        assert_eq!(
            reading(&serde_json::json!([1]), None).unwrap_err(),
            Unavailability::UnreadableReply
        );
    }

    #[test]
    fn balance_shows_at_most_one_decimal() {
        assert_eq!(balance_text(12.0), "12");
        assert_eq!(balance_text(12.5), "12.5");
        assert_eq!(balance_text(12.34), "12.3");
        assert_eq!(balance_text(0.04), "0");
    }
}
