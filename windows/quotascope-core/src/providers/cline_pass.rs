//! ClinePass, Cline's subscription: a five-hour, a weekly and a monthly limit,
//! each reported as a percentage by the service itself.
//!
//! Read with a key the user pastes, from the endpoint Cline's own app calls:
//! `GET https://api.cline.bot/api/v1/users/me/plan/usage-limits`. The shape is
//! second-hand — taken from CodexBar's ClinePass provider and its tests, not
//! from a captured reply — and the test here says so.
//!
//! Only the three named limits are read. A limit type this build does not know
//! is left off rather than guessed at: its length is not stated anywhere, so
//! neither its name nor its window clock could be said honestly.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.cline.bot/api/v1/users/me/plan/usage-limits";

pub struct ClinePassService {
    http: Arc<HttpClient>,
}

impl ClinePassService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for ClinePassService {
    fn provider(&self) -> Provider {
        Provider::ClinePass
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::ClinePass);
        let Some(key) = pasted_or_none(keys.api_key(Provider::ClinePass)) else {
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
        let windows = match reading(&reply) {
            Ok(windows) => windows,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.origin = Some(self.origin_token().into());
        usage
    }
}

/// The limits Cline names, with the length each one is. `monthly` is a
/// billing month rather than a fixed thirty days, so its length is only a
/// sort key and is not claimed.
fn shape(kind: &str) -> Option<(Kind, i64, bool)> {
    match kind {
        "five_hour" => Some((Kind::FiveHour, 5 * 3_600, true)),
        "weekly" => Some((Kind::Weekly, 7 * 86_400, true)),
        "monthly" => Some((Kind::Monthly, 30 * 86_400, false)),
        _ => None,
    }
}

pub fn reading(reply: &serde_json::Value) -> Result<Vec<UsageWindow>, Unavailability> {
    if crate::http::bool_field(reply, "success") != Some(true) {
        return Err(Unavailability::UnreadableReply);
    }
    let Some(limits) = reply.pointer("/data/limits").and_then(|v| v.as_array()) else {
        return Err(Unavailability::UnreadableReply);
    };
    let mut windows: Vec<UsageWindow> = limits
        .iter()
        .filter_map(|limit| {
            let kind = crate::http::string_field(limit, "type")?;
            let (kind, seconds, reports_length) = shape(kind)?;
            let percent = crate::http::number_field(limit, "percentUsed")?;
            if !percent.is_finite() || percent < 0.0 {
                return None;
            }
            let id = format!("clinepass.{}", crate::http::string_field(limit, "type")?);
            let mut window = UsageWindow::new(
                &id,
                kind,
                None,
                percent / 100.0,
                seconds,
                crate::http::string_field(limit, "resetsAt")
                    .and_then(crate::timeutil::parse_iso8601_ms),
            );
            window.reports_length = reports_length;
            window.is_exhausted = percent >= 100.0;
            Some(window)
        })
        .collect();
    windows.sort_by_key(|window| window.window_seconds);
    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(limits: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"success": true, "data": {"limits": limits}})
    }

    #[test]
    fn three_limits_sort_by_length() {
        let windows = reading(&reply(serde_json::json!([
            {"type": "monthly", "percentUsed": 10.0, "resetsAt": "2026-10-31T00:00:00Z"},
            {"type": "five_hour", "percentUsed": 42.5, "resetsAt": "2026-10-01T18:00:00Z"},
            {"type": "weekly", "percentUsed": 0, "resetsAt": "2026-10-06T00:00:00Z"},
        ])))
        .unwrap();
        let ids: Vec<&str> = windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "clinepass.five_hour",
                "clinepass.weekly",
                "clinepass.monthly"
            ]
        );

        let five_hour = &windows[0];
        assert_eq!(five_hour.kind, Kind::FiveHour);
        assert_eq!(five_hour.used_fraction, 0.425);
        // Five hours is a stated length; the reset says when it turns over.
        assert!(five_hour.reports_length);
        assert!(five_hour.resets_at.is_some());

        let weekly = &windows[1];
        assert_eq!(weekly.kind, Kind::Weekly);
        assert_eq!(weekly.used_fraction, 0.0);
        assert!(weekly.reports_length);

        let monthly = &windows[2];
        assert_eq!(monthly.kind, Kind::Monthly);
        // A billing month is not a fixed thirty days, so no length is claimed.
        assert!(!monthly.reports_length);
        assert!(monthly.resets_at.is_some());
    }

    #[test]
    fn a_limit_cline_says_is_spent_is_marked_exhausted() {
        let windows = reading(&reply(serde_json::json!([
            {"type": "five_hour", "percentUsed": 100, "resetsAt": "2026-10-01T18:00:00Z"},
        ])))
        .unwrap();
        assert!(windows[0].is_exhausted);
        assert_eq!(windows[0].used_fraction, 1.0);
    }

    #[test]
    fn unknown_or_broken_limits_are_left_off_not_guessed() {
        let windows = reading(&reply(serde_json::json!([
            {"type": "daily", "percentUsed": 50.0},
            {"type": "weekly", "percentUsed": null},
            {"type": "weekly", "percentUsed": -3.0},
            {"percentUsed": 10.0},
            {"type": "weekly", "percentUsed": 25.0},
        ])))
        .unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "clinepass.weekly");
        assert_eq!(windows[0].used_fraction, 0.25);
    }

    #[test]
    fn no_limits_left_is_an_answer() {
        assert_eq!(
            reading(&reply(serde_json::json!([]))).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn a_reply_that_is_not_a_success_is_unreadable() {
        for reply in [
            serde_json::json!({"success": false, "data": {"limits": []}}),
            serde_json::json!({"success": true}),
            serde_json::json!({"success": true, "data": {}}),
            serde_json::json!({"success": true, "data": {"limits": "none"}}),
            serde_json::json!({}),
        ] {
            assert_eq!(
                reading(&reply).unwrap_err(),
                Unavailability::UnreadableReply,
                "reply: {reply}"
            );
        }
    }
}
