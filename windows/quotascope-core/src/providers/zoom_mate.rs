//! ZoomMate, Zoom's AI assistant: one credit allowance against a budget cap,
//! for a billing cycle whose start and end the service states.
//!
//! Read with a Zoom browser session, the way ZoomMate's own web client does:
//! the session is exchanged for a short-lived bearer token at
//! `GET /ai-computer/api/v1/login/?continue=https://zoommate.zoom.us/`, and
//! that token reads `GET /ai-computer/api/v1/credits/status`. Both are on
//! `ai.zoom.us`, with `zoommate.zoom.us` — which serves the same API — tried
//! when the first does not answer. The token is held for the one refresh and
//! never stored; nothing is written anywhere. The shapes are second-hand —
//! taken from CodexBar's ZoomMate provider and its tests, not from a
//! captured reply — and so are the cookie names.
//!
//! Left out: the credit history and the pace built on it. That is spend over
//! thirty days, which there is no place for, and pace is work the app does
//! from the cycle it already draws.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button. Zoom's session
/// cookie, and Cloudflare's clearance beside it — both are set on the parent
/// `zoom.us`, which is why that is the host read.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["zoom.us"],
    cookies: &["_zm_ssid", "cf_clearance"],
};

/// The two hosts that serve ZoomMate's API, in the order its web client
/// uses them. Nothing is sent anywhere else.
const HOSTS: [&str; 2] = ["ai.zoom.us", "zoommate.zoom.us"];
const ORIGIN: &str = "https://zoommate.zoom.us";

pub struct ZoomMateService {
    http: Arc<HttpClient>,
}

impl ZoomMateService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// The session exchanged for a token, then the token for the status —
    /// both on the same host.
    fn status(&self, host: &str, cookie: &str) -> Result<Value, Unavailability> {
        let login_url = format!("https://{host}/ai-computer/api/v1/login/?continue={ORIGIN}/");
        let status_url = format!("https://{host}/ai-computer/api/v1/credits/status");
        let login = self.get(&login_url, cookie, None)?;
        let token = token_from(&login)?;
        self.get(&status_url, cookie, Some(&token))
    }

    /// One call, with the session always and the token when it has been
    /// minted. The shared fetch folds 401, 403 and the unfollowed redirect
    /// into the refused key, which is the wrong story for a browser session
    /// — those all mean the session no longer works.
    fn get(&self, url: &str, cookie: &str, token: Option<&str>) -> Result<Value, Unavailability> {
        let mut headers = vec![
            // What the web client's own calls carry.
            ("Accept", "application/json, text/plain, */*".to_string()),
            ("Origin", ORIGIN.to_string()),
            ("Referer", ORIGIN.to_string()),
            ("Cookie", cookie.to_string()),
        ];
        if let Some(token) = token {
            headers.push(("Authorization", format!("Bearer {token}")));
        }
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        session_json(&self.http, Method::Get, url, &refs, None)
    }
}

impl ProviderService for ZoomMateService {
    fn provider(&self) -> Provider {
        Provider::ZoomMate
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::ZoomMate);
        let Some(header) = pasted_or_none(keys.api_key(Provider::ZoomMate)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let Some(cookie) = keep(&header, SESSION.cookies) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        let mut last = Unavailability::Unreachable;
        for host in HOSTS {
            match self.status(host, &cookie) {
                Ok(reply) => {
                    let mut usage = reading(&reply, account);
                    if matches!(usage.state, crate::model::State::Live) {
                        usage.origin = Some(self.origin_token().into());
                    }
                    return usage;
                }
                // Only a host that did not answer is worth asking the other
                // one about. A refused session or an unreadable reply would
                // be the same on both.
                Err(reason @ (Unavailability::Unreachable | Unavailability::ServerError)) => {
                    last = reason;
                }
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            }
        }
        ProviderUsage::unavailable(account, last)
    }
}

/// One call with the imported session as its credential. `fetch_json` folds
/// 401, 403 and the unfollowed redirect into `ApiKeyRefused`, which is the
/// pasted key's refusal — for a session those statuses are the session
/// expiring, so the folded reason is read here and remapped. 404 lands in
/// `NotFound`; upstream's classifier calls it a server error like any other.
fn session_json(
    http: &HttpClient,
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&serde_json::Value>,
) -> Result<Value, Unavailability> {
    match http.fetch_json_detailed(method, url, headers, body) {
        Ok(value) => Ok(value),
        Err(HttpFailure::NotFound) => Err(Unavailability::ServerError),
        Err(HttpFailure::Unavailable(Unavailability::ApiKeyRefused)) => {
            Err(Unavailability::SessionExpired)
        }
        Err(HttpFailure::Unavailable(reason)) => Err(reason),
    }
}

/// The login reply, decoded strictly: a field that arrived the wrong shape
/// spoils the whole reply, the way the upstream decode fails.
#[derive(Deserialize)]
struct LoginReply {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    data: Option<LoginPayload>,
}

#[derive(Deserialize)]
struct LoginPayload {
    #[serde(default)]
    nak: Option<String>,
}

/// The bearer token is `data.nak`. A reply that says it failed is the
/// session being turned away.
pub fn token_from(reply: &Value) -> Result<String, Unavailability> {
    // The decode reads an object and nothing else; a sequence would not
    // reach it.
    if !reply.is_object() {
        return Err(Unavailability::UnreadableReply);
    }
    let Ok(login) = serde_json::from_value::<LoginReply>(reply.clone()) else {
        return Err(Unavailability::UnreadableReply);
    };
    if let Some(nak) = login
        .data
        .and_then(|data| data.nak)
        .map(|nak| nak.trim().to_string())
        .filter(|nak| !nak.is_empty())
    {
        return Ok(nak);
    }
    if login.success == Some(false) {
        return Err(Unavailability::SessionExpired);
    }
    Err(Unavailability::UnreadableReply)
}

/// The credit status reply, decoded strictly like the login's.
#[derive(Deserialize)]
struct StatusReply {
    #[serde(default)]
    data: Option<StatusPayload>,
}

#[derive(Deserialize)]
struct StatusPayload {
    #[serde(default, rename = "credit_status")]
    credit_status: Option<CreditStatus>,
}

#[derive(Deserialize)]
struct CreditStatus {
    #[serde(default, rename = "budget_cap")]
    budget_cap: Option<f64>,
    #[serde(default, rename = "used_credit")]
    used_credit: Option<f64>,
    #[serde(default, rename = "remaining_credit")]
    remaining_credit: Option<f64>,
    #[serde(default, rename = "allow_overage")]
    allow_overage: Option<bool>,
    #[serde(default, rename = "cycle_start_date")]
    cycle_start_date: Option<f64>,
    #[serde(default, rename = "cycle_end_date")]
    cycle_end_date: Option<f64>,
    #[serde(default, rename = "is_quota_available")]
    is_quota_available: Option<bool>,
    #[serde(default, rename = "is_unlimited")]
    is_unlimited: Option<bool>,
}

/// The credit status, as the one allowance the service reports.
pub fn reading(reply: &Value, account: AccountKey) -> ProviderUsage {
    // The decode reads an object and nothing else; a sequence would not
    // reach it.
    if !reply.is_object() {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    let Ok(reply) = serde_json::from_value::<StatusReply>(reply.clone()) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let Some(status) = reply.data.and_then(|data| data.credit_status) else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    // Unlimited has no cap to measure against, and a cap of nothing is not
    // one either. Neither is drawn as 0%.
    if status.is_unlimited == Some(true) {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let Some(cap) = status
        .budget_cap
        .filter(|cap| cap.is_finite() && *cap > 0.0)
    else {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    };
    let used = match status.used_credit {
        Some(used) => Some(used),
        None => status.remaining_credit.map(|remaining| cap - remaining),
    }
    .filter(|used| used.is_finite() && *used >= 0.0);
    let Some(used) = used else {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    };

    let start = date(status.cycle_start_date);
    let end = date(status.cycle_end_date);
    // The cycle's length is stated when both of its ends are. The dates
    // travel as milliseconds; the window is measured in seconds.
    let stated = start
        .and_then(|start| end.map(|end| (end - start) / 1000))
        .filter(|length| *length > 0);
    let fraction = used / cap;
    let mut window = UsageWindow::new(
        "zoommate.credits",
        // Upstream kinds the allowance `.credits`; Spend is the Windows
        // stand-in for it, as elsewhere in the port.
        Kind::Spend,
        None,
        fraction,
        stated.unwrap_or(30 * 86_400),
        end,
    );
    window.reports_length = stated.is_some();
    // Past the cap is only the end if overage isn't allowed.
    window.is_exhausted = status.is_quota_available == Some(false)
        || (fraction >= 1.0 && status.allow_overage != Some(true));
    ProviderUsage::live_now(account, vec![window])
}

/// Epoch milliseconds, already given as such. A figure that names no time at
/// all is no date.
fn date(milliseconds: Option<f64>) -> Option<i64> {
    let milliseconds = milliseconds.filter(|ms| ms.is_finite() && *ms > 0.0)?;
    Some(milliseconds as i64)
}

/// Keeps only the named cookies out of a pasted `Cookie:` header, and
/// nothing at all if the first name is missing. What is not kept never
/// leaves the process.
fn keep(header: &str, cookies: &[&str]) -> Option<String> {
    let first = cookies.first()?;
    let required: Vec<&str> = first.split('|').collect();
    let patterns: Vec<&str> = cookies
        .iter()
        .flat_map(|cookie| cookie.split('|'))
        .collect();
    let matches = |name: &str, pattern: &str| -> bool {
        match pattern.strip_suffix('*') {
            Some(prefix) => {
                !prefix.is_empty() && name.starts_with(prefix) && name.len() > prefix.len()
            }
            None => name == pattern,
        }
    };
    let pairs: Vec<(&str, &str)> = header
        .split(';')
        .filter_map(|part| {
            let trimmed = part.trim();
            let equals = trimmed.find('=')?;
            let (name, value) = (&trimmed[..equals], &trimmed[equals + 1..]);
            (!value.is_empty() && patterns.iter().any(|pattern| matches(name, pattern)))
                .then_some((name, value))
        })
        .collect();
    if !pairs
        .iter()
        .any(|(name, _)| required.iter().any(|want| matches(name, want)))
    {
        return None;
    }
    let mut seen: Vec<&str> = Vec::new();
    let kept: Vec<String> = pairs
        .into_iter()
        .filter(|(name, _)| {
            if seen.contains(name) {
                return false;
            }
            seen.push(name);
            true
        })
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    Some(kept.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn account() -> AccountKey {
        AccountKey::primary(Provider::ZoomMate)
    }

    /// The shape pinned by CodexBar's ZoomMate plugin — second-hand, not
    /// captured from a live account.
    fn status_reply() -> serde_json::Value {
        json!({
            "data": {
                "credit_status": {
                    "budget_cap": 30.0,
                    "used_credit": 7.5,
                    "remaining_credit": 22.5,
                    "allow_overage": false,
                    "cycle_start_date": 1_782_000_000_000i64,
                    "cycle_end_date": 1_784_600_000_000i64,
                    "is_quota_available": true,
                    "is_unlimited": false
                }
            }
        })
    }

    #[test]
    fn the_allowance_against_its_cap_with_the_stated_cycle() {
        let usage = reading(&status_reply(), account());
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.windows.len(), 1);
        let window = &usage.windows[0];
        assert_eq!(window.id, "zoommate.credits");
        // Upstream kinds this `.credits`; Spend is the Windows stand-in.
        assert_eq!(window.kind, Kind::Spend);
        assert_eq!(window.used_fraction, 0.25);
        // Both ends stated: the length between them is the cycle's.
        assert_eq!(window.window_seconds, 2_600_000);
        assert!(window.reports_length);
        assert_eq!(window.resets_at, Some(1_784_600_000_000));
        assert!(!window.is_exhausted);
    }

    #[test]
    fn one_end_only_makes_the_month_a_sort_key_and_the_used_falls_back_to_the_remainder() {
        let reply = json!({
            "data": {"credit_status": {
                "budget_cap": 30.0,
                "remaining_credit": 22.5,
                "cycle_end_date": 1_784_600_000_000i64
            }}
        });
        let usage = reading(&reply, account());
        let window = &usage.windows[0];
        // Used is what is gone from the cap, not what is left of it.
        assert_eq!(window.used_fraction, 0.25);
        assert!(!window.reports_length);
        assert_eq!(window.resets_at, Some(1_784_600_000_000));
    }

    #[test]
    fn the_cap_says_when_the_allowance_is_over() {
        let spent = json!({
            "data": {"credit_status": {
                "budget_cap": 10.0, "used_credit": 10.0,
                "allow_overage": false, "is_quota_available": true
            }}
        });
        assert!(reading(&spent, account()).windows[0].is_exhausted);

        // Past the cap is only the end if overage isn't allowed.
        let overage = json!({
            "data": {"credit_status": {
                "budget_cap": 10.0, "used_credit": 12.0,
                "allow_overage": true, "is_quota_available": true
            }}
        });
        let window = &reading(&overage, account()).windows[0];
        assert!(!window.is_exhausted);
        assert!(window.used_fraction > 1.0);

        // The service's own word ends it either way.
        let refused = json!({
            "data": {"credit_status": {
                "budget_cap": 10.0, "used_credit": 1.0,
                "allow_overage": true, "is_quota_available": false
            }}
        });
        assert!(reading(&refused, account()).windows[0].is_exhausted);
    }

    #[test]
    fn unlimited_or_an_unstated_cap_is_no_limit_never_zero_percent() {
        let unlimited =
            json!({"data": {"credit_status": {"is_unlimited": true, "budget_cap": 30.0}}});
        assert_eq!(
            reading(&unlimited, account()).state,
            State::Unavailable(Unavailability::NoLimitsReported)
        );
        for reply in [
            json!({"data": {"credit_status": {"used_credit": 5.0}}}),
            json!({"data": {"credit_status": {"budget_cap": 0.0, "used_credit": 5.0}}}),
            json!({"data": {"credit_status": {"budget_cap": 30.0}}}),
            json!({"data": {"credit_status": {"budget_cap": 30.0, "used_credit": -1.0}}}),
        ] {
            assert_eq!(
                reading(&reply, account()).state,
                State::Unavailable(Unavailability::NoLimitsReported),
                "{reply}"
            );
        }
    }

    #[test]
    fn a_reply_with_no_status_in_it_is_unreadable() {
        for reply in [json!({}), json!({"data": {}}), json!([1]), json!("no")] {
            assert_eq!(
                reading(&reply, account()).state,
                State::Unavailable(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn the_token_is_the_nak_and_a_failure_is_the_session() {
        assert_eq!(
            token_from(&json!({"success": true, "data": {"nak": "tok"}})).unwrap(),
            "tok"
        );
        // Trimmed, and alone worth having.
        assert_eq!(
            token_from(&json!({"data": {"nak": "  tok  "}})).unwrap(),
            "tok"
        );
        // The reply saying no is the session being turned away; anything
        // else that carries no token is unreadable.
        assert_eq!(
            token_from(&json!({"success": false, "data": {"nak": ""}})),
            Err(Unavailability::SessionExpired)
        );
        assert_eq!(
            token_from(&json!({"success": true})),
            Err(Unavailability::UnreadableReply)
        );
        // A token that arrived the wrong shape spoils the whole reply.
        assert_eq!(
            token_from(&json!({"success": false, "data": {"nak": 3}})),
            Err(Unavailability::UnreadableReply)
        );
        assert_eq!(
            token_from(&json!([1])),
            Err(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn only_the_session_and_the_clearance_are_kept_and_not_without_the_session() {
        assert_eq!(
            keep(
                "__cf_bm=x; _zm_ssid=good; cf_clearance=ok; theme=dark",
                SESSION.cookies
            ),
            Some("_zm_ssid=good; cf_clearance=ok".to_string())
        );
        // The session cookie is the one that must be there.
        assert_eq!(keep("cf_clearance=ok", SESSION.cookies), None);
        assert_eq!(keep("_zm_ssid=", SESSION.cookies), None);
        assert_eq!(keep("", SESSION.cookies), None);
    }
}
