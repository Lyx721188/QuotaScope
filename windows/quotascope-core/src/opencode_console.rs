//! OpenCode's console session is separate from its Go API key. Only the
//! quota and billing fields are retained; request headers and locations are not.
use crate::http::{number, HttpClient, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::Value;

pub const SECRET: &str = "openCodeGo:console";
pub const HOSTS: &[&str] = &["opencode.ai"];
pub const COOKIES: &[&str] = &["__Host-console_session", "auth"];
const BASE: &str = "https://opencode.ai/console/api";

pub fn cookie(header: &str) -> Option<String> {
    let pairs: Vec<_> = header
        .trim()
        .trim_start_matches("Cookie:")
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(name, value)| COOKIES.contains(name) && !value.trim().is_empty())
        .map(|(name, value)| format!("{name}={}", value.trim()))
        .collect();
    pairs
        .iter()
        .any(|p| p.starts_with("__Host-console_session="))
        .then(|| pairs.join("; "))
}

fn session_problem(reason: Unavailability) -> Unavailability {
    if reason == Unavailability::ApiKeyRefused {
        Unavailability::SessionExpired
    } else {
        reason
    }
}

fn get(http: &HttpClient, cookie: &str, org: &str, url: &str) -> Result<Value, Unavailability> {
    http.fetch_json(
        Method::Get,
        url,
        &[
            ("Cookie", cookie),
            ("x-org-id", org),
            ("Accept", "application/json"),
        ],
        None,
    )
    .map_err(session_problem)
}

fn workspace(http: &HttpClient, cookie: &str) -> Result<String, Unavailability> {
    let root = get(http, cookie, "", &format!("{BASE}/orgs"))?;
    let orgs: Vec<_> = root
        .as_array()
        .ok_or(Unavailability::UnreadableReply)?
        .iter()
        .filter_map(|org| org.get("id").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .collect();
    let first = orgs.first().ok_or(Unavailability::NoPlan)?;
    if orgs.len() > 1 {
        for org in &orgs {
            match get(http, cookie, org, &format!("{BASE}/go/status")) {
                Ok(status) if status.get("access").is_some_and(Value::is_object) => {
                    return Ok((*org).to_string())
                }
                Err(Unavailability::SessionExpired) => return Err(Unavailability::SessionExpired),
                _ => {}
            }
        }
    }
    Ok((*first).to_string())
}

pub fn usage(http: &HttpClient, header: &str) -> ProviderUsage {
    let account = AccountKey::primary(Provider::OpenCodeGo);
    let result = (|| {
        let cookie = cookie(header).ok_or(Unavailability::SessionMissing)?;
        let org = workspace(http, &cookie)?;
        get(http, &cookie, &org, &format!("{BASE}/go/status"))
    })();
    match result {
        Err(reason) => ProviderUsage::unavailable(account, reason),
        Ok(reply) => {
            let windows = windows(&reply);
            if windows.is_empty() {
                return ProviderUsage::unavailable(
                    account,
                    if reply.get("access").is_some_and(Value::is_object) {
                        Unavailability::NoLimitsReported
                    } else {
                        Unavailability::NoPlan
                    },
                );
            }
            let mut usage = ProviderUsage::live_now(account, windows);
            usage.origin = Some("webSession".into());
            usage.plan =
                (reply.get("product").and_then(Value::as_str) == Some("go")).then(|| "Go".into());
            usage
        }
    }
}

pub fn windows(reply: &Value) -> Vec<UsageWindow> {
    let access = &reply["access"];
    let meters = &access["meters"];
    [
        ("fiveHour", "rolling", Kind::FiveHour, 18000),
        ("week", "weekly", Kind::Weekly, 604800),
        ("month", "monthly", Kind::Monthly, 2592000),
    ]
    .into_iter()
    .filter_map(|(name, id, kind, nominal)| {
        let meter = &meters[name];
        let limit = number(&meter["limitMicroCents"]).filter(|n| n.is_finite() && *n > 0.0)?;
        let used = number(&meter["usedMicroCents"]).filter(|n| n.is_finite() && *n >= 0.0)?;
        let reset = meter["resetsAt"]
            .as_str()
            .and_then(crate::timeutil::parse_iso8601_ms);
        let start = if name == "month" && access["endsAt"] == meter["resetsAt"] {
            &access["startsAt"]
        } else {
            &meter["startsAt"]
        };
        let length = start
            .as_str()
            .and_then(crate::timeutil::parse_iso8601_ms)
            .zip(reset)
            .and_then(|(s, r)| r.checked_sub(s))
            .map(|ms| ms / 1000)
            .filter(|s| *s > 0);
        let mut window = UsageWindow::new(
            id,
            kind,
            None,
            (used / limit).clamp(0.0, 1.0),
            length.unwrap_or(nominal),
            reset,
        );
        window.reports_length = length.is_some();
        window.is_exhausted = used >= limit;
        Some(window)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn console_session_drops_unrelated_cookies_and_measures_only_stated_windows() {
        assert_eq!(
            cookie("auth=a; _stripe=b; __Host-console_session=c"),
            Some("auth=a; __Host-console_session=c".into())
        );
        assert!(cookie("auth=a").is_none());
        let windows = windows(
            &json!({"access":{"meters":{"fiveHour":{"limitMicroCents":"100", "usedMicroCents":"150", "startsAt":"2026-10-01T00:00:00Z", "resetsAt":"2026-10-01T05:00:00Z"},"month":{"limitMicroCents":"100","usedMicroCents":"25"}}}}),
        );
        assert!(windows[0].reports_length && windows[0].is_exhausted);
        assert!(!windows[1].reports_length);
        assert_eq!(windows[1].used_fraction, 0.25);
    }
}
