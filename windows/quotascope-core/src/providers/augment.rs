//! Augment Code's credits for the billing cycle: how many were used against
//! how many the cycle makes available, both as Augment reports them.
//!
//! Read with the browser session the user imports in Settings, from the two
//! endpoints Augment's own account page calls on `app.augmentcode.com`:
//! `GET /api/credits` for the figures and `GET /api/subscription` for the
//! plan's name and when the cycle ends. The second is a nicety — a reading
//! stands without it, as it does in CodexBar.
//!
//! The shape is second-hand — taken from CodexBar's Augment provider and its
//! tests, not from a captured reply.
//!
//! **What CodexBar does and this does not.** When `usageUnitsAvailable` is
//! missing or zero it adds the remaining credits to the consumed ones and
//! calls that the limit; a limit Augment did not state is not one QuotaScope
//! draws against, so that reading is left off.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use std::sync::Arc;

/// The session comes from the account page's own host.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["app.augmentcode.com"],
    cookies: &["_session", "web_rpc_proxy_session"],
};

const BASE: &str = "https://app.augmentcode.com";

pub struct AugmentService {
    http: Arc<HttpClient>,
}

impl AugmentService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for AugmentService {
    fn provider(&self) -> Provider {
        Provider::Augment
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Augment);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::Augment)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let headers = [
            ("Cookie", cookie),
            ("Accept", "application/json".to_string()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let credits = match session_json(
            &self.http,
            Method::Get,
            &format!("{BASE}/api/credits"),
            &refs,
            None,
        ) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        // Optional, as in CodexBar: without it there is no plan name and no
        // reset, and the figures are still the figures.
        let subscription = session_json(
            &self.http,
            Method::Get,
            &format!("{BASE}/api/subscription"),
            &refs,
            None,
        )
        .ok();

        match reading(&credits, subscription.as_ref()) {
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

/// The figures `api/credits` reports. Strictly decoded: a figure that
/// arrived as a string fails the reply rather than reading as zero.
#[derive(Deserialize)]
struct Credits {
    #[serde(rename = "usageUnitsConsumedThisBillingCycle")]
    consumed: Option<f64>,
    #[serde(rename = "usageUnitsAvailable")]
    available: Option<f64>,
}

#[derive(Deserialize)]
struct Subscription {
    #[serde(rename = "planName")]
    plan_name: Option<String>,
    #[serde(rename = "billingPeriodEnd")]
    billing_period_end: Option<String>,
}

/// The credits window and the plan's name. A subscription reply that cannot
/// be read costs only the reset and the name.
pub fn reading(
    credits: &serde_json::Value,
    subscription: Option<&serde_json::Value>,
) -> Result<(UsageWindow, Option<String>), Unavailability> {
    // Not a dictionary of figures — an HTML sign-in page served with a 200,
    // say — is not a reply this can read.
    let credits: Credits =
        serde_json::from_value(credits.clone()).map_err(|_| Unavailability::UnreadableReply)?;
    let about: Option<Subscription> =
        subscription.and_then(|value| serde_json::from_value(value.clone()).ok());

    let plan = about
        .as_ref()
        .and_then(|about| about.plan_name.as_deref())
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .map(str::to_string);

    let Some(used) = credits
        .consumed
        .filter(|used| used.is_finite() && *used >= 0.0)
    else {
        return Err(Unavailability::NoLimitsReported);
    };
    let Some(available) = credits
        .available
        .filter(|available| available.is_finite() && *available > 0.0)
    else {
        // CodexBar's derived limit — remaining plus consumed — is left off
        // on purpose: Augment did not state it.
        return Err(Unavailability::NoLimitsReported);
    };

    // Credits, with the reset Augment states and no length it claims: a
    // billing cycle's thirty days are a sort key only.
    let mut window = UsageWindow::new(
        "augment.credits",
        // Upstream kinds this `.credits` — an allowance with a reset it
        // states and no length it claims. The Windows kind set has no
        // credits case; Spend is the one kind that never claims a length
        // either.
        Kind::Spend,
        None,
        used / available,
        30 * 86_400,
        about
            .as_ref()
            .and_then(|about| about.billing_period_end.as_deref())
            .and_then(crate::timeutil::parse_iso8601_ms),
    );
    window.reports_length = false;
    window.is_exhausted = used >= available;
    Ok((window, plan))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credits(
        used: impl Into<serde_json::Value>,
        available: impl Into<serde_json::Value>,
    ) -> serde_json::Value {
        serde_json::json!({
            "usageUnitsRemaining": 0,
            "usageUnitsConsumedThisBillingCycle": used.into(),
            "usageUnitsAvailable": available.into()
        })
    }

    #[test]
    fn used_over_the_stated_available_with_reset_and_plan() {
        let subscription = serde_json::json!({
            "planName": " Advanced ",
            "billingPeriodEnd": "2026-10-15T00:00:00Z"
        });
        let (window, plan) = reading(&credits(300.0, 1000.0), Some(&subscription)).unwrap();
        assert_eq!(window.id, "augment.credits");
        assert_eq!(window.used_fraction, 0.3);
        assert_eq!(window.window_seconds, 30 * 86_400);
        assert!(!window.reports_length);
        assert!(!window.is_exhausted);
        assert_eq!(
            window.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-15T00:00:00Z")
        );
        assert_eq!(plan.as_deref(), Some("Advanced"));
    }

    #[test]
    fn without_the_subscription_the_figures_still_stand() {
        let (window, plan) = reading(&credits(300.0, 1000.0), None).unwrap();
        assert_eq!(window.resets_at, None);
        assert_eq!(plan, None);

        // A subscription reply of the wrong shape is swallowed, not fatal.
        let (window, plan) =
            reading(&credits(300.0, 1000.0), Some(&serde_json::json!("nope"))).unwrap();
        assert_eq!(window.resets_at, None);
        assert_eq!(plan, None);
    }

    #[test]
    fn a_cycle_used_to_its_last_unit_is_exhausted() {
        let (window, _) = reading(&credits(1000.0, 1000.0), None).unwrap();
        assert!(window.is_exhausted);
    }

    #[test]
    fn no_stated_available_is_no_limit_not_a_derived_one() {
        for reply in [
            credits(300.0, 0.0),
            credits(300.0, serde_json::json!(null)),
            serde_json::json!({"usageUnitsRemaining": 700, "usageUnitsConsumedThisBillingCycle": 300}),
        ] {
            assert_eq!(
                reading(&reply, None).map(|_| ()),
                Err(Unavailability::NoLimitsReported)
            );
        }
    }

    #[test]
    fn an_html_sign_in_page_or_a_string_figure_is_unreadable() {
        // A 200 that is not a dictionary of figures.
        assert_eq!(
            reading(&serde_json::json!("<html>Sign in</html>"), None).map(|_| ()),
            Err(Unavailability::UnreadableReply)
        );
        // A figure that arrived as a string fails the reply rather than
        // reading as zero.
        assert_eq!(
            reading(&credits("300", 1000.0), None).map(|_| ()),
            Err(Unavailability::UnreadableReply)
        );
    }
}
