//! GitKraken AI: the account's AI credits used out of its allowance, and the
//! organization's shared pool beside it where there is one.
//!
//! Read with an access token the user copies by hand from gitkraken.dev —
//! GitKraken issues no API key for this — from the route its own usage page
//! and GitLens call: `GET https://api.gitkraken.dev/v1/ai-tasks/usage`. The
//! shape is second-hand — taken from CodexBar's GitKraken plugin, which
//! follows GitLens's parser, not from a captured reply — and the tests here
//! say so.
//!
//! **Credits, not a week.** CodexBar calls the allowance weekly; the reply
//! states only `resetsOn`, so no length is claimed and seven days is only
//! where the row sorts. Personal and organization credits remain distinct.
//! A limit of `-1` (unlimited) or `0` (no
//! allowance) is a statement with no fraction in it, and draws nothing.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number_field, object_field, string_field, HttpClient, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.gitkraken.dev/v1/ai-tasks/usage";

/// A sort key only: the reply does not state the period.
const SORT_KEY: i64 = 7 * 86_400;

pub struct GitKrakenService {
    http: Arc<HttpClient>,
}

impl GitKrakenService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for GitKrakenService {
    fn provider(&self) -> Provider {
        Provider::GitKraken
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::GitKraken);
        // People copy the whole header value, "Bearer" and all.
        let Some(token) = pasted_or_none(keys.api_key(Provider::GitKraken))
            .and_then(|pasted| token_from(&pasted))
        else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {token}")),
            ("Accept", "application/json".into()),
            // GitKraken's API asks every caller to name itself.
            ("Client-Name", "Pulse".into()),
            ("Client-Version", env!("CARGO_PKG_VERSION").into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self.http.fetch_json(Method::Get, ENDPOINT, &refs, None) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = reading(&reply, account);
        if matches!(usage.state, crate::model::State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

/// The token alone, without a leading "Bearer", or nothing when anything else
/// surrounds it — a paste that caught two words is not a token.
pub fn token_from(pasted: &str) -> Option<String> {
    let mut words: Vec<&str> = pasted.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    if words[0].eq_ignore_ascii_case("bearer") {
        words.remove(0);
    }
    match words[..] {
        [token] => Some((*token).to_string()),
        _ => None,
    }
}

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    let Some(payload) = object_field(reply, "data") else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    // The `used` figure has to be there for the reply to mean anything at
    // all; the `limit` may not be — an unlimited account states none.
    let Some(used) = number_field(payload, "used") else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let resets_at = string_field(payload, "resetsOn").and_then(crate::timeutil::parse_iso8601_ms);

    let mut windows = Vec::new();
    if let Some(fraction) = fraction(Some(used), number_field(payload, "limit")) {
        windows.push(window("gitkraken.personal", fraction, resets_at));
    }
    if let Some(organization) = object_field(payload, "organization") {
        if let Some(fraction) = fraction(
            number_field(organization, "used"),
            number_field(organization, "limit"),
        ) {
            windows.push(window("gitkraken.organization", fraction, resets_at));
        }
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    ProviderUsage::live_now(account, windows)
}

/// Used out of a positive limit. `-1` is "unlimited" and `0` is "no
/// allowance": neither is a denominator.
fn fraction(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let used = used.filter(|used| used.is_finite() && *used >= 0.0)?;
    let limit = limit.filter(|limit| limit.is_finite() && *limit > 0.0)?;
    Some(used / limit)
}

fn window(id: &str, used: f64, resets_at: Option<i64>) -> UsageWindow {
    let kind = if id == "gitkraken.organization" {
        Kind::SharedCredits
    } else {
        Kind::Credits
    };
    let mut window = UsageWindow::new(id, kind, None, used, SORT_KEY, resets_at);
    // The reply does not state the period; seven days only sorts.
    window.reports_length = false;
    window.is_exhausted = used >= 1.0;
    window
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::GitKraken))
    }

    #[test]
    fn personal_and_organization_credits_read() {
        let usage = reading_for(serde_json::json!({
            "data": {
                "used": 120.0,
                "limit": 500.0,
                "resetsOn": "2026-10-06T00:00:00Z",
                "organization": {"used": 10.0, "limit": 1000.0},
            },
        }));
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.windows.len(), 2);

        let personal = &usage.windows[0];
        assert_eq!(personal.id, "gitkraken.personal");
        assert_eq!(personal.used_fraction, 0.24);
        assert_eq!(personal.window_seconds, SORT_KEY);
        assert!(!personal.reports_length, "the reply states no period");
        assert_eq!(
            personal.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-06T00:00:00Z")
        );

        let organization = &usage.windows[1];
        assert_eq!(organization.id, "gitkraken.organization");
        assert_eq!(organization.used_fraction, 0.01);
    }

    #[test]
    fn unlimited_and_empty_allowances_draw_nothing() {
        let usage = reading_for(serde_json::json!({
            "data": {"used": 5.0, "limit": -1.0, "organization": {"used": 3.0, "limit": 0.0}},
        }));
        assert_eq!(
            usage,
            ProviderUsage::unavailable(
                AccountKey::primary(Provider::GitKraken),
                Unavailability::NoLimitsReported
            )
        );
    }

    #[test]
    fn a_spent_allowance_is_marked_exhausted() {
        let usage = reading_for(serde_json::json!({"data": {"used": 500.0, "limit": 500.0}}));
        assert!(usage.windows[0].is_exhausted);
    }

    #[test]
    fn a_reply_without_a_used_figure_is_unreadable() {
        for reply in [
            serde_json::json!({}),
            serde_json::json!({"data": {}}),
            serde_json::json!({"data": {"used": null, "limit": 100.0}}),
        ] {
            assert!(matches!(
                reading_for(reply).state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn the_pasted_token_may_carry_its_bearer_word() {
        assert_eq!(token_from("abc123"), Some("abc123".into()));
        assert_eq!(token_from("  Bearer abc123 "), Some("abc123".into()));
        assert_eq!(token_from("bearer abc123"), Some("abc123".into()));
        assert_eq!(token_from("Bearer"), None);
        assert_eq!(token_from("abc 123"), None);
        assert_eq!(token_from(""), None);
    }
}
