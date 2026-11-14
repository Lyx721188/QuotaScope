//! Bounded lifecycle-tail reads. Recent writes alone never mean working.
use crate::model::Provider;
use serde_json::Value;
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub fn working(provider: Provider, now: i64, home: &Path) -> bool {
    let root = match provider {
        Provider::ClaudeCode => home.join(".claude/projects"),
        Provider::Codex => home.join(".codex/sessions"),
        Provider::CommandCode => home.join(".commandcode/projects"),
        _ => return false,
    };
    fn walk(root: &Path, depth: usize, visited: &mut usize, out: &mut Vec<(i64, PathBuf)>) {
        if depth > 8 || *visited >= 10000 || !crate::scan::checkpoint() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            *visited += 1;
            if *visited > 10000 || !crate::scan::checkpoint() {
                break;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                walk(&path, depth + 1, visited, out);
            } else if kind.is_file()
                && path.extension().is_some_and(|s| s == "jsonl")
                && !path.to_string_lossy().ends_with(".checkpoints.jsonl")
            {
                let Some(at) = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
                else {
                    continue;
                };
                out.push((at, path));
                if out.len() > 256 {
                    out.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
                    out.truncate(32);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(&root, 0, &mut 0, &mut files);
    files.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    files.into_iter().take(8).any(|(modified, path)| {
        if modified < now - 300000 || modified > now + 5000 {
            return false;
        }
        let Ok(mut file) = std::fs::File::open(path) else {
            return false;
        };
        let Ok(len) = file.metadata().map(|m| m.len()) else {
            return false;
        };
        let start = len.saturating_sub(256 * 1024);
        if file.seek(SeekFrom::Start(start)).is_err() {
            return false;
        }
        let mut bytes = Vec::new();
        if file.take(256 * 1024).read_to_end(&mut bytes).is_err() {
            return false;
        }
        if start > 0 {
            if let Some(index) = bytes.iter().position(|b| *b == b'\n') {
                bytes.drain(..=index);
            }
        }
        verdict(provider, &bytes, modified, now)
    })
}
pub fn verdict(provider: Provider, bytes: &[u8], modified: i64, now: i64) -> bool {
    for line in bytes.rsplit(|b| *b == b'\n').filter(|b| !b.is_empty()) {
        let Ok(v) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let at = v
            .pointer("/message/meta/createdAt")
            .and_then(Value::as_i64)
            .or_else(|| {
                v.get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(crate::timeutil::parse_iso8601_ms)
            })
            .unwrap_or(modified);
        let wait = match provider {
            Provider::CommandCode if v.get("type").and_then(Value::as_str) == Some("message") => {
                match v.pointer("/message/role").and_then(Value::as_str) {
                    Some("user") => 90_000,
                    Some("assistant")
                        if v.pointer("/message/content")
                            .and_then(Value::as_array)
                            .is_some_and(|parts| {
                                parts.iter().any(|p| {
                                    p.get("type").and_then(Value::as_str) == Some("tool_use")
                                })
                            }) =>
                    {
                        300_000
                    }
                    Some("assistant") => return false,
                    _ => continue,
                }
            }
            Provider::Codex => match v.pointer("/payload/type").and_then(Value::as_str) {
                Some("task_complete" | "turn_aborted") => return false,
                Some(
                    "task_started"
                    | "function_call_output"
                    | "custom_tool_call_output"
                    | "tool_search_output",
                ) => 90000,
                Some(
                    "function_call" | "custom_tool_call" | "local_shell_call" | "web_search_call"
                    | "tool_search_call",
                ) => 300000,
                _ => continue,
            },
            Provider::ClaudeCode => match v.get("type").and_then(Value::as_str) {
                Some("user") if v.get("isMeta").and_then(Value::as_bool) == Some(true) => continue,
                Some("user")
                    if v.pointer("/message/content").is_some_and(|c| {
                        c.to_string().contains("[Request interrupted by user")
                    }) =>
                {
                    return false
                }
                Some("user") => 90000,
                Some("assistant") => {
                    match v.pointer("/message/stop_reason").and_then(Value::as_str) {
                        Some("tool_use") => 300000,
                        None => 90000,
                        _ => return false,
                    }
                }
                _ => continue,
            },
            _ => continue,
        };
        return at <= now + 5000 && now.saturating_sub(at) <= wait;
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_lifecycle_finishes_immediately_and_tools_have_their_own_deadline() {
        let user = br#"{"type":"message","message":{"role":"user","meta":{"createdAt":1000000}}}"#;
        assert!(verdict(Provider::CommandCode, user, 1000000, 1080000));
        assert!(!verdict(Provider::CommandCode, user, 1000000, 1100000));
        let tool =
            br#"{"type":"message","message":{"role":"assistant","content":[{"type":"tool_use"}]}}"#;
        assert!(verdict(Provider::CommandCode, tool, 1000000, 1200000));
        let done=br#"{"type":"message","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}"#;
        assert!(!verdict(Provider::CommandCode, done, 1000000, 1000001));
    }
}
