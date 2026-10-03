//! Additional upstream catalogue sources. Native formats are dispatched
//! explicitly. Every catalogue entry also accepts a documented local JSONL
//! export, rather than guessing counters from arbitrary objects.
use crate::ledger::{TokenTally, UsageLedger};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const CATALOG: &[(&str, &str, &str)] = &[
    ("grok", "Grok Build", ".grok/sessions"),
    ("kimi", "Kimi CLI", ".kimi/sessions"),
    ("devin-cli", "Devin", ".local/share/devin/cli/sessions.db"),
    ("roocode", "Roo Code", ""),
    ("kilocode", "Kilo Code", ""),
    ("cline", "Cline", ""),
    ("codebuddy", "CodeBuddy", ".codebuddy"),
    ("workbuddy", "WorkBuddy", ".workbuddy"),
    ("cherrystudio", "Cherry Studio", ""),
    ("commandcode", "Command Code", ".commandcode"),
    ("opencodereview", "OpenCodeReview", ".opencodereview"),
    ("zcode", "ZCode", ".zcode"),
    ("hermes", "Hermes", ".hermes"),
    ("goose", "Goose", ".local/share/goose/sessions"),
    ("zed", "Zed", ""),
    ("kiro", "Kiro", ".kiro"),
    ("crush", "Crush", ".local/share/crush"),
    ("unsloth", "Unsloth", ".unsloth"),
    (
        "antigravity-cli",
        "Antigravity CLI",
        ".gemini/antigravity/conversations",
    ),
    ("micode", "MiMo Code", ".micode"),
    ("devin-desktop", "Devin Desktop", ""),
    ("freebuff", "Freebuff", ".freebuff"),
    ("dsh", "DeepSeek Harness", ".dsh/sessions"),
    ("cursor", "Cursor (export)", ".config/tokscale/cursor-cache"),
    (
        "antigravity",
        "Antigravity (export)",
        ".config/tokscale/antigravity-cache/sessions",
    ),
    (
        "trae",
        "Trae (export)",
        ".config/tokscale/trae-cache/sessions",
    ),
    ("warp", "Warp (export)", ".config/tokscale/warp-cache"),
    ("hindsight", "Hindsight (export)", ".hindsight/usage"),
    (
        "mcode",
        "MiniMax Code (export)",
        ".config/tokscale/headless/mcode",
    ),
    ("copilot", "GitHub Copilot", ".copilot/session-state"),
];

pub fn native_supported(id: &str) -> bool {
    matches!(
        id,
        "grok"
            | "kimi"
            | "devin-cli"
            | "roocode"
            | "kilocode"
            | "cline"
            | "dsh"
            | "antigravity"
            | "hindsight"
            | "cursor"
    )
}

pub fn roots(id: &str, relative: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if !relative.is_empty() {
        paths.push(crate::home_dir().join(relative));
    }
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::home_dir().join("AppData/Roaming"));
    if let Some(extension) = match id {
        "roocode" => Some("rooveterinaryinc.roo-cline"),
        "kilocode" => Some("kilocode.kilo-code"),
        "cline" => Some("saoudrizwan.claude-dev"),
        _ => None,
    } {
        for editor in ["Code", "Code - Insiders", "VSCodium"] {
            paths.push(
                roaming
                    .join(editor)
                    .join("User/globalStorage")
                    .join(extension)
                    .join("tasks"),
            );
        }
        for server in [".vscode-server", ".vscode-server-insiders"] {
            paths.push(
                crate::home_dir()
                    .join(server)
                    .join("data/User/globalStorage")
                    .join(extension)
                    .join("tasks"),
            );
        }
    }
    match id {
        "cherrystudio" => paths.push(roaming.join("CherryStudio")),
        "zed" => paths.push(
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join("Zed"),
        ),
        "devin-desktop" => paths.push(roaming.join("Devin/User/acp-events")),
        "devin-cli" => paths.push(roaming.join("devin/cli/sessions.db")),
        _ => {}
    }
    paths.push(crate::data_dir().join("UsageImports").join(id));
    paths
}

#[derive(Default)]
struct Reader {
    buckets: BTreeMap<String, BTreeMap<String, TokenTally>>,
    ids: HashMap<String, (String, String, TokenTally)>,
    partial: bool,
}
impl Reader {
    fn add(&mut self, at: i64, model: &str, tally: TokenTally, id: Option<String>) {
        if tally.total() <= 0 || chrono::DateTime::from_timestamp_millis(at).is_none() || at <= 0 {
            return;
        }
        let key = crate::ledger::slot_key_from_ms(at);
        if let Some(id) = id {
            if let Some((old_slot, old_model, old)) =
                self.ids.insert(id, (key.clone(), model.to_string(), tally))
            {
                if let Some(previous) = self
                    .buckets
                    .get_mut(&old_slot)
                    .and_then(|b| b.get_mut(&old_model))
                {
                    previous.input -= old.input;
                    previous.output -= old.output;
                    previous.cache_write -= old.cache_write;
                    previous.cache_read -= old.cache_read;
                }
            }
        }
        *self
            .buckets
            .entry(key)
            .or_default()
            .entry(model.to_string())
            .or_default() += tally;
    }
}

pub fn read(id: &str, paths: &[PathBuf]) -> UsageLedger {
    let mut reader = Reader::default();
    let mut files = Vec::new();
    let mut seen_paths = HashSet::new();
    for path in paths {
        if !native_supported(id) && !path.components().any(|p| p.as_os_str() == "UsageImports") {
            continue;
        }
        collect(path, &mut files, &mut seen_paths);
    }
    for file in files {
        if !crate::scan::checkpoint() {
            reader.partial = true;
            break;
        }
        let import = file.components().any(|p| p.as_os_str() == "UsageImports");
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if id == "devin-cli" && name == "sessions.db" && !import {
            devin(&file, &mut reader);
            continue;
        }
        let accepted = if import {
            matches!(
                file.extension().and_then(|v| v.to_str()),
                Some("jsonl" | "json")
            )
        } else {
            match id {
                "grok" => name == "updates.jsonl",
                "kimi" => name == "wire.jsonl",
                "roocode" | "kilocode" | "cline" => name == "ui_messages.json",
                "dsh" => {
                    name.starts_with("session")
                        && (name.ends_with(".jsonl") || name.ends_with(".zstd"))
                }
                "antigravity" | "hindsight" | "cursor" => {
                    file.extension().is_some_and(|s| s == "jsonl")
                }
                _ => false,
            }
        };
        if !accepted {
            continue;
        }
        if file.metadata().is_ok_and(|m| m.len() > 128 * 1024 * 1024) {
            reader.partial = true;
            continue;
        }
        let Ok(mut input) = std::fs::File::open(&file) else {
            reader.partial = true;
            continue;
        };
        let mut magic = [0_u8; 4];
        let _ = input.read_exact(&mut magic);
        if magic == [0x28, 0xb5, 0x2f, 0xfd] {
            reader.partial = true;
            continue;
        }
        if file.extension().is_some_and(|s| s == "json") {
            if let Ok(value) = std::fs::read(&file).and_then(|bytes| {
                serde_json::from_slice::<Value>(&bytes).map_err(std::io::Error::other)
            }) {
                if let Some(rows) = value.as_array() {
                    for row in rows {
                        parse(id, row, import, &file, &mut reader, &mut (0, None));
                    }
                } else if import {
                    parse(id, &value, true, &file, &mut reader, &mut (0, None));
                }
            } else {
                reader.partial = true;
            }
        } else {
            let Ok(input) = std::fs::File::open(&file) else {
                continue;
            };
            let hash = std::collections::hash_map::DefaultHasher::new();
            let mut lines = crate::scan::LineReader::new(input, hash, 0);
            let mut context = (0, None);
            for line in lines.by_ref() {
                if let Ok(value) = serde_json::from_str::<Value>(&line) {
                    parse(id, &value, import, &file, &mut reader, &mut context);
                } else {
                    reader.partial = true;
                }
            }
            if lines.finish().is_err() {
                reader.partial = true;
            }
        }
    }
    if !native_supported(id)
        && paths
            .iter()
            .any(|p| p.exists() && !p.components().any(|c| c.as_os_str() == "UsageImports"))
    {
        reader.partial = true;
    }
    let mut ledger = crate::ledger::priced(&reader.buckets, &crate::model_prices::prices(), None);
    ledger.has_partial_records = reader.partial;
    ledger
}

fn collect(path: &Path, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    if !crate::scan::checkpoint() || !seen.insert(path.to_path_buf()) {
        return;
    }
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    if meta.is_file() {
        out.push(path.to_path_buf());
    } else if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_name() != "binaries" {
                collect(&entry.path(), out, seen);
            }
        }
    }
}

fn count(value: &Value) -> i64 {
    crate::http::number(value)
        .filter(|n| n.is_finite() && *n >= 0.0 && *n <= 1e12)
        .map(|n| n as i64)
        .unwrap_or(0)
}
fn timestamp(value: &Value, numeric_ms: bool) -> Option<i64> {
    value
        .as_str()
        .and_then(crate::timeutil::parse_iso8601_ms)
        .or_else(|| {
            crate::http::number(value)
                .filter(|n| n.is_finite() && *n > 0.0)
                .map(|n| {
                    if numeric_ms {
                        n as i64
                    } else {
                        (n * 1000.0) as i64
                    }
                })
        })
}
fn anthropic(usage: &Value) -> TokenTally {
    TokenTally {
        input: count(&usage["input_tokens"]),
        output: count(&usage["output_tokens"]),
        cache_write: count(&usage["cache_creation_input_tokens"]),
        cache_read: count(&usage["cache_read_input_tokens"]),
    }
}
fn camel(usage: &Value) -> TokenTally {
    TokenTally {
        input: count(&usage["inputTokens"]),
        output: count(&usage["outputTokens"]),
        cache_write: count(&usage["cacheWriteTokens"]),
        cache_read: count(&usage["cacheReadTokens"]),
    }
}

fn parse(
    id: &str,
    row: &Value,
    imported: bool,
    file: &Path,
    reader: &mut Reader,
    context: &mut (i64, Option<String>),
) {
    if imported {
        if row["schema"].as_str() != Some("quotascope.usage.v1")
            || row["source"].as_str() != Some(id)
        {
            reader.partial = true;
            return;
        }
        if let (Some(at), Some(model), Some(record)) = (
            timestamp(&row["timestamp"], true),
            row["model"].as_str(),
            row["id"].as_str(),
        ) {
            reader.add(
                at,
                model,
                camel(&row["usage"]),
                Some(format!("export:{record}")),
            );
        } else {
            reader.partial = true;
        }
        return;
    }
    let file_id = file
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    match id {
        "grok" => {
            let update = &row["params"]["update"];
            if update["sessionUpdate"].as_str() != Some("turn_completed") {
                return;
            }
            let Some(at) = timestamp(&row["timestamp"], false) else {
                return;
            };
            let usage = &update["usage"];
            let records = usage["modelUsage"]
                .as_object()
                .cloned()
                .unwrap_or_else(|| serde_json::Map::from_iter([("grok".into(), usage.clone())]));
            for (model, usage) in records {
                reader.add(
                    at,
                    &model,
                    TokenTally {
                        input: count(&usage["inputTokens"]),
                        output: count(&usage["outputTokens"]) + count(&usage["reasoningTokens"]),
                        cache_write: count(&usage["cacheCreationTokens"]),
                        cache_read: count(&usage["cachedReadTokens"]),
                    },
                    Some(format!("{file_id}:{at}:{model}")),
                );
            }
        }
        "kimi" => {
            let usage = &row["message"]["payload"]["token_usage"];
            let Some(at) = timestamp(&row["timestamp"], count(&row["timestamp"]) > 10_000_000_000)
            else {
                return;
            };
            reader.add(
                at,
                "kimi (unnamed)",
                TokenTally {
                    input: count(&usage["input_other"]),
                    output: count(&usage["output"]),
                    cache_write: count(&usage["input_cache_creation"]),
                    cache_read: count(&usage["input_cache_read"]),
                },
                Some(format!("{file_id}:{at}")),
            );
        }
        "roocode" | "kilocode" | "cline" => {
            if row["type"].as_str() != Some("say") || row["say"].as_str() != Some("api_req_started")
            {
                return;
            }
            if let (Some(at), Some(text), Some(model)) = (
                timestamp(&row["ts"], true),
                row["text"].as_str(),
                row["modelInfo"]["modelId"].as_str(),
            ) {
                if let Ok(usage) = serde_json::from_str::<Value>(text) {
                    reader.add(
                        at,
                        model,
                        TokenTally {
                            input: count(&usage["tokensIn"]),
                            output: count(&usage["tokensOut"]),
                            cache_write: count(&usage["cacheWrites"]),
                            cache_read: count(&usage["cacheReads"]),
                        },
                        Some(format!("{file_id}:{at}")),
                    );
                }
            }
        }
        "dsh" => {
            let kind = row["type"].as_str().unwrap_or("");
            if kind == "session" {
                context.0 = count(&row["seedLength"]);
            }
            if kind == "request/header" {
                if let Some(model) = row["data"]["header"]["config"]["model"].as_str() {
                    context.1 = Some(model.into());
                }
            }
            if !matches!(kind, "assistant/message" | "compaction/summary")
                || count(&row["seq"]) < context.0
            {
                return;
            }
            let data = &row["data"];
            let source = &data["message"]["source"];
            let model = source["replayState"]["response"]["responseModel"]
                .as_str()
                .or_else(|| source["model"].as_str())
                .or(context.1.as_deref());
            if let (Some(at), Some(model)) = (timestamp(&row["time"], true), model) {
                let tally = camel(&data["usage"]);
                let identity = data["message"]["id"]
                    .as_str()
                    .or_else(|| data["compactionId"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("seq:{}", count(&row["seq"])));
                reader.add(
                    at,
                    model,
                    tally,
                    Some(format!(
                        "dsh:{identity}:{at}:{model}:{}",
                        serde_json::to_string(&tally).unwrap_or_default()
                    )),
                );
            }
        }
        "antigravity" => {
            if let (Some(at), Some(model)) =
                (timestamp(&row["timestamp"], true), row["modelId"].as_str())
            {
                let tally = TokenTally {
                    input: count(&row["input"]),
                    output: count(&row["output"]),
                    cache_write: count(&row["cacheWrite"]),
                    cache_read: count(&row["cacheRead"]),
                };
                reader.add(at, model, tally, Some(format!("{model}:{at}")));
            }
        }
        "cursor" => {
            if let (Some(at), Some(model)) =
                (timestamp(&row["timestamp"], true), row["model"].as_str())
            {
                reader.add(
                    at,
                    model,
                    camel(&row["usage"]),
                    Some(format!("{model}:{at}")),
                );
            }
        }
        "hindsight" => {
            if let (Some(at), Some(model)) =
                (timestamp(&row["started_at"], false), row["model"].as_str())
            {
                let read = count(&row["cached_tokens"]);
                reader.add(
                    at,
                    model,
                    TokenTally {
                        input: (count(&row["input_tokens"]) - read).max(0),
                        output: count(&row["output_tokens"]),
                        cache_read: read,
                        cache_write: 0,
                    },
                    row["id"].as_str().map(|id| format!("hindsight:{id}")),
                );
            }
        }
        _ => {}
    }
}

fn devin(path: &Path, reader: &mut Reader) {
    let Ok(db) =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        reader.partial = true;
        return;
    };
    let Ok(mut q) = db.prepare("SELECT session_id, chat_message, created_at FROM message_nodes")
    else {
        reader.partial = true;
        return;
    };
    let Ok(rows) = q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    }) else {
        reader.partial = true;
        return;
    };
    for row in rows {
        if !crate::scan::checkpoint() {
            reader.partial = true;
            break;
        }
        let Ok((session, message, time)) = row else {
            reader.partial = true;
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(&message) else {
            reader.partial = true;
            continue;
        };
        let metadata = &value["metadata"];
        let usage = &metadata["metrics"];
        let mut tally = anthropic(usage);
        tally.cache_read = count(&usage["cache_read_tokens"]);
        tally.cache_write = count(&usage["cache_creation_tokens"]);
        if let Some(at) = timestamp(&Value::from(time), time > 10_000_000_000) {
            reader.add(
                at,
                metadata["generation_model"].as_str().unwrap_or("devin"),
                tally,
                Some(format!("devin:{session}:{time}:{}", {
                    use sha2::{Digest, Sha256};
                    format!("{:x}", Sha256::digest(message.as_bytes()))
                })),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn dsh_fork_seed_and_reasoning_subset_do_not_double_count() {
        let mut reader = Reader::default();
        let mut context = (0, None);
        let file = Path::new("session.jsonl");
        parse(
            "dsh",
            &json!({"type":"session","seedLength":3}),
            false,
            file,
            &mut reader,
            &mut context,
        );
        parse(
            "dsh",
            &json!({"type":"request/header","data":{"header":{"config":{"model":"test"}}}}),
            false,
            file,
            &mut reader,
            &mut context,
        );
        let mut row = json!({"type":"assistant/message","seq":2,"time":1790850000000_i64,"data":{"message":{"id":"one"},"usage":{"inputTokens":10,"outputTokens":8,"reasoningTokens":5}}});
        parse("dsh", &row, false, file, &mut reader, &mut context);
        assert!(reader.buckets.is_empty());
        row["seq"] = json!(3);
        parse("dsh", &row, false, file, &mut reader, &mut context);
        parse("dsh", &row, false, file, &mut reader, &mut context);
        assert_eq!(reader.buckets.values().next().unwrap()["test"].total(), 18);
    }
    #[test]
    fn imports_require_source_schema_and_stable_identity() {
        let mut reader = Reader::default();
        let file = Path::new("export.jsonl");
        parse(
            "zed",
            &json!({"tokens":999}),
            true,
            file,
            &mut reader,
            &mut (0, None),
        );
        assert!(reader.partial && reader.buckets.is_empty());
        let row = json!({"schema":"quotascope.usage.v1","source":"zed","id":"request","timestamp":1790850000000_i64,"model":"unknown","usage":{"inputTokens":10,"outputTokens":7}});
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        parse("zed", &row, true, file, &mut reader, &mut (0, None));
        assert_eq!(
            reader.buckets.values().next().unwrap()["unknown"].total(),
            17
        );
    }
}
