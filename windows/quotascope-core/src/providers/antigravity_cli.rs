//! Read-only agy /usage. Versions before 1.1.11 can send it as a model prompt,
//! so an unreadable or older version is never asked for usage.
use crate::model::{AccountKey, Provider, ProviderUsage, Unavailability};
use std::process::Command;
use std::time::Duration;

pub fn locate() -> Option<std::path::PathBuf> {
    crate::local_process::locate("agy").or_else(|| {
        [
            ".gemini/bin/agy.exe",
            ".gemini/bin/agy.cmd",
            ".local/bin/agy.exe",
            ".local/bin/agy.cmd",
        ]
        .into_iter()
        .map(crate::model::home_path)
        .find(|p| p.is_file())
    })
}

pub fn supported_version(text: &str) -> bool {
    let parts: Option<Vec<u32>> = text
        .trim()
        .split('.')
        .map(|s| {
            let digits: String = s.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        })
        .collect();
    let Some(parts) = parts.filter(|p| p.len() >= 2) else {
        return false;
    };
    let version = [parts[0], parts[1], *parts.get(2).unwrap_or(&0)];
    version >= [1, 1, 11]
}

pub fn fetch() -> ProviderUsage {
    let account = AccountKey::primary(Provider::Antigravity);
    let unavailable = |r| ProviderUsage::unavailable(account.clone(), r);
    let Some(binary) = locate() else {
        return unavailable(Unavailability::AntigravityCliMissing);
    };
    let mut version = Command::new(&binary);
    version.arg("--version").current_dir(std::env::temp_dir());
    let Ok(bytes) = crate::local_process::run(version, Duration::from_secs(10), 4096) else {
        return unavailable(Unavailability::AntigravityCliOutdated);
    };
    if !supported_version(&String::from_utf8_lossy(&bytes)) {
        return unavailable(Unavailability::AntigravityCliOutdated);
    }
    let mut command = Command::new(binary);
    #[cfg(windows)]
    let null = "NUL";
    #[cfg(not(windows))]
    let null = "/dev/null";
    command
        .args([
            "--log-file",
            null,
            "-p",
            "/usage",
            "--output-format",
            "json",
        ])
        .current_dir(std::env::temp_dir());
    match crate::local_process::run(command, Duration::from_secs(30), 512 * 1024) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(|root| reading(&root))
            .unwrap_or_else(|_| unavailable(Unavailability::UnreadableReply)),
        Err(crate::local_process::Failure::TimedOut) => unavailable(Unavailability::Unreachable),
        Err(_) => unavailable(Unavailability::UnreadableReply),
    }
}

pub fn reading(root: &serde_json::Value) -> ProviderUsage {
    use serde_json::{json, Value};
    let account = AccountKey::primary(Provider::Antigravity);
    if root.get("status").and_then(Value::as_str) != Some("SUCCESS")
        || !root.pointer("/command/data").is_some_and(Value::is_object)
    {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    let groups: Vec<_> = root.pointer("/command/data/groups").and_then(Value::as_array).into_iter().flatten().map(|group| {
        let buckets: Vec<_> = group.get("buckets").and_then(Value::as_array).into_iter().flatten().map(|b| json!({
            "bucketId":b.get("id"),"window":b.get("window"),"remainingFraction":b.get("remaining_fraction"),"resetTime":b.get("reset_time")
        })).collect();
        json!({"displayName":group.get("name"),"buckets":buckets})
    }).collect();
    let windows = super::antigravity::windows_from_reply(&json!({"response":{"groups":groups}}));
    if windows.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.origin = Some("antigravityCLI".into());
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsafe_or_ambiguous_versions_are_never_asked() {
        for v in ["1.1.10", "1.0.99", "agy 1.3.5", "", "1", "1.1.bad"] {
            assert!(!supported_version(v), "{v}");
        }
        for v in ["1.1.11", "1.3.5", "2.0", "1.3.5-beta"] {
            assert!(supported_version(v), "{v}");
        }
    }
    #[test]
    fn cli_groups_use_the_same_mapping_as_the_desktop() {
        let usage = reading(
            &serde_json::json!({"status":"SUCCESS","command":{"data":{"groups":[{"name":"Gemini 4 Argon Models","buckets":[{"id":"argon-week","window":"weekly","remaining_fraction":0.7}]}]}}}),
        );
        assert_eq!(usage.windows.len(), 1);
        assert_eq!(usage.windows[0].scope.as_deref(), Some("Gemini 4 Argon"));
        assert!((usage.windows[0].used_fraction - 0.3).abs() < 1e-12);
        assert_eq!(usage.origin.as_deref(), Some("antigravityCLI"));
    }
}
