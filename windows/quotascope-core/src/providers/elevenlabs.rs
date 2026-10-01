//! ElevenLabs: the subscription's character credits for the current billing
//! period — how many have been used, out of how many, and when they reset.
//!
//! Read with a key the user pastes, from ElevenLabs' own subscription route:
//! `GET https://api.elevenlabs.io/v1/user/subscription`, key in `xi-api-key`.
//! The key needs the `user_read` permission. The shape is second-hand —
//! taken from CodexBar's ElevenLabs plugin and its docs, not from a captured
//! reply.
//!
//! **Credits, not a month.** The period follows the subscription's billing
//! date and the reply states only when it ends, so the allowance claims no
//! length. Voice slots are left off: they are a count of voices kept, not an
//! allowance spent over time, and no kind of window here reads as one.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number_field, string_field, HttpClient};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.elevenlabs.io/v1/user/subscription";

pub struct ElevenLabsService {
    http: Arc<HttpClient>,
}

impl ElevenLabsService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for ElevenLabsService {
    fn provider(&self) -> Provider {
        Provider::ElevenLabs
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::ElevenLabs);
        let Some(key) = pasted_or_none(keys.api_key(Provider::ElevenLabs)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        // ElevenLabs takes its key in a header of its own rather than as a
        // bearer.
        let headers = [("xi-api-key", key), ("Accept", "application/json".into())];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self
            .http
            .fetch_json(crate::http::Method::Get, ENDPOINT, &refs, None)
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        match reading(&reply) {
            Ok(reading) => {
                let mut usage = ProviderUsage::live_now(account, reading.windows);
                usage.plan = reading.plan;
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// What one reply yields, kept apart from the HTTP plumbing so tests can feed
/// it fixtures directly.
#[derive(Debug)]
pub struct ElevenLabsReading {
    pub windows: Vec<UsageWindow>,
    pub plan: Option<String>,
}

pub fn reading(reply: &serde_json::Value) -> Result<ElevenLabsReading, Unavailability> {
    let mut windows = Vec::new();
    // A limit of zero is no allowance at all, and is left off rather than
    // drawn as an empty ring.
    if let (Some(used), Some(limit)) = (
        number_field(reply, "character_count").filter(|value| value.is_finite() && *value >= 0.0),
        number_field(reply, "character_limit").filter(|value| value.is_finite() && *value > 0.0),
    ) {
        let mut window = UsageWindow::new(
            "elevenlabs.characters",
            Kind::Credits,
            None,
            used / limit,
            // A sort key only: the billing period's length is not stated.
            30 * 86_400,
            number_field(reply, "next_character_count_reset_unix")
                .filter(|seconds| *seconds > 0.0)
                .map(crate::timeutil::epoch_to_ms),
        );
        window.reports_length = false;
        window.is_exhausted = used >= limit;
        windows.push(window);
    }

    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    let plan = plan(string_field(reply, "tier"));
    Ok(ElevenLabsReading { windows, plan })
}

/// "creator" → "Creator", "growing_business" → "Growing Business". The tier
/// is ElevenLabs' own plan name, so it is left untranslated.
pub fn plan(tier: Option<&str>) -> Option<String> {
    let tier = tier?.trim();
    if tier.is_empty() {
        return None;
    }
    Some(
        tier.replace('_', " ")
            .split_whitespace()
            .map(capitalize)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.push_str(&chars.as_str().to_lowercase());
            out
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn characters_window_from_a_documented_shape() {
        let reply = json!({
            "tier": "growing_business",
            "character_count": 3_250.0,
            "character_limit": 10_000.0,
            "next_character_count_reset_unix": 1_791_168_000.0,
        });
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Growing Business"));
        let window = &reading.windows[0];
        assert_eq!(window.id, "elevenlabs.characters");
        assert_eq!(window.used_fraction, 0.325);
        assert!(!window.reports_length);
        assert!(!window.is_exhausted);
        assert_eq!(
            window.resets_at,
            Some(crate::timeutil::epoch_to_ms(1_791_168_000.0))
        );
    }

    #[test]
    fn a_zero_or_absent_limit_is_no_allowance() {
        let empty = json!({ "tier": "free", "character_count": 0.0, "character_limit": 0.0 });
        assert_eq!(
            reading(&empty).unwrap_err(),
            Unavailability::NoLimitsReported
        );
        let bare = json!({ "tier": "free" });
        assert_eq!(
            reading(&bare).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn used_up_credits_are_the_provider_s_own_arithmetic() {
        let reply = json!({ "character_count": 10_000.0, "character_limit": 10_000.0 });
        let result = reading(&reply).unwrap();
        assert_eq!(result.windows[0].used_fraction, 1.0);
        assert!(result.windows[0].is_exhausted);
        // Past the end stands, unclamped.
        let over = json!({ "character_count": 10_500.0, "character_limit": 10_000.0 });
        assert_eq!(reading(&over).unwrap().windows[0].used_fraction, 1.05);
    }

    #[test]
    fn a_reset_at_or_before_the_epoch_is_no_reset() {
        let reply = json!({
            "character_count": 1.0,
            "character_limit": 2.0,
            "next_character_count_reset_unix": 0.0,
        });
        assert!(reading(&reply).unwrap().windows[0].resets_at.is_none());
    }

    #[test]
    fn tier_names_are_capitalized_word_by_word() {
        assert_eq!(plan(Some("creator")), Some("Creator".into()));
        assert_eq!(
            plan(Some("growing_business")),
            Some("Growing Business".into())
        );
        assert_eq!(plan(Some("  free  ")), Some("Free".into()));
        assert_eq!(plan(Some("")), None);
        assert_eq!(plan(None), None);
    }
}
