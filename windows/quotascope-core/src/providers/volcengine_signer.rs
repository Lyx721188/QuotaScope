//! Volcengine HMAC-SHA256 signing: cn-beijing/ark/request, sorted headers.
use crate::http::{number, HttpClient, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
fn mac(key: &[u8], text: &str) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts arbitrary key lengths");
    mac.update(text.as_bytes());
    mac.finalize().into_bytes().to_vec()
}
pub fn headers(
    access: &str,
    secret: &str,
    action: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<(String, String)> {
    let stamp = now.format("%Y%m%dT%H%M%SZ").to_string();
    let day = now.format("%Y%m%d").to_string();
    let hash = hex(Sha256::digest([]));
    let content = "application/x-www-form-urlencoded; charset=utf-8";
    let host = "open.volcengineapi.com";
    let names = "content-type;host;x-content-sha256;x-date";
    let query = format!("Action={action}&Version=2024-01-01");
    let canonical=format!("GET\n/\n{query}\ncontent-type:{content}\nhost:{host}\nx-content-sha256:{hash}\nx-date:{stamp}\n\n{names}\n{hash}");
    let scope = format!("{day}/cn-beijing/ark/request");
    let string = format!(
        "HMAC-SHA256\n{stamp}\n{scope}\n{}",
        hex(Sha256::digest(canonical.as_bytes()))
    );
    let mut key = secret.as_bytes().to_vec();
    for part in [&day, "cn-beijing", "ark", "request"] {
        key = mac(&key, part);
    }
    vec![
        ("Content-Type".into(), content.into()),
        ("Host".into(), host.into()),
        ("X-Date".into(), stamp),
        ("X-Content-Sha256".into(), hash),
        (
            "Authorization".into(),
            format!(
                "HMAC-SHA256 Credential={access}/{scope}, SignedHeaders={names}, Signature={}",
                hex(mac(&key, &string))
            ),
        ),
    ]
}
pub fn fetch(http: &HttpClient, credential: &str) -> Result<ProviderUsage, Unavailability> {
    let (access, secret) = credential
        .split_once(':')
        .or_else(|| credential.split_once('|'))
        .ok_or(Unavailability::ApiKeyRefused)?;
    if access.trim().is_empty() || secret.trim().is_empty() {
        return Err(Unavailability::ApiKeyRefused);
    }
    let mut windows = Vec::new();
    let mut errors = Vec::new();
    for action in ["GetCodingPlanUsage", "GetAFPUsage"] {
        let headers = headers(access.trim(), secret.trim(), action, chrono::Utc::now());
        let refs: Vec<_> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        match http.fetch_json(
            Method::Get,
            &format!("https://open.volcengineapi.com/?Action={action}&Version=2024-01-01"),
            &refs,
            None,
        ) {
            Ok(reply) => windows.extend(parse(&reply, action)),
            Err(reason) => errors.push(reason),
        }
    }
    if windows.is_empty() {
        return Err(errors
            .iter()
            .copied()
            .find(|r| {
                matches!(
                    r,
                    Unavailability::ApiKeyRefused | Unavailability::RateLimited
                )
            })
            .or_else(|| errors.first().copied())
            .unwrap_or(Unavailability::NoLimitsReported));
    }
    let mut usage = ProviderUsage::live_now(AccountKey::primary(Provider::Volcengine), windows);
    usage.origin = Some("endpoint".into());
    Ok(usage)
}
pub fn parse(reply: &serde_json::Value, action: &str) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    let root = &reply["Result"];
    if action == "GetCodingPlanUsage" {
        for quota in root["QuotaUsage"].as_array().into_iter().flatten() {
            if let (Some(level), Some(percent)) = (
                quota["QuotaLevel"]
                    .as_str()
                    .or_else(|| quota["Level"].as_str()),
                number(&quota["Percent"]),
            ) {
                if let Some(w) = window(
                    level,
                    percent,
                    super::remaining::stamp(&quota["ResetTimestamp"]),
                    "Coding Plan",
                ) {
                    windows.push(w);
                }
            }
        }
    } else {
        for (label, key) in [
            ("fiveHour", "AFPFiveHour"),
            ("weekly", "AFPWeekly"),
            ("monthly", "AFPMonthly"),
        ] {
            let row = &root[key];
            if let (Some(quota), Some(used)) = (
                number(&row["Quota"]).filter(|n| *n > 0.0),
                number(&row["Used"]),
            ) {
                if let Some(w) = window(
                    if label == "fiveHour" { "5h" } else { label },
                    used / quota * 100.0,
                    super::remaining::stamp(&row["ResetTime"]),
                    "Agent Plan",
                ) {
                    windows.push(w);
                }
            }
        }
    }
    windows
}
pub fn window(label: &str, percent: f64, reset: Option<i64>, scope: &str) -> Option<UsageWindow> {
    if !percent.is_finite() || percent < 0.0 {
        return None;
    }
    let (kind, seconds) = match label.to_lowercase().as_str() {
        "5h" | "5-hour" | "five_hour" | "session" => (Kind::FiveHour, 18000),
        "week" | "weekly" => (Kind::Weekly, 604800),
        "month" | "monthly" => (Kind::Monthly, 2592000),
        _ => return None,
    };
    let mut window = UsageWindow::new(
        &format!("{scope}.{label}"),
        kind,
        Some(scope.into()),
        (percent / 100.0).min(1.0),
        seconds,
        reset,
    );
    window.reports_length = kind != Kind::Monthly;
    window.is_exhausted = percent >= 100.0;
    Some(window)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signature_matches_independent_upstream_vector() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-07T09:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let a = headers(
            "AKLTTestAccessKeyId",
            "dGVzdC1zZWNyZXQtYWNjZXNzLWtleQ==",
            "GetCodingPlanUsage",
            now,
        );
        assert_eq!(a.last().unwrap().1, "HMAC-SHA256 Credential=AKLTTestAccessKeyId/20260907/cn-beijing/ark/request, SignedHeaders=content-type;host;x-content-sha256;x-date, Signature=3bc6ebb4fd6da065cae0c05dbfc35285cdced26090d7c0aea87b2f2330cd031d");
        assert!(window("unknown", 20.0, None, "Coding Plan").is_none());
    }
    #[test]
    fn coding_and_afp_shapes_match_upstream_fixtures() {
        let coding = parse(
            &serde_json::from_str(include_str!(
                "../../tests/fixtures/upstream-volcengine-coding-plan.json"
            ))
            .unwrap(),
            "GetCodingPlanUsage",
        );
        let agent = parse(
            &serde_json::from_str(include_str!(
                "../../tests/fixtures/upstream-volcengine-agent-plan.json"
            ))
            .unwrap(),
            "GetAFPUsage",
        );
        assert_eq!(coding.len(), 2); // Unknown fortnightly window is omitted.
        assert_eq!(agent.len(), 2); // Zero monthly quota is omitted.
        assert!(coding.iter().any(|w| w.kind == Kind::FiveHour));
        assert!(agent.iter().any(|w| w.kind == Kind::Weekly));
    }
}
