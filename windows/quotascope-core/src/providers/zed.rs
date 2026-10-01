//! Zed, the editor: its edit-prediction allowance and its token spend against
//! the spending limit the account set.
//!
//! **Read with a browser session, not the editor's login.** zed.dev's billing
//! page has a signed-in session, and that is what is read — kept to the
//! `zed.session` cookie. Signing in only inside the editor does not create
//! one; signing in at zed.dev in a browser does.
//!
//! `GET https://cloud.zed.dev/frontend/billing/usage` with the cookie. The
//! page's own frontend call, undocumented. The shape is second-hand — taken
//! from CodexBar's Zed plugin and its tests, not from a captured reply — and
//! the fixture in the tests says so.
//!
//! A limit the account doesn't have is left off: unlimited predictions, or no
//! spending limit, draw nothing rather than a ring at zero.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button: zed.dev's own
/// sign-in cookie.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["zed.dev"],
    cookies: &["zed.session"],
};

const ENDPOINT: &str = "https://cloud.zed.dev/frontend/billing/usage";

pub struct ZedService {
    http: Arc<HttpClient>,
}

impl ZedService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for ZedService {
    fn provider(&self) -> Provider {
        Provider::Zed
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Zed);
        // The imported header is stored where pasted keys live, but a missing
        // one is a session that was never imported, not a key never pasted.
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::Zed)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let headers = [
            ("Cookie", cookie),
            ("Accept", "application/json".to_string()),
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
            // `fetch_json` folds 401/403 and every redirect into the refused
            // key; for a session those all mean the same thing.
            Err(failure) => {
                return ProviderUsage::unavailable(account, session_failure(failure));
            }
        };
        let usage = reading(&reply, account);
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

/// A reported figure: a finite number, not negative, and never a boolean.
/// Anything else is left off rather than read as zero.
fn count(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .filter(|figure| figure.is_finite() && *figure >= 0.0)
}

/// A number, or `{ "limited": n }`. `"unlimited"` and null are no limit.
fn prediction_limit(value: &Value) -> Option<f64> {
    match value {
        Value::Object(map) => map.get("limited").and_then(count),
        other => count(other),
    }
}

pub fn reading(reply: &Value, account: AccountKey) -> ProviderUsage {
    let Some(usage) = reply.get("current_usage").filter(|v| v.is_object()) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    let mut windows: Vec<UsageWindow> = Vec::new();

    // So many predictions in the account's allowance, and so many used. The
    // reply names no period; Zed's plans count them by the month, so the
    // length is a sort key and not a claim.
    if let Some(predictions) = usage.get("edit_predictions").filter(|v| v.is_object()) {
        if let Some(used) = predictions.get("used").and_then(count) {
            if let Some(limit) = predictions.get("limit").and_then(prediction_limit) {
                if limit > 0.0 {
                    let mut window = UsageWindow::new(
                        "zed.editPredictions",
                        Kind::Monthly,
                        Some("Edit Predictions".into()),
                        used / limit,
                        30 * 86_400,
                        None,
                    );
                    // No period is stated, and nothing says when it turns over.
                    window.reports_length = false;
                    window.is_exhausted = used >= limit;
                    windows.push(window);
                }
            }
        }
    }

    // Token spend against the spending limit, both in cents. No limit set is
    // spend with nothing to measure it against, and is left off.
    if let Some(spend) = usage.get("token_spend").filter(|v| v.is_object()) {
        if let Some(spent) = spend.get("spend_in_cents").and_then(count) {
            if let Some(limit) = spend.get("limit_in_cents").and_then(count) {
                if limit > 0.0 {
                    let mut window = UsageWindow::new(
                        "zed.tokenSpend",
                        Kind::Spend,
                        None,
                        spent / limit,
                        30 * 86_400,
                        None,
                    );
                    window.reports_length = false;
                    window.is_exhausted = spent >= limit;
                    windows.push(window);
                }
            }
        }
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let mut read = ProviderUsage::live_now(account, windows);
    read.plan = reply
        .get("plan")
        .and_then(|v| v.as_str())
        .and_then(plan_name);
    read
}

/// `zed_pro_trial` → "Zed Pro Trial". A product's own name, so it is not
/// translated.
pub fn plan_name(raw: &str) -> Option<String> {
    let words: Vec<String> = raw
        .split(['_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn reading_for(value: Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::Zed))
    }

    /// Upstream's `zed-billing-usage` fixture — second-hand, written from
    /// CodexBar's Zed plugin, not captured from a live account.
    #[test]
    fn edit_predictions_and_token_spend_are_read_as_reported() {
        let usage = reading_for(json!({
            "plan": "zed_pro",
            "current_usage": {
                "token_spend": {"spend_in_cents": 250, "limit_in_cents": 1000},
                "edit_predictions": {"used": 12, "limit": 100}
            }
        }));
        assert!(matches!(usage.state, State::Live));
        let predictions = &usage.windows[0];
        assert_eq!(predictions.kind, Kind::Monthly);
        assert_eq!(predictions.scope.as_deref(), Some("Edit Predictions"));
        assert_eq!(predictions.used_fraction, 0.12);
        let spend = &usage.windows[1];
        assert_eq!(spend.kind, Kind::Spend);
        assert_eq!(spend.used_fraction, 0.25);
        // No period is stated, and nothing says when either turns over.
        assert!(usage
            .windows
            .iter()
            .all(|w| !w.reports_length && w.resets_at.is_none()));
        assert_eq!(usage.plan.as_deref(), Some("Zed Pro"));
    }

    #[test]
    fn a_limit_written_as_an_object_reads_the_same() {
        let usage = reading_for(json!({
            "plan": "zed_pro_trial",
            "current_usage": {"edit_predictions": {"used": 10, "limit": {"limited": 20}}}
        }));
        assert_eq!(usage.windows[0].used_fraction, 0.5);
        assert_eq!(usage.plan.as_deref(), Some("Zed Pro Trial"));
    }

    #[test]
    fn overspend_is_kept_and_marked_spent() {
        let usage = reading_for(json!({
            "plan": "zed_pro",
            "current_usage": {"token_spend": {"spend_in_cents": 1500, "limit_in_cents": 1000}}
        }));
        let spend = &usage.windows[0];
        assert_eq!(spend.used_fraction, 1.5);
        assert!(spend.is_exhausted);
    }

    #[test]
    fn unlimited_predictions_and_no_spending_limit_draw_nothing_not_a_zero() {
        for current in [
            json!({"token_spend": {"spend_in_cents": 250, "limit_in_cents": Value::Null},
                   "edit_predictions": {"used": 12, "limit": "unlimited"}}),
            json!({"token_spend": {"spend_in_cents": 250},
                   "edit_predictions": {"used": 12, "limit": Value::Null}}),
        ] {
            let usage = reading_for(json!({"plan": "zed_pro", "current_usage": current}));
            assert_eq!(
                usage.state,
                State::Unavailable(Unavailability::NoLimitsReported)
            );
        }
    }

    #[test]
    fn a_figure_that_isnt_one_is_left_off() {
        for current in [
            // Negative, or a boolean where a figure belongs.
            json!({"token_spend": {"spend_in_cents": -1, "limit_in_cents": 1000},
                   "edit_predictions": {"used": true, "limit": 100}}),
            json!({"token_spend": {"spend_in_cents": 250, "limit_in_cents": -1},
                   "edit_predictions": {"used": 12, "limit": 0}}),
        ] {
            let usage = reading_for(json!({"current_usage": current}));
            assert_eq!(
                usage.state,
                State::Unavailable(Unavailability::NoLimitsReported)
            );
        }
    }

    #[test]
    fn a_reply_that_cant_be_read() {
        for value in [json!({}), json!([]), json!({"current_usage": []})] {
            assert_eq!(
                reading_for(value).state,
                State::Unavailable(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn plan_names_are_capitalized_product_words() {
        assert_eq!(plan_name("zed_pro_trial").as_deref(), Some("Zed Pro Trial"));
        assert_eq!(plan_name("zed_pro").as_deref(), Some("Zed Pro"));
        assert_eq!(plan_name(""), None);
    }
}
