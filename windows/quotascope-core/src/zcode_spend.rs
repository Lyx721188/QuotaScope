//! ZCode's normalized model-attempt counters, without reading conversation tables.
//! Input includes cache and output includes reasoning. Database defaults alone
//! do not prove that a provider reported a count.
use crate::ledger::{TokenTally, UsageLedger};
use rusqlite::{limits::Limit, Connection, OpenFlags};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

const MAX_COUNTER: i64 = 1_000_000_000_000;
const MAX_ROWS: usize = 100_000;
const MAX_BUCKET_BYTES: usize = 8 * 1024 * 1024;
const MAX_DATABASE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_WAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_USAGE_BYTES: usize = 4096;
const MAX_SECONDS: u64 = 4;

pub type Buckets = BTreeMap<String, BTreeMap<String, TokenTally>>;

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub buckets: Buckets,
    pub rows_read: usize,
    pub rows_skipped: usize,
    pub partial: bool,
    complete: bool,
}

#[derive(Debug)]
struct Record {
    id: String,
    logical: String,
    attempt: i64,
    model: String,
    status: String,
    at: i64,
    input: i64,
    output: i64,
    reasoning: i64,
    write: i64,
    read: i64,
    total: i64,
    provider_total: Option<i64>,
    raw: Option<String>,
}

struct Parsed {
    at: i64,
    model: String,
    tally: TokenTally,
    partial: bool,
}

fn parse(record: Record) -> Option<Parsed> {
    if record.id.trim().is_empty()
        || record.id.len() > 1024
        || record.logical.trim().is_empty()
        || record.logical.len() > 1024
        || !(0..=100_000).contains(&record.attempt)
        || !matches!(record.status.as_str(), "completed" | "error" | "cancelled")
        || record.model.len() > 256
        || record.at <= 0
        || chrono::DateTime::from_timestamp_millis(record.at).is_none()
    {
        return None;
    }
    for count in [
        record.input,
        record.output,
        record.reasoning,
        record.write,
        record.read,
        record.total,
    ] {
        if !(0..=MAX_COUNTER).contains(&count) {
            return None;
        }
    }
    let raw = record.raw?;
    if raw.len() > MAX_USAGE_BYTES {
        return None;
    }
    let raw: Value = serde_json::from_str(&raw).ok()?;
    let raw = raw.as_object()?;
    let required = |name: &str, stored: i64| raw.get(name)?.as_i64().filter(|n| *n == stored);
    required("inputTokens", record.input)?;
    required("outputTokens", record.output)?;
    for (name, stored) in [
        ("reasoningTokens", record.reasoning),
        ("cacheWriteTokens", record.write),
        ("cacheReadTokens", record.read),
    ] {
        match raw.get(name) {
            Some(value) if value.as_i64() == Some(stored) => {}
            None if stored == 0 => {}
            _ => return None,
        }
    }
    let total = record.input.checked_add(record.output)?;
    if total != record.total || record.reasoning > record.output {
        return None;
    }
    match (record.provider_total, raw.get("totalTokens")) {
        (Some(stored), Some(value)) if stored == total && value.as_i64() == Some(stored) => {}
        (None, None) => {}
        _ => return None,
    }
    let input = record
        .input
        .checked_sub(record.read)?
        .checked_sub(record.write)?;
    if input < 0 {
        return None;
    }
    let model = record.model.trim();
    Some(Parsed {
        at: record.at,
        model: if model.is_empty() { "unknown" } else { model }.to_owned(),
        tally: TokenTally {
            input,
            output: record.output,
            cache_write: record.write,
            cache_read: record.read,
        },
        partial: model.is_empty(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}

impl FileStamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return None;
        }
        Some(Self {
            size: meta.len(),
            modified: meta.modified().ok()?,
            created: meta.created().ok(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    database: FileStamp,
    wal: Option<FileStamp>,
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let database = FileStamp::of(path)?;
        let wal_path = path.with_file_name(format!("{}-wal", path.file_name()?.to_str()?));
        let wal = if wal_path.try_exists().ok()? {
            Some(FileStamp::of(&wal_path)?)
        } else {
            None
        };
        if database.size > MAX_DATABASE_BYTES || wal.is_some_and(|m| m.size > MAX_WAL_BYTES) {
            return None;
        }
        Some(Self { database, wal })
    }
}

/// A bounded read-only SELECT provides a SQLite snapshot, including committed
/// WAL frames. It does not query messages, parts, errors, or provider metadata.
pub fn read_counts(path: &Path) -> Report {
    read_limited(path, MAX_ROWS)
}

pub fn read_with_prices(
    path: &Path,
    prices: &BTreeMap<String, crate::model_prices::ModelPrice>,
) -> UsageLedger {
    let report = read_counts(path);
    let mut ledger = crate::ledger::priced(&report.buckets, prices, None);
    ledger.has_partial_records = report.partial;
    ledger
}

fn read_limited(path: &Path, maximum_rows: usize) -> Report {
    let mut report = Report::default();
    let Some(before) = Stamp::of(path) else {
        report.partial = path.try_exists().unwrap_or(true);
        return report;
    };
    if !crate::scan::checkpoint() {
        report.partial = true;
        return report;
    }
    let deadline = Instant::now() + Duration::from_secs(MAX_SECONDS);
    let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        report.partial = true;
        return report;
    };
    let setup = db.busy_timeout(Duration::from_millis(100)).and_then(|_| {
        db.execute_batch(
            "PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA cache_size=-1024; \
             PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY;",
        )
    });
    if setup.is_err() || db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 64 * 1024).is_err() {
        report.partial = true;
        return report;
    }
    db.progress_handler(
        1000,
        Some(move || !crate::scan::checkpoint() || Instant::now() >= deadline),
    );
    if db
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name='model_usage'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .as_deref()
        != Some("table")
    {
        report.partial = true;
        return report;
    }
    let Ok(mut query) = db.prepare(
        "SELECT id, logical_request_id, attempt_index, model_id, status, started_at, \
         input_tokens, output_tokens, reasoning_tokens, cache_creation_input_tokens, \
         cache_read_input_tokens, computed_total_tokens, provider_total_tokens, raw_usage_json \
         FROM model_usage ORDER BY started_at DESC LIMIT ?1",
    ) else {
        report.partial = true;
        return report;
    };
    let Ok(mut rows) = query.query([maximum_rows.saturating_add(1) as i64]) else {
        report.partial = true;
        return report;
    };
    let mut bytes = 0;
    loop {
        if !crate::scan::checkpoint() || Instant::now() >= deadline {
            report.partial = true;
            break;
        }
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) => {
                report.complete = true;
                break;
            }
            Err(_) => {
                report.partial = true;
                break;
            }
        };
        if report.rows_read >= maximum_rows {
            report.partial = true;
            break;
        }
        report.rows_read += 1;
        let record = (|| -> rusqlite::Result<Record> {
            Ok(Record {
                id: row.get(0)?,
                logical: row.get(1)?,
                attempt: row.get(2)?,
                model: row.get(3)?,
                status: row.get(4)?,
                at: row.get(5)?,
                input: row.get(6)?,
                output: row.get(7)?,
                reasoning: row.get(8)?,
                write: row.get(9)?,
                read: row.get(10)?,
                total: row.get(11)?,
                provider_total: row.get(12)?,
                raw: row.get(13)?,
            })
        })();
        let Some(record) = record.ok().and_then(parse) else {
            report.rows_skipped += 1;
            report.partial = true;
            continue;
        };
        report.partial |= record.partial;
        if record.tally.total() == 0 {
            continue;
        }
        let slot = crate::ledger::slot_key_from_ms(record.at);
        let present = report
            .buckets
            .get(&slot)
            .is_some_and(|m| m.contains_key(&record.model));
        if !present {
            bytes += 256 + slot.len() + record.model.len();
            if bytes > MAX_BUCKET_BYTES {
                report.partial = true;
                break;
            }
        }
        // MAX_ROWS and MAX_COUNTER keep every aggregate below i64::MAX.
        *report
            .buckets
            .entry(slot)
            .or_default()
            .entry(record.model)
            .or_default() += record.tally;
    }
    if !crate::scan::checkpoint() || Stamp::of(path).as_ref() != Some(&before) {
        report.partial = true;
        report.complete = false;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> Record {
        Record {
            id: "request-1-attempt-0".into(),
            logical: "request-1".into(),
            attempt: 0,
            model: "fixture".into(),
            status: "completed".into(),
            at: 1790850000000,
            input: 100,
            output: 20,
            reasoning: 5,
            write: 10,
            read: 30,
            total: 120,
            provider_total: Some(120),
            raw: Some(
                json!({"inputTokens":100,"outputTokens":20,"reasoningTokens":5,
                "cacheWriteTokens":10,"cacheReadTokens":30,"totalTokens":120})
                .to_string(),
            ),
        }
    }

    #[test]
    fn normalized_cache_and_reasoning_are_disjoint_without_rebilling() {
        let parsed = parse(record()).unwrap();
        assert_eq!(
            parsed.tally,
            TokenTally {
                input: 60,
                output: 20,
                cache_write: 10,
                cache_read: 30
            }
        );
        assert_eq!(parsed.tally.total(), 120);
        assert!(!parsed.partial);
    }

    #[test]
    fn failed_cancelled_and_retry_attempts_keep_reported_counts() {
        for status in ["completed", "error", "cancelled"] {
            let mut row = record();
            row.status = status.into();
            row.attempt = 1;
            assert_eq!(parse(row).unwrap().tally.total(), 120);
        }
    }

    #[test]
    fn database_defaults_are_not_reported_zero() {
        let mut row = record();
        row.raw = None;
        assert!(parse(row).is_none());
        let mut row = record();
        row.input = 0;
        row.output = 0;
        row.reasoning = 0;
        row.write = 0;
        row.read = 0;
        row.total = 0;
        row.provider_total = None;
        row.raw = Some(json!({"inputTokens":0,"outputTokens":0}).to_string());
        assert_eq!(parse(row).unwrap().tally.total(), 0);
    }

    #[test]
    fn missing_invalid_and_conflicting_counters_are_not_guessed() {
        for raw in [
            json!({}),
            json!({"inputTokens":100}),
            json!({"inputTokens":100,"outputTokens":20.0}),
            json!({"inputTokens":100,"outputTokens":null}),
        ] {
            let mut row = record();
            row.raw = Some(raw.to_string());
            assert!(parse(row).is_none());
        }
        for change in 0..7 {
            let mut row = record();
            match change {
                0 => row.read = 101,
                1 => row.input = -1,
                2 => row.reasoning = 21,
                3 => row.total = 121,
                4 => row.provider_total = Some(130),
                5 => row.input = MAX_COUNTER + 1,
                _ => row.status = "running".into(),
            }
            assert!(parse(row).is_none());
        }
    }

    #[test]
    fn absent_model_is_counted_as_unknown_with_a_gap_and_invalid_time_is_skipped() {
        let mut row = record();
        row.model.clear();
        let parsed = parse(row).unwrap();
        assert!(parsed.partial);
        assert_eq!(parsed.model, "unknown");
        let mut row = record();
        row.at = 0;
        assert!(parse(row).is_none());
    }

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "qs-zcode-{}-{}-{}",
                std::process::id(),
                crate::timeutil::now_ms(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            let fixture = Self(path);
            fixture.db().execute_batch("CREATE TABLE model_usage (
                id TEXT PRIMARY KEY, logical_request_id TEXT, attempt_index INTEGER, model_id TEXT,
                status TEXT, started_at INTEGER, input_tokens INTEGER, output_tokens INTEGER,
                reasoning_tokens INTEGER, cache_creation_input_tokens INTEGER, cache_read_input_tokens INTEGER,
                computed_total_tokens INTEGER, provider_total_tokens INTEGER, raw_usage_json TEXT);
                CREATE INDEX model_usage_started_model_idx ON model_usage(started_at, model_id);").unwrap();
            fixture
        }
        fn path(&self) -> std::path::PathBuf {
            self.0.join("db.sqlite")
        }
        fn db(&self) -> Connection {
            Connection::open(self.path()).unwrap()
        }
        fn insert(&self, row: Record) {
            self.db()
                .execute(
                    "INSERT OR REPLACE INTO model_usage VALUES
                (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    rusqlite::params![
                        row.id,
                        row.logical,
                        row.attempt,
                        row.model,
                        row.status,
                        row.at,
                        row.input,
                        row.output,
                        row.reasoning,
                        row.write,
                        row.read,
                        row.total,
                        row.provider_total,
                        row.raw
                    ],
                )
                .unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn total(report: &Report) -> i64 {
        report
            .buckets
            .values()
            .flat_map(|m| m.values())
            .map(TokenTally::total)
            .sum()
    }

    #[test]
    fn primary_key_updates_replace_counts_while_independent_retries_accumulate() {
        let fixture = Fixture::new();
        fixture.insert(record());
        fixture.insert(record());
        let mut second = record();
        second.id = "request-1-attempt-1".into();
        second.attempt = 1;
        second.status = "error".into();
        fixture.insert(second);
        let before = std::fs::read(fixture.path()).unwrap();
        let report = read_counts(&fixture.path());
        assert_eq!(total(&report), 240);
        assert_eq!(report.rows_read, 2);
        assert!(report.complete && !report.partial);
        assert_eq!(std::fs::read(fixture.path()).unwrap(), before);
    }

    #[test]
    fn row_limits_missing_usage_and_broken_schema_are_partial() {
        let fixture = Fixture::new();
        fixture.insert(record());
        let mut missing = record();
        missing.id = "missing".into();
        missing.raw = None;
        fixture.insert(missing);
        let report = read_counts(&fixture.path());
        assert_eq!(total(&report), 120);
        assert_eq!(report.rows_skipped, 1);
        assert!(report.complete && report.partial);
        let limited = read_limited(&fixture.path(), 1);
        assert!(limited.partial && !limited.complete);
        fixture
            .db()
            .execute_batch("DROP TABLE model_usage;")
            .unwrap();
        assert!(read_counts(&fixture.path()).partial);
    }

    #[test]
    fn committed_wal_rows_are_read_and_pre_cancelled_scans_publish_nothing() {
        let fixture = Fixture::new();
        let db = fixture.db();
        db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        fixture.insert(record());
        assert_eq!(total(&read_counts(&fixture.path())), 120);
        let control = std::sync::Arc::new(crate::scan::Control::default());
        control.cancel();
        let result = crate::scan::run(control, 1, |_| {}, || read_counts(&fixture.path()));
        assert!(result.is_err());
    }
}
