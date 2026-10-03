//! Claude status-line fallback. Store only reported quota counters, never the
//! prompt, transcript, working directory or credential from stdin.
use crate::model::{AccountKey, Provider, ProviderUsage, State};
use serde_json::{json, Value};
use std::io::Read;

fn sanitized(root: &Value, now: i64) -> Option<Value> {
    let mut limits = serde_json::Map::new();
    for key in ["five_hour", "seven_day"] {
        let row = &root["rate_limits"][key];
        let Some(percent) = crate::http::number(&row["used_percentage"])
            .filter(|n| n.is_finite() && (0.0..=101.0).contains(n))
        else {
            continue;
        };
        let reset = row["resets_at"]
            .as_str()
            .and_then(crate::timeutil::parse_iso8601_ms)
            .or_else(|| crate::providers::remaining::stamp(&row["resets_at"]));
        let reset_iso = reset
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|d| d.to_rfc3339());
        limits.insert(
            key.into(),
            json!({"utilization":percent,"resets_at":reset_iso}),
        );
    }
    (!limits.is_empty()).then(|| json!({"observedAt":now,"usage":limits}))
}
pub fn run(input: impl Read) {
    let mut bytes = Vec::new();
    if input.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_err() || bytes.len() > 1024 * 1024 {
        return;
    }
    let Ok(root) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    let Some(saved) = sanitized(&root, crate::timeutil::now_ms()) else {
        return;
    };
    let directory = crate::data_dir();
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let temporary = directory.join(format!("claude-statusline-{}.tmp", std::process::id()));
    let path = directory.join("claude-statusline.json");
    if path
        .symlink_metadata()
        .is_ok_and(|m| !m.file_type().is_file())
    {
        return;
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
    {
        use std::io::Write;
        let result = file.write_all(saved.to_string().as_bytes());
        drop(file);
        if result.is_ok() {
            let _ = std::fs::rename(&temporary, path);
        }
        let _ = std::fs::remove_file(temporary);
    }
    let windows = crate::providers::claude_code::parse_windows(&saved["usage"]);
    print!(
        "{}",
        windows
            .iter()
            .map(|w| format!(
                "{} {:.0}%",
                if w.window_seconds == 18000 {
                    "5h"
                } else {
                    "7d"
                },
                w.used_fraction * 100.0
            ))
            .collect::<Vec<_>>()
            .join(" · ")
    );
}
pub fn read(now: i64) -> Option<ProviderUsage> {
    let path = crate::data_dir().join("claude-statusline.json");
    if path.metadata().ok()?.len() > 8192 {
        return None;
    }
    let saved: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    reading(&saved, now)
}
fn reading(saved: &Value, now: i64) -> Option<ProviderUsage> {
    let observed = saved["observedAt"].as_i64()?;
    if observed > now + 60000 || now.saturating_sub(observed) > 15 * 60000 {
        return None;
    }
    let windows = crate::providers::claude_code::parse_windows(&saved["usage"]);
    if windows.is_empty() {
        return None;
    }
    let mut usage = ProviderUsage::live_now(AccountKey::primary(Provider::ClaudeCode), windows);
    usage.state = State::Stale;
    usage.observed_at = Some(observed);
    usage.origin = Some("statusLine".into());
    usage.is_cached = true;
    Some(usage)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stores_only_reported_percentages_and_expires() {
        let saved = sanitized(&json!({"prompt":"private","rate_limits":{"five_hour":{"used_percentage":0},"seven_day":{"used_percentage":1791000000}}}), 1000000).unwrap();
        assert!(!saved.to_string().contains("private"));
        let usage = reading(&saved, 1000001).unwrap();
        assert_eq!(usage.windows.len(), 1);
        assert_eq!(usage.windows[0].used_fraction, 0.0);
        assert!(usage.is_cached);
        assert!(reading(&saved, 2000000).is_none());
        assert!(sanitized(&json!({"rate_limits":{}}), 0).is_none());
    }
}
