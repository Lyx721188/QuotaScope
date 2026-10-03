//! Read-only OpenCode 1/2 SQLite token counters. Copied v2 message IDs win
//! over their old-table copies; reasoning tokens are part of output here.
use crate::ledger::{TokenTally, UsageLedger};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub fn path(client: &str) -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join(client).join("opencode.db"));
    local.filter(|p| p.is_file()).unwrap_or_else(|| {
        crate::home_dir()
            .join(".local/share")
            .join(client)
            .join("opencode.db")
    })
}

pub fn read(path: &Path) -> UsageLedger {
    let Ok(db) = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return UsageLedger::empty();
    };
    let _ = db.busy_timeout(std::time::Duration::from_millis(500));
    let mut seen = HashSet::new();
    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut partial = false;
    for (table, role_column) in [("session_message", true), ("message", false)] {
        let columns: HashSet<String> = db
            .prepare(&format!("PRAGMA table_info({table})"))
            .ok()
            .and_then(|mut q| {
                q.query_map([], |r| r.get::<_, String>(1))
                    .ok()
                    .map(|r| r.flatten().collect())
            })
            .unwrap_or_default();
        if columns.is_empty() {
            continue;
        }
        if !columns.contains("data") || (role_column && !columns.contains("type")) {
            partial = true;
            continue;
        }
        let id = if columns.contains("id") { "id" } else { "NULL" };
        let filter = if role_column && columns.contains("type") {
            " WHERE type = 'assistant'"
        } else {
            ""
        };
        let Ok(mut query) = db.prepare(&format!("SELECT {id}, data FROM {table}{filter}")) else {
            partial = true;
            continue;
        };
        let Ok(rows) = query.query_map([], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
        }) else {
            partial = true;
            continue;
        };
        for row in rows {
            if !crate::scan::checkpoint() {
                partial = true;
                break;
            }
            let Ok((id, data)) = row else {
                partial = true;
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(&data) else {
                partial = true;
                continue;
            };
            if !role_column && value["role"].as_str() != Some("assistant") {
                continue;
            }
            let Some((at, model, tally)) = counters(&value) else {
                continue;
            };
            if id.is_some_and(|id| !seen.insert(id)) {
                continue;
            }
            *buckets
                .entry(crate::ledger::slot_key_from_ms(at))
                .or_default()
                .entry(model)
                .or_default() += tally;
        }
    }
    let mut ledger = crate::ledger::priced(&buckets, &crate::model_prices::prices(), None);
    ledger.has_partial_records = partial;
    ledger
}

fn counters(value: &Value) -> Option<(i64, String, TokenTally)> {
    let at = value["time"]["created"].as_i64()?;
    chrono::DateTime::from_timestamp_millis(at)?;
    let model = value["modelID"]
        .as_str()
        .or_else(|| value["model"]["id"].as_str())?
        .to_string();
    let tokens = &value["tokens"];
    let n = |v: &Value| v.as_i64().unwrap_or(0).max(0);
    let tally = TokenTally {
        input: n(&tokens["input"]),
        output: n(&tokens["output"]).saturating_add(n(&tokens["reasoning"])),
        cache_read: n(&tokens["cache"]["read"]),
        cache_write: n(&tokens["cache"]["write"]),
    };
    (tally.total() > 0).then_some((at, model, tally))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v2_copy_wins_and_reasoning_is_counted_once() {
        let path = std::env::temp_dir().join(format!("qs-opencode-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE message (id TEXT, data TEXT); CREATE TABLE session_message (id TEXT, type TEXT, data TEXT);").unwrap();
        let old = r#"{"role":"assistant","modelID":"unknown","time":{"created":1790850000000},"tokens":{"input":10,"output":2}}"#;
        let new = r#"{"model":{"id":"unknown"},"time":{"created":1790850000000},"tokens":{"input":10,"output":3,"reasoning":4,"cache":{"read":5}}}"#;
        db.execute(
            "INSERT INTO message VALUES ('copy', ?1), ('old-only', ?1)",
            [old],
        )
        .unwrap();
        db.execute(
            "INSERT INTO session_message VALUES ('copy', 'assistant', ?1), ('user', 'user', ?1)",
            [new],
        )
        .unwrap();
        drop(db);
        let ledger = read(&path);
        assert_eq!(ledger.days[0].tokens, 34);
        assert_eq!(ledger.days[0].tally.output, 9);
        assert_eq!(ledger.days[0].unpriced_tokens, 34);
        assert!(!ledger.has_partial_records);
        std::fs::remove_file(path).unwrap();
    }
}
