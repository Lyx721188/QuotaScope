//! Optional OpenCLI Browser Bridge cookie import for protected browser stores.
//! Only explicit "all browsers" imports reach this local, credential-free RPC.
use super::{name_matches, Browser, Found};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Read;
use std::time::{Duration, Instant};

const ADDRESS: &str = "http://127.0.0.1:19825";
const MAX_BYTES: u64 = 1024 * 1024;
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) fn session(hosts: &[&str], names: &[&str]) -> Option<Found> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_millis(500))
        .build()
        .ok()?;
    read_with(hosts, names, |body, timeout| {
        let request = match body {
            Some(body) => client.post(format!("{ADDRESS}/command")).json(body),
            None => client.get(format!("{ADDRESS}/status")),
        };
        let response = request
            .header("X-OpenCLI", "1")
            .timeout(timeout)
            .send()
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let mut bytes = Vec::new();
        response.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() as u64 > MAX_BYTES {
            return None;
        }
        serde_json::from_slice(&bytes).ok()
    })
}

fn read_with(
    hosts: &[&str],
    names: &[&str],
    mut request: impl FnMut(Option<&Value>, Duration) -> Option<Value>,
) -> Option<Found> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = request(None, Duration::from_millis(750))?;
    if status["ok"] != true {
        return None;
    }
    let profiles = status["profiles"].as_array()?;
    let session = format!(
        "quotascope-import-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    for profile in profiles.iter().take(16) {
        if profile["extensionConnected"] != true {
            continue;
        }
        let Some(context) = profile["contextId"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
        else {
            continue;
        };
        for host in hosts.iter().take(16) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let now = crate::timeutil::now_ms();
            let body = json!({
                "id": format!("quotascope-cookie-{}-{}",std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
                "action":"cookies", "contextId": context,
                "session":session, "surface":"adapter", "siteSession":"ephemeral",
                "idleTimeout":15, "windowMode":"background",
                "url":format!("https://{host}/"),
                "timeout":remaining.as_secs().max(1),
                "deadlineAt":now.saturating_add(remaining.as_millis() as i64),
            });
            let Some(reply) = request(Some(&body), remaining) else {
                continue;
            };
            if reply["ok"] != true {
                continue;
            }
            if let Some(header) = header(&reply["data"], host, names, now) {
                return Some(Found {
                    browser: Browser::Connected,
                    header,
                });
            }
        }
    }
    None
}

fn header(rows: &Value, host: &str, names: &[&str], now: i64) -> Option<String> {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    for row in rows.as_array()?.iter().take(4096) {
        let (Some(domain), Some(name), Some(value)) = (
            row["domain"].as_str(),
            row["name"].as_str(),
            row["value"].as_str(),
        ) else {
            continue;
        };
        let domain = domain.trim_start_matches('.');
        if domain.is_empty() || (host != domain && !host.ends_with(&format!(".{domain}"))) {
            continue;
        }
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || !names.iter().any(|wanted| name_matches(name, wanted))
            || value.is_empty()
            || value.len() > 64 * 1024
            || value.bytes().any(|b| !(33..=126).contains(&b) || b == b';')
            || row["expirationDate"]
                .as_f64()
                .is_some_and(|at| at <= now as f64 / 1000.0)
        {
            continue;
        }
        if seen.insert(name) {
            kept.push(format!("{name}={value}"));
        }
    }
    (!kept.is_empty()).then(|| kept.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_requested_live_cookies_from_the_requested_site_are_imported() {
        let rows = json!([
            {"domain":".example.com","name":"session","value":"accepted","httpOnly":true},
            {"domain":"example.com","name":"analytics","value":"skip"},
            {"domain":"notexample.com","name":"session","value":"foreign"},
            {"domain":"example.com","name":"session_other","value":"bad\r\nheader"},
            {"domain":"example.com","name":"session_old","value":"expired","expirationDate":1},
            {"domain":"example.com","name":"session","value":"duplicate"}
        ]);
        assert_eq!(
            header(&rows, "api.example.com", &["session*"], 2000).as_deref(),
            Some("session=accepted")
        );
        assert!(header(&rows, "other.test", &["session*"], 2000).is_none());
    }
    #[test]
    fn bridge_requests_pin_the_connected_profile_and_cookie_url() {
        let mut calls = 0;
        let found=read_with(&["example.com"], &["session"], |body,_| {
            calls+=1;
            match body {
                None => Some(json!({"ok":true,"extensionConnected":true,"profiles":[{"contextId":"fixture","extensionConnected":true}]})),
                Some(body) => {
                    assert_eq!(body["action"],"cookies");
                    assert_eq!(body["contextId"],"fixture");
                    assert_eq!(body["url"],"https://example.com/");
                    Some(json!({"ok":true,"data":[{"domain":"example.com","name":"session","value":"test"}]}))
                }
            }
        }).unwrap();
        assert_eq!(found.browser, Browser::Connected);
        assert_eq!(found.header, "session=test");
        assert_eq!(calls, 2);
    }
}
