//! Hermes cumulative session usage, adapted from Pulse 1.7.2 HermesReader.swift,
//! Copyright (c) 2026 qunqin24, Apache-2.0. Read no conversation or billing data.
use crate::ledger::{TokenTally, UsageLedger};
use crate::sqlite_snapshot::{wal_index_head, Stamp};
use rusqlite::{limits::Limit, types::ValueRef, Connection, OpenFlags, Row};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_COUNTER: i64 = 1_000_000_000_000;
const MAX_ROWS: usize = 100_000;
const MAX_MEMORY: usize = 8 * 1024 * 1024;
const MAX_DATABASES: usize = 16;
const CACHE_TTL: Duration = Duration::from_secs(300);
type Buckets = BTreeMap<String, BTreeMap<String, TokenTally>>;
type UnknownBuckets = BTreeMap<String, BTreeMap<String, i64>>;

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub buckets: Buckets,
    pub unclassified: UnknownBuckets,
    pub partial: bool,
    pub rows_read: usize,
    pub rows_skipped: usize,
    complete: bool,
    bytes: usize,
}

#[derive(Debug)]
struct Counts {
    input: Option<i64>,
    output: Option<i64>,
    read: Option<i64>,
    write: Option<i64>,
    reasoning: Option<i64>,
}
impl Counts {
    fn from_row(row: &Row<'_>, first: usize) -> Self {
        let count = |i| match row.get_ref(i).ok()? {
            ValueRef::Integer(n) if (0..=MAX_COUNTER).contains(&n) => Some(n),
            _ => None,
        };
        Self {
            input: count(first),
            output: count(first + 1),
            read: count(first + 2),
            write: count(first + 3),
            reasoning: count(first + 4),
        }
    }
    fn measured(&self) -> (TokenTally, i64, bool) {
        let split = self.read == Some(0) && self.write == Some(0);
        let input = self.input.unwrap_or(0);
        (
            TokenTally {
                input: if split { input } else { 0 },
                output: self.output.unwrap_or(0),
                ..TokenTally::default()
            },
            if split { 0 } else { input },
            !split || self.input.is_none() || self.output.is_none() || self.reasoning != Some(0),
        )
    }
}
struct Session {
    model: String,
    at: Option<i64>,
    counts: Counts,
}

fn text(row: &Row<'_>, index: usize, cap: usize) -> Option<String> {
    let ValueRef::Text(bytes) = row.get_ref(index).ok()? else {
        return None;
    };
    let value = std::str::from_utf8(bytes).ok()?.trim();
    (!value.is_empty() && value.len() <= cap).then(|| value.into())
}
fn epoch(row: &Row<'_>, index: usize) -> Option<i64> {
    let n = match row.get_ref(index).ok()? {
        ValueRef::Real(n) => n,
        ValueRef::Integer(n) => n as f64,
        _ => return None,
    };
    // Hermes stores epoch seconds. Milliseconds or implausible years are not guessed.
    if !n.is_finite() || !(0.001..=253_402_214_399.0).contains(&n) {
        return None;
    }
    let ms = (n * 1000.0) as i64;
    chrono::DateTime::from_timestamp_millis(ms)?;
    Some(ms)
}

/// Only fixed table/column names enter SQL. Missing old columns remain NULL,
/// which is a coverage gap, never an invented zero. Views are not executed.
fn select(
    db: &Connection,
    table: &str,
    required: &str,
    columns: &[&str],
) -> Result<Option<String>, ()> {
    let kind = db.query_row(
        "SELECT type FROM sqlite_schema WHERE name=?1",
        [table],
        |r| r.get::<_, String>(0),
    );
    let kind = match kind {
        Ok(kind) => kind,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(_) => return Err(()),
    };
    if kind != "table" {
        return Err(());
    }
    let mut query = db
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|_| ())?;
    let names = query
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|_| ())?
        .take(129)
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .map_err(|_| ())?;
    if names.len() > 128 || !names.contains(required) {
        return Err(());
    }
    let columns = columns
        .iter()
        .map(|c| if names.contains(*c) { *c } else { "NULL" })
        .collect::<Vec<_>>()
        .join(",");
    Ok(Some(format!("SELECT {columns} FROM {table} LIMIT ?1")))
}

impl Report {
    fn add(&mut self, at: Option<i64>, model: &str, counts: &Counts) -> bool {
        let (tally, unclassified, partial) = counts.measured();
        self.partial |= partial || model == "unknown";
        if tally.total() + unclassified == 0 {
            return true;
        }
        let Some(at) = at else {
            self.partial = true;
            self.rows_skipped += 1;
            return true;
        };
        let slot = crate::ledger::slot_key_from_ms(at);
        if !self
            .buckets
            .get(&slot)
            .is_some_and(|m| m.contains_key(model))
        {
            self.bytes += 512 + slot.len() + model.len();
            if self.bytes > MAX_MEMORY {
                self.partial = true;
                return false;
            }
        }
        *self
            .buckets
            .entry(slot.clone())
            .or_default()
            .entry(model.into())
            .or_default() += tally;
        if unclassified > 0 {
            *self
                .unclassified
                .entry(slot)
                .or_default()
                .entry(model.into())
                .or_default() += unclassified;
        }
        true
    }
    pub(crate) fn price(
        &self,
        prices: &BTreeMap<String, crate::model_prices::ModelPrice>,
    ) -> UsageLedger {
        let mut ledger =
            crate::ledger::priced_with_unclassified(&self.buckets, &self.unclassified, prices);
        ledger.has_partial_records = self.partial;
        ledger
    }
}

pub(crate) fn roots(home: &Path, custom: Option<PathBuf>, local: Option<PathBuf>) -> Vec<PathBuf> {
    let root = custom
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(".hermes"));
    let mut paths = vec![root.join("state.db"), root.join("profiles")];
    if let Some(local) = local {
        paths.push(local.join("hermes/state.db"));
    }
    paths.push(home.join("AppData/Local/hermes/state.db"));
    paths
}

#[derive(Default)]
struct Discovery {
    paths: Vec<PathBuf>,
    partial: bool,
}
fn plain(path: &Path, directory: bool) -> Result<bool, ()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(()),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(());
        }
    }
    if metadata.file_type().is_symlink() {
        return Err(());
    }
    Ok(if directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    })
}
fn discover(paths: &[PathBuf]) -> Discovery {
    let mut found = Discovery::default();
    let mut candidates = Vec::new();
    let mut listed = 0;
    for path in paths.iter().take(MAX_DATABASES) {
        if !crate::scan::checkpoint() || candidates.len() >= 64 {
            found.partial = true;
            break;
        }
        if path.components().any(|p| p.as_os_str() == "UsageImports") {
            continue;
        }
        if path.file_name().is_some_and(|n| n == "state.db") {
            candidates.push(path.clone());
            continue;
        }
        let profiles = if path.file_name().is_some_and(|n| n == "profiles") {
            path.clone()
        } else {
            candidates.push(path.join("state.db"));
            path.join("profiles")
        };
        if profiles.parent().is_none_or(|p| plain(p, true) != Ok(true)) {
            if profiles.try_exists().unwrap_or(true) {
                found.partial = true;
            }
            continue;
        }
        match plain(&profiles, true) {
            Ok(false) => {
                found.partial |= profiles.try_exists().unwrap_or(true);
                continue;
            }
            Err(_) => {
                found.partial = true;
                continue;
            }
            Ok(true) => {}
        }
        let Ok(entries) = std::fs::read_dir(&profiles) else {
            found.partial = true;
            continue;
        };
        for entry in entries {
            listed += 1;
            if !crate::scan::checkpoint() || listed > 1000 {
                found.partial = true;
                break;
            }
            let Ok(entry) = entry else {
                found.partial = true;
                continue;
            };
            match plain(&entry.path(), true) {
                Ok(true) => candidates.push(entry.path().join("state.db")),
                Err(_) => found.partial = true,
                _ => {}
            }
            if candidates.len() >= 64 {
                found.partial = true;
                break;
            }
        }
    }
    found.partial |= paths.len() > MAX_DATABASES;
    candidates.sort();
    candidates.dedup();
    let mut seen = BTreeSet::new();
    for path in candidates {
        if !crate::scan::checkpoint() {
            found.partial = true;
            break;
        }
        if path.as_os_str().len() > 32 * 1024 {
            found.partial = true;
            continue;
        }
        if path.parent().is_none_or(|p| plain(p, true) != Ok(true)) {
            if path.try_exists().unwrap_or(true) {
                found.partial = true;
            }
            continue;
        }
        match plain(&path, false) {
            Ok(false) => {
                found.partial |= path.try_exists().unwrap_or(true);
                continue;
            }
            Err(_) => {
                found.partial = true;
                continue;
            }
            Ok(true) => {}
        }
        let (Ok(canonical), Some(stamp)) = (path.canonicalize(), Stamp::of(&path)) else {
            found.partial = true;
            continue;
        };
        if let Some(id) = stamp.database.identity {
            if !seen.insert(id) {
                continue;
            }
        }
        if found.paths.len() == MAX_DATABASES {
            found.partial = true;
            break;
        }
        found.paths.push(canonical);
    }
    found
}

struct Kept {
    stamp: Stamp,
    wal: Option<[u8; 48]>,
    offset: i32,
    report: Arc<Report>,
    used: Instant,
}
#[derive(Default)]
struct Memory {
    generation: u64,
    reports: BTreeMap<PathBuf, Kept>,
}
#[derive(Default)]
pub struct Reader {
    memory: Mutex<Memory>,
}
pub struct CachedRead {
    pub report: Arc<Report>,
    pub reused: bool,
}
impl Reader {
    pub fn clear(&self) {
        let mut m = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        m.generation = m.generation.wrapping_add(1);
        m.reports.clear();
    }
    fn release(&self, enabled: bool) {
        if let Ok(mut m) = self.memory.try_lock() {
            let before = m.reports.len();
            m.reports
                .retain(|_, k| enabled && k.used.elapsed() < CACHE_TTL);
            if !enabled || before != m.reports.len() {
                m.generation = m.generation.wrapping_add(1);
            }
        }
    }
    pub fn read(&self, path: &Path) -> CachedRead {
        if !crate::scan::checkpoint() {
            return CachedRead {
                report: Arc::new(Report {
                    partial: true,
                    ..Report::default()
                }),
                reused: false,
            };
        }
        let stamp = Stamp::of(path);
        let wal = stamp
            .as_ref()
            .filter(|s| s.has_wal())
            .and_then(|_| wal_index_head(path));
        let can_cache = stamp
            .as_ref()
            .is_some_and(|s| s.has_identity() && (!s.has_wal() || wal.is_some()));
        let offset = chrono::Local::now().offset().local_minus_utc();
        let generation = {
            let mut m = self.memory.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(k) = m.reports.get_mut(path).filter(|k| {
                can_cache
                    && Some(&k.stamp) == stamp.as_ref()
                    && k.wal == wal
                    && k.offset == offset
                    && k.used.elapsed() < CACHE_TTL
            }) {
                k.used = Instant::now();
                return CachedRead {
                    report: k.report.clone(),
                    reused: true,
                };
            }
            if stamp.is_none() {
                m.reports.remove(path);
            }
            m.generation
        };
        let report = Arc::new(read_counts(path));
        if can_cache
            && report.complete
            && crate::scan::checkpoint()
            && Stamp::of(path) == stamp
            && (wal.is_none() || wal_index_head(path) == wal)
        {
            let mut m = self.memory.lock().unwrap_or_else(|e| e.into_inner());
            if m.generation == generation && crate::scan::checkpoint() {
                m.reports.retain(|_, k| k.used.elapsed() < CACHE_TTL);
                while m.reports.len() >= MAX_DATABASES
                    || m.reports.values().map(|k| k.report.bytes).sum::<usize>() + report.bytes
                        > MAX_MEMORY
                {
                    let Some(oldest) = m
                        .reports
                        .iter()
                        .min_by_key(|(_, k)| k.used)
                        .map(|(p, _)| p.clone())
                    else {
                        break;
                    };
                    m.reports.remove(&oldest);
                }
                m.reports.insert(
                    path.to_path_buf(),
                    Kept {
                        stamp: stamp.unwrap(),
                        wal,
                        offset,
                        report: report.clone(),
                        used: Instant::now(),
                    },
                );
            }
        }
        CachedRead {
            report,
            reused: false,
        }
    }
}
static READER: OnceLock<Reader> = OnceLock::new();
pub(crate) fn clear_memory() {
    if let Some(reader) = READER.get() {
        reader.clear();
    }
}
pub(crate) fn release_memory(enabled: bool) {
    if let Some(reader) = READER.get() {
        reader.release(enabled);
    }
}
pub(crate) fn read_paths(paths: &[PathBuf]) -> Report {
    let found = discover(paths);
    let mut combined = Report {
        partial: found.partial,
        ..Report::default()
    };
    let reader = READER.get_or_init(Reader::default);
    let deadline = Instant::now() + Duration::from_secs(20);
    for path in found.paths {
        if !crate::scan::checkpoint() || Instant::now() >= deadline {
            combined.partial = true;
            break;
        }
        let report = reader.read(&path).report;
        combined.partial |= report.partial;
        combined.rows_read += report.rows_read;
        combined.rows_skipped += report.rows_skipped;
        for (slot, models) in &report.buckets {
            for (model, tally) in models {
                if !crate::scan::checkpoint() {
                    combined.partial = true;
                    return combined;
                }
                if !combined
                    .buckets
                    .get(slot)
                    .is_some_and(|m| m.contains_key(model))
                {
                    combined.bytes += 512 + slot.len() + model.len();
                    if combined.bytes > MAX_MEMORY {
                        combined.partial = true;
                        return combined;
                    }
                }
                *combined
                    .buckets
                    .entry(slot.clone())
                    .or_default()
                    .entry(model.clone())
                    .or_default() += *tally;
                if let Some(n) = report.unclassified.get(slot).and_then(|m| m.get(model)) {
                    *combined
                        .unclassified
                        .entry(slot.clone())
                        .or_default()
                        .entry(model.clone())
                        .or_default() += *n;
                }
            }
        }
    }
    combined
}

pub fn read_counts(path: &Path) -> Report {
    read_limited(path, MAX_ROWS)
}

pub fn read_with_prices(
    path: &Path,
    prices: &BTreeMap<String, crate::model_prices::ModelPrice>,
) -> UsageLedger {
    read_counts(path).price(prices)
}
fn read_limited(path: &Path, limit: usize) -> Report {
    let mut report = Report::default();
    let Some(before) = Stamp::of(path) else {
        report.partial = path.try_exists().unwrap_or(true);
        return report;
    };
    let wal = before.has_wal().then(|| wal_index_head(path)).flatten();
    if !crate::scan::checkpoint() {
        report.partial = true;
        return report;
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    let Ok(db) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        report.partial = true;
        return report;
    };
    crate::scan::file_read();
    let setup = db.busy_timeout(Duration::from_millis(100)).and_then(|_| db.execute_batch(
        "PRAGMA query_only=ON; PRAGMA trusted_schema=OFF; PRAGMA cache_size=-1024; PRAGMA mmap_size=0; PRAGMA temp_store=MEMORY; BEGIN DEFERRED;"
    ));
    if setup.is_err() || db.set_limit(Limit::SQLITE_LIMIT_LENGTH, 64 * 1024).is_err() {
        report.partial = true;
        return report;
    }
    db.progress_handler(
        1000,
        Some(move || !crate::scan::checkpoint() || Instant::now() >= deadline),
    );
    let scanned = scan_tables(&db, &mut report, limit, deadline);
    report.complete = scanned.is_ok();
    report.partial |= scanned.is_err();
    if !crate::scan::checkpoint()
        || Stamp::of(path).as_ref() != Some(&before)
        || wal.is_some_and(|head| wal_index_head(path) != Some(head))
    {
        report.partial = true;
        report.complete = false;
    }
    report
}

fn scan_tables(
    db: &Connection,
    report: &mut Report,
    limit: usize,
    deadline: Instant,
) -> Result<(), ()> {
    let active = || crate::scan::checkpoint() && Instant::now() < deadline;
    let sql = select(
        db,
        "sessions",
        "id",
        &[
            "id",
            "model",
            "started_at",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "reasoning_tokens",
        ],
    )?
    .ok_or(())?;
    let mut query = db.prepare(&sql).map_err(|_| ())?;
    let mut rows = query
        .query([limit.saturating_add(1) as i64])
        .map_err(|_| ())?;
    let mut sessions = BTreeMap::new();
    let mut session_bytes = 0;
    let mut count = 0;
    let mut truncated = false;
    while let Some(row) = rows.next().map_err(|_| ())? {
        if !active() {
            return Err(());
        }
        if count >= limit {
            truncated = true;
            report.partial = true;
            break;
        }
        count += 1;
        report.rows_read += 1;
        let Some(id) = text(row, 0, 1024) else {
            report.partial = true;
            report.rows_skipped += 1;
            continue;
        };
        let model = text(row, 1, 256).unwrap_or_else(|| "unknown".into());
        session_bytes += 256 + id.len() + model.len();
        if session_bytes > MAX_MEMORY || sessions.contains_key(&id) {
            return Err(());
        }
        sessions.insert(
            id,
            Session {
                model,
                at: epoch(row, 2),
                counts: Counts::from_row(row, 3),
            },
        );
    }
    drop(rows);
    drop(query);
    let mut covered = BTreeSet::new();
    if let Some(sql) = select(
        db,
        "session_model_usage",
        "session_id",
        &[
            "session_id",
            "model",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_write_tokens",
            "reasoning_tokens",
        ],
    )? {
        let mut query = db.prepare(&sql).map_err(|_| ())?;
        let mut rows = query
            .query([limit.saturating_add(1) as i64])
            .map_err(|_| ())?;
        let mut count = 0;
        while let Some(row) = rows.next().map_err(|_| ())? {
            if !active() || count >= limit {
                return Err(());
            }
            count += 1;
            report.rows_read += 1;
            let Some(id) = text(row, 0, 1024) else {
                report.partial = true;
                report.rows_skipped += 1;
                continue;
            };
            let Some(session) = sessions.get(&id) else {
                report.partial = true;
                report.rows_skipped += 1;
                continue;
            };
            // Including valid zero rows prevents re-emitting obsolete session totals.
            covered.insert(id);
            let model = text(row, 1, 256).unwrap_or_else(|| "unknown".into());
            if !report.add(session.at, &model, &Counts::from_row(row, 2)) {
                return Err(());
            }
        }
    }
    for (id, session) in sessions {
        if !active() {
            return Err(());
        }
        if !covered.contains(&id) && !report.add(session.at, &session.model, &session.counts) {
            return Err(());
        }
    }
    if truncated {
        Err(())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "qs-hermes-{}-{}-{}",
                std::process::id(),
                crate::timeutil::now_ms(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root.join("state.db"))
        }
        fn open(&self) -> Connection {
            Connection::open(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(self.0.with_file_name(format!("state.db{suffix}")));
            }
            let _ = std::fs::remove_dir(self.0.parent().unwrap());
        }
    }
    fn schema(db: &Connection) {
        db.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY, model TEXT, started_at REAL, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER); CREATE TABLE session_model_usage(session_id TEXT, model TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER);").unwrap();
    }
    fn tokens(report: &Report) -> i64 {
        report
            .buckets
            .values()
            .flat_map(|m| m.values())
            .map(TokenTally::total)
            .sum::<i64>()
            + report
                .unclassified
                .values()
                .flat_map(|m| m.values())
                .sum::<i64>()
    }
    #[test]
    fn hermes_model_rows_cover_cumulative_session_and_keep_all_routes_once() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','old',1791100800,9000,8000,0,0,0),('fallback','solo',1791100800,30,10,0,0,0); INSERT INTO session_model_usage VALUES('s','a',100,20,0,0,0),('s','a',5,3,0,0,0),('s','b',40,7,0,0,0);").unwrap();
        let report = read_counts(&f.0);
        assert_eq!(tokens(&report), 215);
        assert!(!report.partial && report.complete);
        let all = report.buckets.values().next().unwrap();
        assert_eq!(all["a"].total(), 128);
        assert_eq!(all["b"].total(), 47);
        assert_eq!(all["solo"].total(), 40);
        assert!(!all.contains_key("old"));
        assert_eq!(tokens(&read_counts(&f.0)), 215);
    }
    #[test]
    fn hermes_cache_and_reasoning_never_double_count_and_model_filters_keep_unknown_kind() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','known',1791100800,100,20,50,10,7);")
            .unwrap();
        let report = read_counts(&f.0);
        assert_eq!(tokens(&report), 120);
        assert!(report.partial);
        let prices = BTreeMap::from([(
            "known".into(),
            crate::model_prices::ModelPrice {
                input: 1.0,
                output: 2.0,
                cache_read: None,
                cache_write: None,
                name: None,
            },
        )]);
        let ledger = report.price(&prices);
        let day = ledger.days.iter().find(|d| d.tokens > 0).unwrap();
        assert_eq!(day.tokens, 120);
        assert_eq!(day.tally.input, 0);
        assert_eq!(day.tally.output, 20);
        assert_eq!(day.models["known"], 120);
        assert_eq!(day.unpriced_tokens, 100);
        assert!((day.cost - 0.00004).abs() < 1e-12);
        assert!(ledger.has_partial_records);
    }
    #[test]
    fn hermes_zero_and_invalid_model_rows_do_not_revive_old_session_totals() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('zero','old',1791100800,999,999,0,0,0),('bad','old',1791100800,999,999,0,0,0); INSERT INTO session_model_usage VALUES('zero','a',0,0,0,0,0),('bad','a',-1,20,0,0,0);").unwrap();
        let report = read_counts(&f.0);
        assert_eq!(tokens(&report), 20);
        assert!(report.partial);
        assert!(report.complete);
    }
    #[test]
    fn hermes_old_columns_and_nulls_preserve_known_counts_without_assuming_cache_zero() {
        let f = Fixture::new();
        let db = f.open();
        db.execute_batch("CREATE TABLE sessions(id TEXT, model TEXT, started_at REAL, input_tokens INTEGER, output_tokens INTEGER); INSERT INTO sessions VALUES('s','a',1791100800,100,20),('input',NULL,1791100800,30,NULL),('no-date','a',NULL,99,88);").unwrap();
        let report = read_counts(&f.0);
        assert_eq!(tokens(&report), 150);
        assert!(report.partial);
        assert_eq!(report.buckets.values().next().unwrap()["a"].input, 0);
        assert_eq!(report.unclassified.values().next().unwrap()["unknown"], 30);
        assert_eq!(report.rows_skipped, 1);
    }
    #[test]
    fn hermes_invalid_counter_types_limits_and_orphans_remain_partial() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('a','m',1791100800,1.5,5,0,0,0),('b','m',1791100800,1000000000001,7,0,0,0),('c','m',1791100800,9,11,NULL,0,0); INSERT INTO session_model_usage VALUES('orphan','m',999,999,0,0,0);").unwrap();
        let report = read_counts(&f.0);
        assert_eq!(tokens(&report), 32);
        assert!(report.partial);
        assert_eq!(report.rows_skipped, 1);
        assert_eq!(report.unclassified.values().next().unwrap()["m"], 9);
    }
    #[test]
    fn hermes_committed_wal_is_visible_but_uncommitted_counts_and_messages_are_not_read() {
        let f = Fixture::new();
        let db = f.open();
        db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,100,20,0,0,0); CREATE TABLE messages(body TEXT); INSERT INTO messages VALUES('conversation data');").unwrap();
        assert_eq!(tokens(&read_counts(&f.0)), 120);
        db.execute_batch("BEGIN; UPDATE sessions SET input_tokens=999;")
            .unwrap();
        assert_eq!(tokens(&read_counts(&f.0)), 120);
        db.execute_batch("ROLLBACK; UPDATE sessions SET input_tokens=150;")
            .unwrap();
        assert_eq!(tokens(&read_counts(&f.0)), 170);
    }
    #[test]
    fn hermes_row_budget_keeps_readable_subset_and_changed_schema_is_not_executed() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('a','m',1791100800,10,1,0,0,0),('b','m',1791100800,20,2,0,0,0),('c','m',1791100800,30,3,0,0,0);").unwrap();
        let report = read_limited(&f.0, 2);
        assert_eq!(tokens(&report), 33);
        assert!(report.partial && !report.complete);
        db.execute_batch("DROP TABLE session_model_usage; CREATE VIEW session_model_usage AS SELECT id AS session_id,model,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens FROM sessions;").unwrap();
        let report = read_counts(&f.0);
        assert_eq!(tokens(&report), 0);
        assert!(report.partial && !report.complete);
    }
    #[test]
    fn hermes_cancelled_scan_cannot_publish_a_complete_result() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,100,20,0,0,0);")
            .unwrap();
        let control = std::sync::Arc::new(crate::scan::Control::default());
        let cancelled = control.clone();
        assert!(crate::scan::run(
            control,
            1,
            |_| {},
            || {
                cancelled.cancel();
                read_counts(&f.0)
            }
        )
        .is_err());
        assert_eq!(tokens(&read_counts(&f.0)), 120);
    }

    #[test]
    fn hermes_cache_observes_growth_partial_repair_delete_and_recreate() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,100,20,0,0,0);")
            .unwrap();
        let reader = Reader::default();
        assert!(!reader.read(&f.0).reused);
        assert!(reader.read(&f.0).reused);
        db.execute_batch("UPDATE sessions SET input_tokens=150;")
            .unwrap();
        let result = reader.read(&f.0);
        assert!(!result.reused);
        assert_eq!(tokens(&result.report), 170);
        db.execute_batch("UPDATE sessions SET input_tokens=NULL;")
            .unwrap();
        let result = reader.read(&f.0);
        assert!(result.report.partial);
        assert_eq!(tokens(&result.report), 20);
        assert!(reader.read(&f.0).reused);
        db.execute_batch("UPDATE sessions SET input_tokens=100;")
            .unwrap();
        assert!(!reader.read(&f.0).report.partial);
        drop(db);
        std::fs::remove_file(&f.0).unwrap();
        let result = reader.read(&f.0);
        assert_eq!(tokens(&result.report), 0);
        assert!(!result.reused);
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,10,2,0,0,0);")
            .unwrap();
        assert_eq!(tokens(&reader.read(&f.0).report), 12);
    }
    #[test]
    fn hermes_cache_tracks_wal_commit_and_same_mtime_file_replacement() {
        let f = Fixture::new();
        let db = f.open();
        db.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,100,20,0,0,0);")
            .unwrap();
        let reader = Reader::default();
        assert_eq!(tokens(&reader.read(&f.0).report), 120);
        assert!(reader.read(&f.0).reused);
        db.execute_batch("UPDATE sessions SET input_tokens=150;")
            .unwrap();
        let result = reader.read(&f.0);
        assert!(!result.reused);
        assert_eq!(tokens(&result.report), 170);
        drop(db);
        reader.read(&f.0);
        let before = std::fs::metadata(&f.0).unwrap();
        let other = Fixture::new();
        let db = other.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,200,20,0,0,0);")
            .unwrap();
        drop(db);
        assert_eq!(std::fs::metadata(&other.0).unwrap().len(), before.len());
        std::fs::File::options()
            .write(true)
            .open(&other.0)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
            .unwrap();
        std::fs::remove_file(&f.0).unwrap();
        std::fs::rename(&other.0, &f.0).unwrap();
        let result = reader.read(&f.0);
        assert!(!result.reused);
        assert_eq!(tokens(&result.report), 220);
    }
    #[test]
    fn hermes_memory_release_clear_and_cancel_reject_old_reuse() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,100,20,0,0,0);")
            .unwrap();
        let reader = Reader::default();
        reader.read(&f.0);
        reader.release(true);
        assert!(reader.read(&f.0).reused);
        reader.release(false);
        assert!(!reader.read(&f.0).reused);
        reader.clear();
        assert!(!reader.read(&f.0).reused);
        {
            let mut m = reader.memory.lock().unwrap();
            for k in m.reports.values_mut() {
                k.used = Instant::now() - CACHE_TTL;
            }
        }
        reader.release(true);
        assert!(!reader.read(&f.0).reused);
        let control = Arc::new(crate::scan::Control::default());
        let cancelled = control.clone();
        assert!(crate::scan::run(
            control,
            1,
            |_| {},
            || {
                cancelled.cancel();
                let result = reader.read(&f.0);
                assert!(!result.reused);
                assert!(result.report.partial);
            }
        )
        .is_err());
        assert!(reader.read(&f.0).reused);
    }
    #[test]
    fn hermes_discovery_finds_named_profiles_deduplicates_aliases_and_keeps_imports_separate() {
        let f = Fixture::new();
        let db = f.open();
        schema(&db);
        db.execute_batch("INSERT INTO sessions VALUES('s','m',1791100800,10,2,0,0,0);")
            .unwrap();
        drop(db);
        let root = f.0.parent().unwrap();
        let profiles = root.join("profiles");
        let team = profiles.join("team");
        std::fs::create_dir_all(&team).unwrap();
        std::fs::copy(&f.0, team.join("state.db")).unwrap();
        let found = discover(&[root.to_path_buf(), profiles.clone(), f.0.clone()]);
        assert_eq!(found.paths.len(), 2);
        assert!(!found.partial);
        let imports = root.join("UsageImports/hermes");
        std::fs::create_dir_all(&imports).unwrap();
        std::fs::write(imports.join("one.json"),format!(r#"{{"schema":"quotascope.usage.v1","source":"hermes","id":"import","timestamp":{},"model":"m","usage":{{"inputTokens":3,"outputTokens":4}}}}"#,crate::timeutil::now_ms())).unwrap();
        let ledger = crate::additional_spend::read_with_prices(
            "hermes",
            &[root.to_path_buf(), imports.clone()],
            &BTreeMap::new(),
        );
        assert_eq!(ledger.days.iter().map(|d| d.tokens).sum::<i64>(), 31);
        assert!(crate::additional_spend::native_supported("hermes"));
        let paths = roots(root, Some(root.join("custom")), Some(root.join("local")));
        assert_eq!(paths[0], root.join("custom/state.db"));
        assert_eq!(paths[1], root.join("custom/profiles"));
        assert!(!paths.contains(&root.join(".hermes/state.db")));
        std::fs::remove_file(imports.join("one.json")).unwrap();
        std::fs::remove_dir(&imports).unwrap();
        std::fs::remove_dir(root.join("UsageImports")).unwrap();
        std::fs::remove_file(team.join("state.db")).unwrap();
        std::fs::remove_dir(team).unwrap();
        std::fs::remove_dir(profiles).unwrap();
    }
    #[test]
    fn hermes_discovery_caps_databases_and_rechecks_new_profiles() {
        let f = Fixture::new();
        let root = f.0.parent().unwrap();
        let profiles = root.join("profiles");
        std::fs::create_dir(&profiles).unwrap();
        assert!(discover(std::slice::from_ref(&profiles)).paths.is_empty());
        for n in 0..17 {
            let dir = profiles.join(format!("p{n:02}"));
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("state.db"), []).unwrap();
        }
        let found = discover(std::slice::from_ref(&profiles));
        assert_eq!(found.paths.len(), 16);
        assert!(found.partial);
        for n in 0..17 {
            let dir = profiles.join(format!("p{n:02}"));
            std::fs::remove_file(dir.join("state.db")).unwrap();
            std::fs::remove_dir(dir).unwrap();
        }
        std::fs::remove_dir(profiles).unwrap();
    }
}
