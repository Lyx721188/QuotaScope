//! Abacus AI's ChatLLM / RouteLLM plan: one allowance of compute credits,
//! with the used amount and the total both reported, and the next billing
//! date as its reset.
//!
//! Read with the browser session the user imports in Settings, from the two
//! endpoints Abacus's own web app calls on `apps.abacus.ai`:
//! `GET /api/_getOrganizationComputePoints` for the credits, and
//! `POST /api/_getBillingInfo` for the next billing date and the plan's
//! name. The second is optional: without it the credits still show, with no
//! reset. The shapes are second-hand — taken from CodexBar's Abacus provider
//! and its tests, not from a captured reply — and so are the cookie names,
//! which neither source pins down.
//!
//! **No length is claimed.** Nothing in either reply says how long a cycle
//! is; the billing date says only when this one ends. The thirty days on the
//! window are a sort key.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::sync::Arc;

/// The session comes from `abacus.ai`. Abacus's session sits under any one
/// of several cookie names, which upstream spells as one pipe-separated
/// entry: any one of them is enough, and every one present is kept.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["abacus.ai"],
    // Upstream spells these as one pipe-separated entry; the import step
    // keeps a header when any wanted name is present, so the alternatives
    // are listed individually here.
    cookies: &[
        "sessionid",
        "session_id",
        "session_token",
        "auth_token",
        "access_token",
    ],
};

const POINTS_URL: &str = "https://apps.abacus.ai/api/_getOrganizationComputePoints";
const BILLING_URL: &str = "https://apps.abacus.ai/api/_getBillingInfo";

pub struct AbacusService {
    http: Arc<HttpClient>,
}

impl AbacusService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for AbacusService {
    fn provider(&self) -> Provider {
        Provider::Abacus
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Abacus);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::Abacus)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let headers = [
            ("Cookie", cookie),
            ("Accept", "application/json".to_string()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let points = match session_json(&self.http, Method::Get, POINTS_URL, &refs, None) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        // The billing date is extra: a failure there costs only the reset.
        let billing = session_json(
            &self.http,
            Method::Post,
            BILLING_URL,
            &refs,
            Some(&serde_json::json!({})),
        )
        .ok();

        match reading(&points, billing.as_ref()) {
            Ok((window, plan)) => {
                let mut usage = ProviderUsage::live_now(account, vec![window]);
                usage.origin = Some(self.origin_token().into());
                usage.plan = plan;
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// One call with the imported session as its credential. `fetch_json` folds
/// 401, 403 and the unfollowed redirect into `ApiKeyRefused`, which is the
/// pasted key's refusal — for a session those statuses are the session
/// expiring, so the folded reason is read here and remapped. A request that
/// sends no key can arrive at that folding no other way. 404 lands in
/// `NotFound`; upstream's classifier calls it a server error like any other.
fn session_json(
    http: &HttpClient,
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, Unavailability> {
    match http.fetch_json_detailed(method, url, headers, body) {
        Ok(value) => Ok(value),
        Err(HttpFailure::NotFound) => Err(Unavailability::ServerError),
        Err(HttpFailure::Unavailable(Unavailability::ApiKeyRefused)) => {
            Err(Unavailability::SessionExpired)
        }
        Err(HttpFailure::Unavailable(reason)) => Err(reason),
    }
}

/// Both endpoints answer `{ success, result, error }`, and a refused session
/// can come back as a 200 that says so in `error`.
#[derive(Deserialize)]
struct Envelope<P> {
    success: Option<bool>,
    result: Option<P>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct Points {
    #[serde(rename = "totalComputePoints")]
    total_compute_points: Option<f64>,
    #[serde(rename = "computePointsLeft")]
    compute_points_left: Option<f64>,
}

#[derive(Deserialize)]
struct Billing {
    #[serde(rename = "nextBillingDate")]
    next_billing_date: Option<String>,
    #[serde(rename = "currentTier")]
    current_tier: Option<String>,
}

/// The `result` of an envelope that says it succeeded, or the reason there
/// is none.
fn envelope<P: DeserializeOwned>(reply: &serde_json::Value) -> Result<P, Unavailability> {
    let envelope: Envelope<P> =
        serde_json::from_value(reply.clone()).map_err(|_| Unavailability::UnreadableReply)?;
    match (envelope.success, envelope.result) {
        (Some(true), Some(result)) => Ok(result),
        (Some(false), _) => Err(signed_out(&envelope.error)),
        // A success that carries no result, or no verdict at all, is not a
        // reply this reads.
        _ => Err(Unavailability::UnreadableReply),
    }
}

/// The error text of a refused envelope: a signed-out session says so in its
/// own words, and anything else is a failure of the service.
fn signed_out(error: &Option<String>) -> Unavailability {
    let text = error
        .as_deref()
        .map(|text| text.to_ascii_lowercase())
        .unwrap_or_default();
    let said = [
        "expired",
        "session",
        "login",
        "authenticate",
        "unauthorized",
        "unauthenticated",
        "forbidden",
    ];
    if said.iter().any(|word| text.contains(word)) {
        Unavailability::SessionExpired
    } else {
        Unavailability::ServerError
    }
}

/// The credits window and the plan's name, from the two replies. The billing
/// reply may be absent — a failure there costs only the reset and the name.
pub fn reading(
    points: &serde_json::Value,
    billing: Option<&serde_json::Value>,
) -> Result<(UsageWindow, Option<String>), Unavailability> {
    let points: Points = envelope(points)?;
    // Billing goes through the same envelope: a bare dict there is no
    // billing info, and is swallowed rather than failing the reading.
    let billing: Option<Billing> = billing.and_then(|value| envelope(value).ok());

    // Both halves are reported; neither is ever assumed.
    let Some(total) = points
        .total_compute_points
        .filter(|total| total.is_finite())
    else {
        return Err(Unavailability::NoLimitsReported);
    };
    let Some(left) = points.compute_points_left.filter(|left| left.is_finite()) else {
        return Err(Unavailability::NoLimitsReported);
    };
    if total <= 0.0 {
        return Err(Unavailability::NoLimitsReported);
    }

    let used = (total - left).max(0.0) / total;
    let resets_at = billing
        .as_ref()
        .and_then(|billing| billing.next_billing_date.as_deref())
        .and_then(crate::timeutil::parse_iso8601_ms);
    let mut window = UsageWindow::new(
        "abacus.credits",
        Kind::Credits,
        None,
        used,
        30 * 86_400,
        resets_at,
    );
    // The thirty days are a sort key; the cycle's length is never stated.
    window.reports_length = false;
    window.is_exhausted = left <= 0.0;

    let tier = billing
        .as_ref()
        .and_then(|billing| billing.current_tier.as_deref())
        .map(str::trim)
        .filter(|tier| !tier.is_empty())
        .map(str::to_string);
    Ok((window, tier))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(
        total: impl Into<serde_json::Value>,
        left: impl Into<serde_json::Value>,
    ) -> serde_json::Value {
        serde_json::json!({
            "success": true,
            "result": {"totalComputePoints": total.into(), "computePointsLeft": left.into()}
        })
    }

    fn billing(next: &str, tier: &str) -> serde_json::Value {
        serde_json::json!({
            "success": true,
            "result": {"nextBillingDate": next, "currentTier": tier}
        })
    }

    #[test]
    fn credits_against_total_with_the_billing_reset_and_tier() {
        let (window, plan) = reading(
            &points(1000.0, 250.0),
            Some(&billing("2026-11-01T00:00:00Z", " Pro ")),
        )
        .unwrap();
        assert_eq!(window.id, "abacus.credits");
        assert_eq!(window.used_fraction, 0.75);
        assert_eq!(window.window_seconds, 30 * 86_400);
        // No cycle length is stated; the month is a sort key only.
        assert!(!window.reports_length);
        assert!(!window.is_exhausted);
        assert_eq!(
            window.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-11-01T00:00:00Z")
        );
        assert_eq!(plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn a_spent_allowance_is_exhausted_and_a_negative_left_reads_over_one() {
        let (spent, _) = reading(&points(1000.0, 0.0), None).unwrap();
        assert!(spent.is_exhausted);
        assert_eq!(spent.used_fraction, 1.0);

        // The guard is only against a used figure below zero.
        let (over, _) = reading(&points(100.0, -10.0), None).unwrap();
        assert_eq!(over.used_fraction, 1.1);
    }

    #[test]
    fn without_billing_the_credits_still_show_with_no_reset_or_plan() {
        let (window, plan) = reading(&points(100.0, 10.0), None).unwrap();
        assert_eq!(window.resets_at, None);
        assert_eq!(plan, None);

        // A billing reply that is not an envelope is no billing info either.
        let (window, plan) = reading(
            &points(100.0, 10.0),
            Some(&serde_json::json!({"nextBillingDate": "x"})),
        )
        .unwrap();
        assert_eq!(window.resets_at, None);
        assert_eq!(plan, None);
    }

    #[test]
    fn missing_or_unsound_figures_are_no_limits_not_zero() {
        for reply in [
            serde_json::json!({"success": true, "result": {}}),
            serde_json::json!({"success": true, "result": {"totalComputePoints": 0, "computePointsLeft": 0}}),
            serde_json::json!({"success": true, "result": {"totalComputePoints": 100}}),
        ] {
            assert_eq!(
                reading(&reply, None).map(|_| ()),
                Err(Unavailability::NoLimitsReported)
            );
        }
        // A figure that arrived as a string fails the reply rather than
        // reading as zero.
        assert_eq!(
            reading(
                &serde_json::json!({"success": true, "result": {"totalComputePoints": "100", "computePointsLeft": 1}}),
                None,
            )
            .map(|_| ()),
            Err(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn a_refused_envelope_is_read_out_of_its_error_text() {
        // Both endpoints answer success==false in-band when the session is gone.
        let expired =
            serde_json::json!({"success": false, "error": "Session expired, please login again"});
        assert_eq!(reading(&expired, None), Err(Unavailability::SessionExpired));
        let other = serde_json::json!({"success": false, "error": "Billing unavailable"});
        assert_eq!(reading(&other, None), Err(Unavailability::ServerError));
        // No verdict, or a verdict with no result, is unreadable.
        for reply in [
            serde_json::json!({"success": true}),
            serde_json::json!({"totalComputePoints": 1}),
            serde_json::json!([]),
        ] {
            assert_eq!(
                reading(&reply, None).map(|_| ()),
                Err(Unavailability::UnreadableReply)
            );
        }
    }
}
