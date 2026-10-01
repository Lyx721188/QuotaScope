//! OpenAI's API platform: the prepaid credit left on the account, and nothing
//! else.
//!
//! Read with a pasted key, from the billing route OpenAI's own dashboard used
//! for the balance: `GET
//! https://api.openai.com/v1/dashboard/billing/credit_grants`. It is not in
//! OpenAI's current public reference and it answers older user keys only;
//! the shape is second-hand — taken from CodexBar's OpenAI plugin, not from a
//! captured reply — and the fixture in the tests says so.
//!
//! **No ring.** The API has no allowance to be a fraction of: what OpenAI
//! reports is money granted, spent and left, so the balance is shown as a
//! balance. The organization's spend over the last days — what an Admin key
//! reads, and what CodexBar draws — is a total with no limit behind it, and
//! there is nowhere to show one.
//!
//! **A key the balance route turns away is not yet a bad key.** Admin and
//! project keys are refused there by design, so the refusal is checked
//! against the Admin API's cost route before anything is said: a key that
//! reads costs works, and the honest answer is that it reports no limits.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, State, Unavailability};
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

const CREDIT_GRANTS: &str = "https://api.openai.com/v1/dashboard/billing/credit_grants";
const COSTS_PROBE: &str = "https://api.openai.com/v1/organization/costs";

pub struct OpenAiPlatformService {
    http: Arc<HttpClient>,
}

impl OpenAiPlatformService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for OpenAiPlatformService {
    fn provider(&self) -> Provider {
        Provider::OpenAiPlatform
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::OpenAiPlatform);
        let Some(key) = pasted_or_none(keys.api_key(Provider::OpenAiPlatform)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        // The status has to be read directly, not through the shared
        // classifier: which refusals probe and which end the fetch is this
        // provider's own branch, and the difference is the point.
        let (status, body) = match self.send(CREDIT_GRANTS, &key) {
            Ok(reply) => reply,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        match grants_outcome(status) {
            Outcome::Read => {
                let mut usage = reading(&body.unwrap_or(serde_json::Value::Null), account);
                if matches!(usage.state, State::Live) {
                    usage.origin = Some(self.origin_token().into());
                }
                usage
            }
            Outcome::Probe => {
                // Refused, or a route this key's kind cannot see: ask the
                // Admin API whether the key itself is any good. The probe's
                // figures are never read — only its status.
                let now_secs = crate::timeutil::now_ms() / 1000;
                let (probe_status, _) = match self.send(&costs_probe_url(now_secs), &key) {
                    Ok(reply) => reply,
                    Err(reason) => return ProviderUsage::unavailable(account, reason),
                };
                let reason = if (200..300).contains(&probe_status) {
                    Unavailability::NoLimitsReported
                } else {
                    probe_reason(probe_status)
                };
                ProviderUsage::unavailable(account, reason)
            }
            Outcome::RateLimited => {
                ProviderUsage::unavailable(account, Unavailability::RateLimited)
            }
            Outcome::ServerError => {
                ProviderUsage::unavailable(account, Unavailability::ServerError)
            }
        }
    }
}

impl OpenAiPlatformService {
    /// One GET with the key, answered with the status and — only for a 2xx —
    /// the body when it decodes. A non-2xx body is never read, so a refusal
    /// keeps its meaning whatever it says, and a 2xx body that will not
    /// decode stays a 2xx: the reading decides what that means, and the
    /// probe never reads one.
    fn send(
        &self,
        url: &str,
        key: &str,
    ) -> Result<(u16, Option<serde_json::Value>), Unavailability> {
        let response = self
            .http
            .client_for_login()
            .get(url)
            .header("Authorization", format!("Bearer {key}"))
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(15))
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        let status = crate::http::status_of(&response);
        if !(200..300).contains(&status) {
            return Ok((status, None));
        }
        let text = response
            .text()
            .map_err(|_| Unavailability::UnreadableReply)?;
        let body = serde_json::from_str(&text).ok();
        Ok((status, body))
    }
}

/// What the balance route's status means: read it, ask the Admin API, or
/// stop. Upstream's branch, kept pure so the tests can pin it.
pub enum Outcome {
    Read,
    Probe,
    RateLimited,
    ServerError,
}

pub fn grants_outcome(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Read,
        429 => Outcome::RateLimited,
        s if (500..600).contains(&s) => Outcome::ServerError,
        // A redirect is never followed — the one it sends is to the sign-in
        // page — and every other answer is the route turning this key away,
        // which is not yet the key being bad.
        _ => Outcome::Probe,
    }
}

/// The shared classifier's word for a probe status; a 2xx never reaches it.
pub fn probe_reason(status: u16) -> Unavailability {
    match status {
        300..=399 | 401 | 403 => Unavailability::ApiKeyRefused,
        429 => Unavailability::RateLimited,
        _ => Unavailability::ServerError,
    }
}

/// One day of the organization's costs, asked for only to learn whether the
/// key is an Admin key. The figures are not read.
pub fn costs_probe_url(now_secs: i64) -> String {
    format!("{COSTS_PROBE}?start_time={}&limit=1", now_secs - 86_400)
}

#[derive(Deserialize)]
struct Reply {
    #[serde(rename = "total_granted")]
    total_granted: Option<f64>,
    #[serde(rename = "total_used")]
    total_used: Option<f64>,
    #[serde(rename = "total_available")]
    total_available: Option<f64>,
}

/// The route's figures are dollars. It names no currency because the
/// platform bills in one.
const CURRENCY: &str = "USD";

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    let Ok(reply) = serde_json::from_value::<Reply>(reply.clone()) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    // A reply carrying none of the three figures is one this reader does not
    // understand; anything else names the account's money.
    if reply.total_granted.is_none()
        && reply.total_used.is_none()
        && reply.total_available.is_none()
    {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    // A balance that isn't one is left off, and then there is nothing.
    let Some(available) = reply
        .total_available
        .filter(|available| available.is_finite() && *available >= 0.0)
    else {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    };
    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.credit_balance = Some(money_text(available));
    usage.credit_remaining = Some(CreditAmount {
        amount: available,
        currency: CURRENCY.into(),
    });
    usage
}

/// The balance as money — symbol first, cents kept. A negative balance is
/// money owed, and the sign leads the figure the way a currency formatter
/// writes it.
fn money_text(amount: f64) -> String {
    if amount < 0.0 {
        format!("-${:.2}", -amount)
    } else {
        format!("${amount:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::OpenAiPlatform))
    }

    #[test]
    fn the_balance_reads_live_without_a_ring() {
        let usage =
            reading_for(json!({"total_granted": 20.0, "total_used": 7.5, "total_available": 12.5}));
        assert!(matches!(usage.state, State::Live));
        assert!(usage.windows.is_empty());
        assert_eq!(usage.credit_balance.as_deref(), Some("$12.50"));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 12.5,
                currency: "USD".into(),
            })
        );
    }

    #[test]
    fn a_reply_without_any_figure_is_unreadable() {
        let usage = reading_for(json!({"object": "credit_summary"}));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }

    #[test]
    fn a_drifting_type_is_unreadable() {
        // The figures are numbers; strings fail the decode the way Swift's
        // would.
        let usage = reading_for(json!({"total_available": "12.50"}));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }

    #[test]
    fn granted_without_an_available_balance_is_no_limits() {
        let usage = reading_for(json!({"total_granted": 20.0, "total_used": 7.5}));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn a_negative_available_balance_reports_no_limits() {
        let usage = reading_for(json!({"total_available": -1.0}));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn statuses_the_route_answers_directly() {
        assert!(matches!(grants_outcome(200), Outcome::Read));
        assert!(matches!(grants_outcome(204), Outcome::Read));
        assert!(matches!(grants_outcome(429), Outcome::RateLimited));
        assert!(matches!(grants_outcome(500), Outcome::ServerError));
        // Every other answer — refused, redirect, unknown — probes.
        assert!(matches!(grants_outcome(401), Outcome::Probe));
        assert!(matches!(grants_outcome(403), Outcome::Probe));
        assert!(matches!(grants_outcome(404), Outcome::Probe));
        assert!(matches!(grants_outcome(302), Outcome::Probe));
        assert!(matches!(grants_outcome(400), Outcome::Probe));
    }

    #[test]
    fn the_probe_is_the_admin_cost_route_one_day_back() {
        assert_eq!(
            costs_probe_url(1_000_000_000),
            "https://api.openai.com/v1/organization/costs?start_time=999913600&limit=1"
        );
    }

    #[test]
    fn probe_statuses_map_like_the_shared_classifier() {
        assert_eq!(probe_reason(401), Unavailability::ApiKeyRefused);
        assert_eq!(probe_reason(403), Unavailability::ApiKeyRefused);
        assert_eq!(probe_reason(302), Unavailability::ApiKeyRefused);
        assert_eq!(probe_reason(429), Unavailability::RateLimited);
        assert_eq!(probe_reason(500), Unavailability::ServerError);
    }
}
