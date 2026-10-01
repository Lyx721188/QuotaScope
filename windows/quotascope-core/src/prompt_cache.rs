//! Claude's main-session prompt cache, from bounded transcript tails.
//! A hit renews the tier last written. Request time precedes streaming;
//! missing parents fall back to response time, never an invented earlier time.

use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

const TAIL_BYTES: u64 = 512 * 1024;
const HEAD_BYTES: u64 = 64 * 1024;
pub const STALE_AFTER_MS: i64 = 24 * 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheLapse {
    pub last_request_ms: i64,
    pub lifetime_ms: i64,
}

impl CacheLapse {
    pub fn expires_at(&self) -> i64 {
        self.last_request_ms.saturating_add(self.lifetime_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSession {
    pub id: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub lapse: CacheLapse,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheReading {
    pub live: Vec<CacheSession>,
    pub last_lapsed: Option<CacheLapse>,
}

impl CacheReading {
    pub fn alive(&self, now_ms: i64) -> Vec<&CacheSession> {
        let mut alive: Vec<_> = self
            .live
            .iter()
            .filter(|s| s.lapse.expires_at() > now_ms)
            .collect();
        alive.sort_by_key(|s| s.lapse.expires_at());
        alive
    }

    pub fn latest_lapsed(&self, now_ms: i64) -> Option<CacheLapse> {
        self.live
            .iter()
            .map(|s| s.lapse)
            .chain(self.last_lapsed)
            .filter(|s| {
                s.expires_at() <= now_ms && now_ms.saturating_sub(s.expires_at()) < STALE_AFTER_MS
            })
            .max_by_key(CacheLapse::expires_at)
    }
}

pub fn read(home: &Path, now_ms: i64) -> CacheReading {
    let Ok(projects) = fs::read_dir(home.join(".claude/projects")) else {
        return CacheReading::default();
    };
    let mut files = Vec::new();
    for project in projects.flatten().filter(|p| p.path().is_dir()) {
        let Ok(entries) = fs::read_dir(project.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let Some(modified) = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            else {
                continue;
            };
            let Ok(modified) = i64::try_from(modified.as_millis()) else {
                continue;
            };
            files.push((path, modified));
        }
    }
    let mut live: Vec<_> = files
        .iter()
        .filter(|(_, modified)| now_ms.saturating_sub(*modified) <= 3_660_000)
        .filter_map(|(path, _)| session(path))
        .filter(|session| session.lapse.expires_at() > now_ms)
        .collect();
    live.sort_by_key(|s| s.lapse.expires_at());
    let last_lapsed = if live.is_empty() {
        files
            .iter()
            .max_by_key(|(_, modified)| *modified)
            .and_then(|(path, _)| session(path))
            .map(|s| s.lapse)
    } else {
        None
    };
    CacheReading { live, last_lapsed }
}

fn session(path: &Path) -> Option<CacheSession> {
    let mut file = File::open(path).ok()?;
    let size = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(size.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    (&mut file).take(TAIL_BYTES).read_to_end(&mut tail).ok()?;
    let text = String::from_utf8_lossy(&tail);
    let lapse = lapse_from_tail(&text)?;
    let (mut title, mut cwd) = names(&text, size <= TAIL_BYTES);
    if (title.is_none() || cwd.is_none()) && size > TAIL_BYTES {
        file.seek(SeekFrom::Start(0)).ok()?;
        let mut head = Vec::new();
        (&mut file).take(HEAD_BYTES).read_to_end(&mut head).ok()?;
        let opening = names(&String::from_utf8_lossy(&head), true);
        title = title.or(opening.0);
        cwd = cwd.or(opening.1);
    }
    let project = cwd
        .as_deref()
        .and_then(|cwd| cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next())
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    Some(CacheSession {
        id: path.to_string_lossy().into_owned(),
        title,
        project,
        lapse,
    })
}

fn names(text: &str, opening: bool) -> (Option<String>, Option<String>) {
    let mut title = None;
    let mut prompt = None;
    let mut cwd = None;
    for entry in text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
    {
        if let Some(name) = entry
            .get("customTitle")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
        {
            title = Some(name.trim().chars().take(120).collect());
        }
        if let Some(path) = entry.get("cwd").and_then(Value::as_str) {
            cwd = Some(path.to_string());
        }
        if opening
            && prompt.is_none()
            && entry.get("type").and_then(Value::as_str) == Some("user")
            && entry.get("isSidechain").and_then(Value::as_bool) != Some(true)
        {
            if let Some(content) = entry.pointer("/message/content") {
                let text = content.as_str().or_else(|| {
                    content
                        .as_array()?
                        .iter()
                        .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))?
                        .get("text")?
                        .as_str()
                });
                prompt = text
                    .and_then(|s| s.lines().find(|l| !l.trim().is_empty()))
                    .map(|s| s.trim().chars().take(80).collect());
            }
        }
    }
    (title.or(prompt), cwd)
}

pub fn lapse_from_tail(text: &str) -> Option<CacheLapse> {
    let entries: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let mut by_id = HashMap::new();
    for entry in &entries {
        if let Some(id) = entry.get("uuid").and_then(Value::as_str) {
            by_id.insert(id, entry);
        }
    }
    let mut last_request = None;
    for entry in entries.iter().rev() {
        if entry.get("type").and_then(Value::as_str) != Some("assistant")
            || entry.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let Some(usage) = entry.pointer("/message/usage") else {
            continue;
        };
        if last_request.is_none() {
            let touched = ["cache_read_input_tokens", "cache_creation_input_tokens"]
                .iter()
                .any(|key| {
                    usage
                        .get(key)
                        .and_then(Value::as_i64)
                        .is_some_and(|n| n > 0)
                });
            if !touched {
                continue;
            }
            let Some(replied_at) = entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(crate::timeutil::parse_iso8601_ms)
            else {
                continue;
            };
            last_request = Some(request_time(entry, replied_at, &by_id).unwrap_or(replied_at));
        }
        let tier = usage.get("cache_creation");
        let lifetime_ms = if tier
            .and_then(|v| v.get("ephemeral_1h_input_tokens"))
            .and_then(Value::as_i64)
            .is_some_and(|n| n > 0)
        {
            3_600_000
        } else if tier
            .and_then(|v| v.get("ephemeral_5m_input_tokens"))
            .and_then(Value::as_i64)
            .is_some_and(|n| n > 0)
        {
            300_000
        } else {
            continue;
        };
        return Some(CacheLapse {
            last_request_ms: last_request?,
            lifetime_ms,
        });
    }
    None
}

fn request_time(reply: &Value, replied_at: i64, entries: &HashMap<&str, &Value>) -> Option<i64> {
    let mut parent = reply.get("parentUuid").and_then(Value::as_str);
    for _ in 0..64 {
        let entry = entries.get(parent?)?;
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            let at = entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(crate::timeutil::parse_iso8601_ms)?;
            let gap = replied_at.checked_sub(at)?;
            return (0..=1_200_000).contains(&gap).then_some(at);
        }
        parent = entry.get("parentUuid").and_then(Value::as_str);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stream(entries: &[Value]) -> String {
        entries
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn assistant(id: &str, parent: &str, at: &str, usage: Value) -> Value {
        json!({"uuid":id,"parentUuid":parent,"type":"assistant","timestamp":at,"message":{"usage":usage}})
    }

    #[test]
    fn cache_hits_renew_the_last_written_tier_from_request_time() {
        let entries = [
            json!({"uuid":"u1","type":"user","timestamp":"2026-10-01T08:00:00Z"}),
            assistant(
                "a1",
                "u1",
                "2026-10-01T08:00:20Z",
                json!({"cache_creation_input_tokens":100,"cache_creation":{"ephemeral_1h_input_tokens":100}}),
            ),
            json!({"uuid":"u2","type":"user","timestamp":"2026-10-01T08:20:00Z"}),
            assistant(
                "a2",
                "u2",
                "2026-10-01T08:20:50Z",
                json!({"cache_read_input_tokens":100}),
            ),
        ];
        let lapse = lapse_from_tail(&stream(&entries)).unwrap();
        assert_eq!(lapse.lifetime_ms, 3_600_000);
        assert_eq!(
            lapse.expires_at(),
            crate::timeutil::parse_iso8601_ms("2026-10-01T09:20:00Z").unwrap()
        );
    }

    #[test]
    fn unknown_tiers_sidechains_and_partial_tail_lines_are_not_assumed() {
        let mut child = assistant(
            "a",
            "missing",
            "2026-10-01T08:00:00Z",
            json!({"cache_creation_input_tokens":20,"cache_creation":{"ephemeral_5m_input_tokens":20}}),
        );
        child["isSidechain"] = json!(true);
        assert!(lapse_from_tail(&child.to_string()).is_none());
        let unknown = assistant(
            "b",
            "missing",
            "2026-10-01T08:00:00Z",
            json!({"cache_read_input_tokens":20}),
        );
        assert!(lapse_from_tail(&format!("partial {{\n{unknown}")).is_none());
    }

    #[test]
    fn implausible_or_cyclic_parents_fall_back_to_the_response_time() {
        let a = assistant(
            "a",
            "a",
            "2026-10-01T08:00:00Z",
            json!({"cache_read_input_tokens":1,"cache_creation":{"ephemeral_5m_input_tokens":1}}),
        );
        let lapse = lapse_from_tail(&a.to_string()).unwrap();
        assert_eq!(lapse.lifetime_ms, 300_000);
        assert_eq!(
            lapse.last_request_ms,
            crate::timeutil::parse_iso8601_ms("2026-10-01T08:00:00Z").unwrap()
        );
    }

    #[test]
    fn open_cards_drop_expired_sessions_without_waiting_for_another_scan() {
        let reading = CacheReading {
            live: vec![CacheSession {
                id: "session".into(),
                title: None,
                project: None,
                lapse: CacheLapse {
                    last_request_ms: 0,
                    lifetime_ms: 300_000,
                },
            }],
            last_lapsed: None,
        };
        assert_eq!(reading.alive(299_999).len(), 1);
        assert!(reading.alive(300_000).is_empty());
        assert!(reading.latest_lapsed(300_000).is_some());
        assert!(reading.latest_lapsed(300_000 + STALE_AFTER_MS).is_none());
    }

    #[test]
    fn disk_scan_reads_only_main_sessions_and_keeps_names_local() {
        let now = crate::timeutil::now_ms();
        let root = std::env::temp_dir().join(format!(
            "quotascope-cache-test-{}-{now}",
            std::process::id()
        ));
        let project = root.join(".claude/projects/project");
        fs::create_dir_all(project.join("session/subagents")).unwrap();
        let timestamp = crate::timeutil::iso8601_utc(now);
        let entries = [
            json!({"uuid":"u","type":"user","timestamp":timestamp,"cwd":"D:\\Projects\\Example",
            "message":{"content":"Opening prompt"}}),
            assistant(
                "a",
                "u",
                &timestamp,
                json!({"cache_creation_input_tokens":1,"cache_creation":{"ephemeral_5m_input_tokens":1}}),
            ),
            json!({"customTitle":"Renamed locally"}),
        ];
        fs::write(project.join("session.jsonl"), stream(&entries)).unwrap();
        fs::write(
            project.join("session/subagents/agent.jsonl"),
            stream(&entries),
        )
        .unwrap();
        let reading = read(&root, now);
        assert_eq!(reading.live.len(), 1);
        assert_eq!(reading.live[0].title.as_deref(), Some("Renamed locally"));
        assert_eq!(reading.live[0].project.as_deref(), Some("Example"));
        let resolved = root.canonicalize().unwrap();
        assert!(resolved.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        fs::remove_dir_all(resolved).unwrap();
    }
}
