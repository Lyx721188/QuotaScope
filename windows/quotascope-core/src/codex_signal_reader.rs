//! A bounded, memory-only cache of complete Codex rollout facts. The caller
//! owns this reader on a worker and drops it when the settings page closes.
use crate::codex_signals::{self, Counts, Facts, ALGORITHM};
use chrono::{Days, Local, NaiveDate};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_FILES: usize = 1024;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_LISTED: usize = 100_000;

#[derive(Clone, Debug)]
pub struct LocatedChange {
    pub session: String,
    pub change: codex_signals::Change,
}
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub models: BTreeMap<String, Counts>,
    pub changes: Vec<LocatedChange>,
    pub changes_count: usize,
    pub sessions: usize,
    pub judged_sessions: usize,
    pub partial: bool,
    pub files_parsed: usize,
    pub files_reused: usize,
    pub failed_files: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: std::time::SystemTime,
}
impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            size: meta.len(),
            modified: meta.modified().ok()?,
        })
    }
}
struct Kept {
    stamp: Stamp,
    algorithm: &'static str,
    facts: Arc<Facts>,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
pub struct Reader {
    kept: BTreeMap<PathBuf, Kept>,
    bytes: usize,
    clock: u64,
    offset: Option<i32>,
}

impl Reader {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn retained_files(&self) -> usize {
        self.kept.len()
    }
    /// Accounting budget for decoded facts, not a process RSS measurement.
    pub fn estimated_bytes(&self) -> usize {
        self.bytes
    }

    pub fn read(&mut self, home: &Path, days: Option<u32>, today: NaiveDate) -> Report {
        self.read_roots(
            &[
                home.join(".codex/sessions"),
                home.join(".codex/archived_sessions"),
            ],
            days,
            today,
        )
    }

    pub fn read_roots(&mut self, roots: &[PathBuf], days: Option<u32>, today: NaiveDate) -> Report {
        let offset = Local::now().offset().local_minus_utc();
        if self.offset != Some(offset) {
            self.clear();
            self.offset = Some(offset);
        }
        let since = days
            .and_then(|days| today.checked_sub_days(Days::new(u64::from(days.saturating_sub(1)))));
        let mut report = Report::default();
        let mut files = BTreeSet::new();
        let mut listed = 0;
        for root in roots {
            collect(root, 0, &mut files, &mut listed, &mut report.partial);
        }
        self.kept.retain(|path, kept| {
            if files.contains(path) {
                true
            } else {
                self.bytes = self.bytes.saturating_sub(kept.bytes);
                false
            }
        });
        for path in files {
            if !crate::scan::checkpoint() {
                report.partial = true;
                break;
            }
            let Some(stamp) = Stamp::of(&path) else {
                report.failed_files += 1;
                report.partial = true;
                continue;
            };
            // Widening the period preserves facts already read. Skip old
            // unmodified files using the same mtime heuristic as the ledger.
            if let Some(since) = since {
                let modified: chrono::DateTime<Local> = stamp.modified.into();
                if modified.date_naive() < since {
                    continue;
                }
            }
            self.clock = self.clock.wrapping_add(1);
            let facts = if let Some(kept) = self
                .kept
                .get_mut(&path)
                .filter(|kept| kept.stamp == stamp && kept.algorithm == ALGORITHM)
            {
                kept.used = self.clock;
                report.files_reused += 1;
                kept.facts.clone()
            } else {
                let parsed = std::fs::File::open(&path).ok().and_then(|file| {
                    let mut lines = crate::scan::LineReader::new(file, Default::default(), 0);
                    let facts = codex_signals::parse(lines.by_ref());
                    (lines.finish().is_ok()
                        && crate::scan::checkpoint()
                        && Stamp::of(&path) == Some(stamp))
                    .then_some(facts)
                });
                let Some(facts) = parsed else {
                    self.remove(&path);
                    report.failed_files += 1;
                    report.partial = true;
                    continue;
                };
                report.files_parsed += 1;
                let facts = Arc::new(facts);
                self.remove(&path);
                let bytes = estimate(&facts) + path.as_os_str().len() * 2 + 2048;
                if bytes <= MAX_BYTES && crate::scan::checkpoint() {
                    while self.kept.len() >= MAX_FILES
                        || self.bytes.saturating_add(bytes) > MAX_BYTES
                    {
                        let Some(oldest) = self
                            .kept
                            .iter()
                            .min_by_key(|(_, kept)| kept.used)
                            .map(|(path, _)| path.clone())
                        else {
                            break;
                        };
                        self.remove(&oldest);
                    }
                    self.bytes += bytes;
                    self.kept.insert(
                        path.clone(),
                        Kept {
                            stamp,
                            algorithm: ALGORITHM,
                            facts: facts.clone(),
                            bytes,
                            used: self.clock,
                        },
                    );
                }
                facts
            };
            report.partial |= facts.partial;
            let in_span = |day: NaiveDate| day <= today && since.is_none_or(|since| day >= since);
            let last_day = facts
                .last_at
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|at| at.with_timezone(&Local).date_naive());
            let present =
                last_day.is_some_and(&in_span) || facts.days.keys().copied().any(&in_span);
            if !present {
                continue;
            }
            report.sessions += 1;
            report.judged_sessions += usize::from(facts.judged);
            for (day, models) in &facts.days {
                if !crate::scan::checkpoint() {
                    report.partial = true;
                    break;
                }
                if !in_span(*day) {
                    continue;
                }
                for (model, counts) in models {
                    if report.models.len() >= 1024 && !report.models.contains_key(model) {
                        report.partial = true;
                        continue;
                    }
                    let combined = report.models.entry(model.clone()).or_default();
                    combined.responses += counts.responses;
                    combined.reached += counts.reached;
                    combined.hits += counts.hits;
                }
            }
            for change in &facts.changes {
                if !crate::scan::checkpoint() {
                    report.partial = true;
                    break;
                }
                let Some(day) = chrono::DateTime::from_timestamp_millis(change.at)
                    .map(|at| at.with_timezone(&Local).date_naive())
                else {
                    continue;
                };
                if !in_span(day) {
                    continue;
                }
                report.changes_count += 1;
                report.changes.push(LocatedChange {
                    session: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    change: change.clone(),
                });
                report.changes.sort_by(|a, b| {
                    b.change
                        .at
                        .cmp(&a.change.at)
                        .then_with(|| a.session.cmp(&b.session))
                        .then(b.change.order.cmp(&a.change.order))
                });
                report.changes.truncate(8);
            }
        }
        report
    }

    fn remove(&mut self, path: &Path) {
        if let Some(old) = self.kept.remove(path) {
            self.bytes = self.bytes.saturating_sub(old.bytes);
        }
    }
}

fn collect(
    path: &Path,
    depth: usize,
    out: &mut BTreeSet<PathBuf>,
    listed: &mut usize,
    partial: &mut bool,
) {
    if !crate::scan::checkpoint() || depth >= 64 || *listed >= MAX_LISTED {
        *partial = true;
        return;
    }
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) => {
            *partial |= error.kind() != std::io::ErrorKind::NotFound;
            return;
        }
    };
    for entry in entries {
        if !crate::scan::checkpoint() || *listed >= MAX_LISTED {
            *partial = true;
            break;
        }
        *listed += 1;
        let Ok(entry) = entry else {
            *partial = true;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            *partial = true;
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect(&entry.path(), depth + 1, out, listed, partial);
        } else if kind.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"))
        {
            out.insert(entry.path());
        }
    }
}

fn estimate(facts: &Facts) -> usize {
    let models = facts
        .days
        .values()
        .map(|models| 1024 + models.keys().map(|name| 1024 + name.len()).sum::<usize>())
        .sum::<usize>();
    let changes = facts
        .changes
        .iter()
        .map(|change| match &change.kind {
            codex_signals::Kind::Model { asked, recorded }
            | codex_signals::Kind::Effort { asked, recorded } => asked.len() + recorded.len(),
            codex_signals::Kind::Context { .. } => 0,
        })
        .sum::<usize>();
    128 + models + changes + 96 * facts.changes.capacity()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "qs-signals-{}-{}-{}",
                std::process::id(),
                crate::timeutil::now_ms(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, day: &str, model: &str) -> PathBuf {
            let path = self.0.join(name);
            let stamp = format!("{day}T12:00:00Z");
            let lines = [
                json!({"timestamp":stamp,"type":"session_meta","payload":{"cli_version":"0.146.0"}}),
                json!({"timestamp":stamp,"type":"turn_context","payload":{"model":model}}),
                json!({"timestamp":stamp,"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"reasoning_output_tokens":516},"total_token_usage":{"total_tokens":100}}}}),
            ];
            std::fs::write(
                &path,
                lines
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n",
            )
            .unwrap();
            path
        }
        fn roots(&self) -> Vec<PathBuf> {
            vec![self.0.clone()]
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 4).unwrap()
    }

    #[test]
    fn periods_reuse_facts_while_append_rewrite_and_delete_invalidate_them() {
        use std::io::Write;
        let fixture = Fixture::new();
        let young = fixture.write("rollout-a.jsonl", "2026-10-04", "model-a");
        fixture.write("rollout-old.jsonl", "2026-08-04", "model-old");
        fixture.write("not-a-rollout.jsonl", "2026-10-04", "ignored");
        let mut reader = Reader::default();
        let month = reader.read_roots(&fixture.roots(), Some(30), today());
        assert_eq!(
            (month.files_parsed, month.sessions, month.models.len()),
            (2, 1, 1)
        );
        let quarter = reader.read_roots(&fixture.roots(), Some(90), today());
        assert_eq!(
            (quarter.files_parsed, quarter.files_reused, quarter.sessions),
            (0, 2, 2)
        );
        std::fs::OpenOptions::new().append(true).open(&young).unwrap().write_all((json!({"timestamp":"2026-10-04T12:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"reasoning_output_tokens":700},"total_token_usage":{"total_tokens":200}}}}).to_string()+"\n").as_bytes()).unwrap();
        let appended = reader.read_roots(&fixture.roots(), Some(30), today());
        assert_eq!(appended.files_parsed, 1);
        assert_eq!(appended.models["model-a"].responses, 2);
        fixture.write("rollout-a.jsonl", "2026-10-04", "model-b");
        let rewritten = reader.read_roots(&fixture.roots(), Some(30), today());
        assert_eq!(rewritten.files_parsed, 1);
        assert!(
            rewritten.models.contains_key("model-b") && !rewritten.models.contains_key("model-a")
        );
        std::fs::remove_file(young).unwrap();
        let deleted = reader.read_roots(&fixture.roots(), Some(30), today());
        assert_eq!(deleted.sessions, 0);
        assert_eq!(reader.retained_files(), 1);
    }

    #[test]
    fn invalid_utf8_never_enters_cache_and_repaired_files_are_read_again() {
        let fixture = Fixture::new();
        let path = fixture.0.join("rollout-bad.jsonl");
        std::fs::write(&path, [0xff, 0xff, 10]).unwrap();
        let mut reader = Reader::default();
        let bad = reader.read_roots(&fixture.roots(), None, today());
        assert!(bad.partial && bad.failed_files == 1 && reader.retained_files() == 0);
        fixture.write("rollout-bad.jsonl", "2026-10-04", "repaired");
        assert_eq!(
            reader.read_roots(&fixture.roots(), None, today()).models["repaired"].responses,
            1
        );
    }

    #[test]
    fn same_size_rewrites_use_exact_file_times_and_model_budget_marks_partial() {
        let fixture = Fixture::new();
        let path = fixture.write("rollout-a.jsonl", "2026-10-04", "model-a");
        let fixed = std::time::SystemTime::now();
        let set_time = |time| {
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(time))
                .unwrap();
        };
        set_time(fixed);
        let before = std::fs::metadata(&path).unwrap().len();
        let mut reader = Reader::default();
        reader.read_roots(&fixture.roots(), None, today());
        fixture.write("rollout-a.jsonl", "2026-10-04", "model-b");
        set_time(fixed + std::time::Duration::from_secs(1));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
        let changed = reader.read_roots(&fixture.roots(), None, today());
        assert_eq!(changed.files_parsed, 1);
        assert!(changed.models.contains_key("model-b"));
        for index in 0..1025 {
            fixture.write(
                &format!("rollout-model-{index:04}.jsonl"),
                "2026-10-04",
                &format!("model-{index:04}"),
            );
        }
        let bounded = reader.read_roots(&fixture.roots(), None, today());
        assert!(bounded.partial);
        assert_eq!(bounded.models.len(), 1024);
    }

    #[test]
    fn cancellation_does_not_cache_an_incomplete_file_or_poison_the_next_read() {
        use std::io::Write;
        let fixture = Fixture::new();
        let path = fixture.write("rollout-long.jsonl", "2026-10-04", "model-a");
        let mut file = std::io::BufWriter::new(
            std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap(),
        );
        for index in 0..50000 {
            writeln!(file, "{}", json!({"timestamp":"2026-10-04T12:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"reasoning_output_tokens":700},"total_token_usage":{"total_tokens":index+200}}}})).unwrap();
        }
        file.flush().unwrap();
        drop(file);
        let control = Arc::new(crate::scan::Control::default());
        let stop = control.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            stop.cancel();
        });
        let mut reader = Reader::default();
        let stopped = crate::scan::run(
            control,
            1,
            |_| {},
            || reader.read_roots(&fixture.roots(), None, today()),
        );
        thread.join().unwrap();
        assert!(stopped.is_err());
        assert_eq!(reader.retained_files(), 0);
        let full = reader.read_roots(&fixture.roots(), None, today());
        assert!(!full.partial);
        assert_eq!(full.models["model-a"].responses, 50001);
    }

    #[test]
    fn file_count_and_accounted_memory_are_bounded_and_recent_facts_survive_eviction() {
        let fixture = Fixture::new();
        for index in 0..MAX_FILES + 2 {
            fixture.write(
                &format!("rollout-{index:04}.jsonl"),
                "2026-10-04",
                "model-a",
            );
        }
        let mut reader = Reader::default();
        let whole = reader.read_roots(&fixture.roots(), None, today());
        assert_eq!(whole.sessions, MAX_FILES + 2);
        assert_eq!(whole.models["model-a"].responses, (MAX_FILES + 2) as u64);
        assert_eq!(reader.retained_files(), MAX_FILES);
        assert!(reader.estimated_bytes() <= MAX_BYTES);
        let mut oversized = Facts::default();
        oversized.days.insert(
            today(),
            (0..70000)
                .map(|n| (format!("model-{n:08}"), Counts::default()))
                .collect(),
        );
        assert!(estimate(&oversized) > MAX_BYTES);
        reader.clear();
        assert_eq!((reader.retained_files(), reader.estimated_bytes()), (0, 0));
    }
}
