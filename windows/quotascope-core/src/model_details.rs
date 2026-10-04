//! Account model shares and measured recent response timings. Missing timing
//! stays unavailable; request-to-finish includes first-token wait.
use crate::ledger::{TokenTally, UsageLedger};
use crate::model::Provider;
use serde_json::Value;
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// An engineering read limit, not a provider's advertised token allowance.
const MAX_TIMING_OUTPUT: i64 = 1_000_000_000_000;

fn output_counter(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .filter(|n| (0..=MAX_TIMING_OUTPUT).contains(n))
}

#[derive(Clone, Debug, Default)]
pub struct Timing {
    pub output: i64,
    pub seconds: f64,
    pub replies: usize,
    pub first_token_seconds: f64,
    pub first_token_turns: usize,
}

impl Timing {
    fn reply(&mut self, output: i64, sent: i64, finished: i64) -> bool {
        if !(0..=MAX_TIMING_OUTPUT).contains(&output) {
            return false;
        }
        let seconds = finished.saturating_sub(sent) as f64 / 1000.0;
        if output >= 50 && seconds > 0.0 && seconds <= 1200.0 {
            let Some(total) = self.output.checked_add(output) else {
                return false;
            };
            let Some(replies) = self.replies.checked_add(1) else {
                return false;
            };
            let elapsed = self.seconds + seconds;
            if !elapsed.is_finite() {
                return false;
            }
            self.output = total;
            self.seconds = elapsed;
            self.replies = replies;
        }
        true
    }
    fn merge(&mut self, other: &Self) -> bool {
        let Some(output) = self.output.checked_add(other.output) else {
            return false;
        };
        let Some(replies) = self.replies.checked_add(other.replies) else {
            return false;
        };
        let Some(first_token_turns) = self.first_token_turns.checked_add(other.first_token_turns)
        else {
            return false;
        };
        let seconds = self.seconds + other.seconds;
        let first_token_seconds = self.first_token_seconds + other.first_token_seconds;
        if !seconds.is_finite() || !first_token_seconds.is_finite() {
            return false;
        }
        self.output = output;
        self.seconds = seconds;
        self.replies = replies;
        self.first_token_seconds = first_token_seconds;
        self.first_token_turns = first_token_turns;
        true
    }
    pub fn speed(&self) -> Option<f64> {
        (self.replies >= 3 && self.seconds > 0.0).then(|| self.output as f64 / self.seconds)
    }
    pub fn first_token(&self) -> Option<f64> {
        (self.first_token_turns >= 3)
            .then(|| self.first_token_seconds / self.first_token_turns as f64)
    }
}

#[derive(Clone, Debug)]
pub struct Model {
    pub id: String,
    pub tokens: i64,
    pub share: f64,
    pub cache_hit: Option<f64>,
    pub timing: Timing,
}

pub fn models(ledger: &UsageLedger, timings: &BTreeMap<String, Timing>, days: i64) -> Vec<Model> {
    let today = chrono::Local::now().date_naive();
    let mut counts: BTreeMap<String, (i64, TokenTally)> = BTreeMap::new();
    for day in ledger
        .days
        .iter()
        .filter(|d| (0..days).contains(&today.signed_duration_since(d.date).num_days()))
    {
        for (id, tokens) in &day.models {
            let count = counts.entry(id.clone()).or_default();
            count.0 += tokens;
            count.1 += day.model_tallies.get(id).copied().unwrap_or_default();
        }
    }
    let total: i64 = counts.values().map(|c| c.0).sum();
    let mut models: Vec<_> = counts
        .into_iter()
        .filter(|(_, (tokens, _))| *tokens > 0)
        .map(|(id, (tokens, tally))| Model {
            share: if total > 0 {
                tokens as f64 / total as f64
            } else {
                0.0
            },
            cache_hit: (tally.total() == tokens
                && tally.input >= 0
                && tally.cache_write >= 0
                && tally.cache_read >= 0
                && tally.input + tally.cache_write + tally.cache_read > 0)
                .then(|| {
                    tally.cache_read as f64
                        / (tally.input + tally.cache_write + tally.cache_read) as f64
                }),
            timing: timings.get(&id).cloned().unwrap_or_default(),
            id,
            tokens,
        })
        .collect();
    models.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.id.cmp(&b.id)));
    models
}

#[derive(Debug, Default)]
pub struct TimingReport {
    pub models: BTreeMap<String, Timing>,
    /// Timing coverage only; this does not change ledger token counts.
    pub partial: bool,
}

#[derive(Clone, Copy)]
struct TimingLimits {
    depth: usize,
    entries: usize,
    files: usize,
    file_bytes: u64,
    total_bytes: u64,
    line_bytes: usize,
}
impl Default for TimingLimits {
    fn default() -> Self {
        Self {
            depth: 64,
            entries: 10_000,
            files: 1024,
            file_bytes: 8 * 1024 * 1024,
            total_bytes: 64 * 1024 * 1024,
            line_bytes: 4 * 1024 * 1024,
        }
    }
}

struct CountedFile {
    file: std::fs::File,
    used: Rc<Cell<u64>>,
}
impl Read for CountedFile {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.file.read(buffer)?;
        self.used.set(self.used.get().saturating_add(count as u64));
        Ok(count)
    }
}

pub fn read_timings(provider: Provider, root: &Path, now: i64) -> TimingReport {
    read_timings_with_limits(provider, root, now, TimingLimits::default())
}

fn read_timings_with_limits(
    provider: Provider,
    root: &Path,
    now: i64,
    limits: TimingLimits,
) -> TimingReport {
    fn collect(
        root: &Path,
        depth: usize,
        limits: TimingLimits,
        listed: &mut usize,
        paths: &mut BTreeSet<PathBuf>,
        partial: &mut bool,
    ) {
        if !crate::scan::checkpoint() || depth > limits.depth {
            *partial = true;
            return;
        }
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(error) => {
                *partial |= error.kind() != std::io::ErrorKind::NotFound;
                return;
            }
        };
        for entry in entries {
            if !crate::scan::checkpoint() || *listed >= limits.entries {
                *partial = true;
                return;
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
            if kind.is_dir() {
                collect(&entry.path(), depth + 1, limits, listed, paths, partial);
            } else if kind.is_file()
                && entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl")
            {
                if paths.len() >= limits.files {
                    *partial = true;
                    return;
                }
                paths.insert(entry.path());
            }
        }
    }

    let cutoff = now.saturating_sub(86_400_000);
    let mut report = TimingReport::default();
    let mut paths = BTreeSet::new();
    collect(root, 0, limits, &mut 0, &mut paths, &mut report.partial);
    let used = Rc::new(Cell::new(0_u64));
    for path in paths {
        if !crate::scan::checkpoint() {
            report.partial = true;
            break;
        }
        let Some(before) = std::fs::metadata(&path)
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?, m.created().ok())))
        else {
            report.partial = true;
            continue;
        };
        let modified: chrono::DateTime<chrono::Utc> = before.1.into();
        if modified.timestamp_millis() < cutoff {
            continue;
        }
        let allowed = limits
            .file_bytes
            .min(limits.total_bytes.saturating_sub(used.get()));
        if before.0 > allowed {
            report.partial = true;
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            report.partial = true;
            continue;
        };
        let mut reader = crate::scan::LineReader::with_line_limit(
            crate::zstd_stream::plain_limit(
                CountedFile {
                    file,
                    used: used.clone(),
                },
                allowed,
            ),
            limits.line_bytes,
        );
        let scanned = parse_checked(provider, reader.by_ref(), cutoff, now);
        let after = std::fs::metadata(&path)
            .ok()
            .and_then(|m| Some((m.len(), m.modified().ok()?, m.created().ok())));
        if reader.finish().is_err()
            || scanned.partial
            || after != Some(before)
            || !crate::scan::checkpoint()
        {
            report.partial = true;
            continue;
        }
        for (model, timing) in scanned.models {
            if report.models.len() >= 1024 && !report.models.contains_key(&model) {
                report.partial = true;
                continue;
            }
            report.partial |= !report.models.entry(model).or_default().merge(&timing);
        }
    }
    report
}

pub fn parse(
    provider: Provider,
    lines: impl Iterator<Item = String>,
    cutoff: i64,
) -> BTreeMap<String, Timing> {
    parse_checked(provider, lines, cutoff, i64::MAX).models
}

fn parse_checked(
    provider: Provider,
    lines: impl Iterator<Item = String>,
    cutoff: i64,
    now: i64,
) -> TimingReport {
    let mut partial = false;
    let mut rows = 0;
    let decoded = lines.take(100_001).filter_map(|line| {
        rows += 1;
        if rows > 100_000 {
            partial = true;
            return None;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => Some(value),
            Err(_) => {
                partial |= !line.trim().is_empty();
                None
            }
        }
    });
    let mut report = parse_values(provider, decoded, cutoff, now);
    report.partial |= partial;
    report
}

fn parse_values(
    provider: Provider,
    values: impl Iterator<Item = Value>,
    cutoff: i64,
    now: i64,
) -> TimingReport {
    let mut out: BTreeMap<String, Timing> = BTreeMap::new();
    let mut partial = false;
    let in_window = |at| (cutoff..=now).contains(&at);
    let mut sent = None;
    let mut parents: HashMap<String, (Option<String>, Option<i64>, bool)> = HashMap::new();
    let mut replies: HashMap<String, (String, i64, i64, i64)> = HashMap::new();
    let mut model: Option<String> = None;
    let mut previous_output = Some(0);
    let mut pending: Option<(String, i64, i64, i64)> = None;
    let mut seen_turns = std::collections::HashSet::new();
    let finish = |pending: &mut Option<(String, i64, i64, i64)>,
                  out: &mut BTreeMap<String, Timing>,
                  partial: &mut bool| {
        if let Some((model, sent, finished, output)) = pending.take() {
            if in_window(finished) {
                *partial |= !out.entry(model).or_default().reply(output, sent, finished);
            }
        }
    };
    for root in values {
        if !crate::scan::checkpoint() {
            break;
        }
        let at = root
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(crate::timeutil::parse_iso8601_ms);
        if provider == Provider::ClaudeCode {
            let kind = root.get("type").and_then(Value::as_str);
            if let Some(uuid) = root.get("uuid").and_then(Value::as_str) {
                parents.insert(
                    uuid.to_string(),
                    (
                        root.get("parentUuid")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        at,
                        kind != Some("assistant"),
                    ),
                );
            }
            if kind != Some("assistant") {
                sent = at;
                continue;
            }
            let Some(id) = root.pointer("/message/id").and_then(Value::as_str) else {
                continue;
            };
            let Some(name) = root
                .pointer("/message/model")
                .and_then(Value::as_str)
                .filter(|s| *s != "<synthetic>")
            else {
                continue;
            };
            let Some(finished) = at else {
                continue;
            };
            let mut ancestor = root
                .get("parentUuid")
                .and_then(Value::as_str)
                .map(str::to_string);
            let mut request = None;
            for _ in 0..64 {
                let Some(parent) = ancestor.as_ref().and_then(|id| parents.get(id)) else {
                    break;
                };
                if parent.2 {
                    request = parent.1;
                    break;
                }
                ancestor = parent.0.clone();
            }
            let Some(request) = request.or(sent) else {
                continue;
            };
            let Some(value) = root.pointer("/message/usage/output_tokens") else {
                continue;
            };
            let Some(output) = output_counter(value) else {
                partial = true;
                continue;
            };
            let reply = replies.entry(id.to_string()).or_insert((
                name.to_string(),
                request,
                finished,
                output,
            ));
            reply.2 = reply.2.max(finished);
            reply.3 = reply.3.max(output);
        } else if provider == Provider::Codex {
            let payload = root.get("payload").unwrap_or(&Value::Null);
            let kind = root.get("type").and_then(Value::as_str);
            let event = payload.get("type").and_then(Value::as_str);
            if kind == Some("turn_context") {
                model = payload
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            if kind == Some("session_meta") {
                model = payload
                    .pointer("/base_instructions/provenance/model")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or(model);
            }
            let input = kind == Some("response_item")
                && (matches!(
                    event,
                    Some("function_call_output" | "custom_tool_call_output")
                ) || (event == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("user")));
            if input || (kind == Some("event_msg") && event == Some("task_started")) {
                finish(&mut pending, &mut out, &mut partial);
                sent = at;
            }
            if event == Some("token_count") {
                if let Some(value) = payload.pointer("/info/total_token_usage/output_tokens") {
                    let Some(output) = output_counter(value) else {
                        partial = true;
                        pending = None;
                        previous_output = None;
                        continue;
                    };
                    let previous = previous_output.replace(output);
                    let Some(delta) = previous
                        .and_then(|old| output.checked_sub(old))
                        .filter(|n| *n >= 0)
                    else {
                        // A reset has no reliable delta. Do not guess the missing output.
                        partial = true;
                        pending = None;
                        continue;
                    };
                    if let (Some(name), Some(sent), Some(at)) = (&model, sent, at) {
                        let record = pending.get_or_insert((name.clone(), sent, at, 0));
                        record.2 = at;
                        if let Some(total) = record.3.checked_add(delta) {
                            record.3 = total;
                        } else {
                            partial = true;
                            pending = None;
                        }
                    }
                }
            }
            if event == Some("task_complete") {
                finish(&mut pending, &mut out, &mut partial);
                if let (Some(name), Some(at), Some(ms)) = (
                    &model,
                    at,
                    payload
                        .get("time_to_first_token_ms")
                        .and_then(Value::as_f64),
                ) {
                    let identity = payload
                        .get("turn_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| at.to_string());
                    if in_window(at)
                        && ms.is_finite()
                        && ms > 0.0
                        && ms <= 1_200_000.0
                        && seen_turns.insert(identity)
                    {
                        let timing = out.entry(name.clone()).or_default();
                        timing.first_token_seconds += ms / 1000.0;
                        timing.first_token_turns += 1;
                    }
                }
                sent = None;
            }
        }
    }
    finish(&mut pending, &mut out, &mut partial);
    for (_, (model, sent, finished, output)) in replies {
        if in_window(finished) {
            partial |= !out.entry(model).or_default().reply(output, sent, finished);
        }
    }
    TimingReport {
        models: out,
        partial,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TimingFixture {
        root: PathBuf,
        temp: PathBuf,
    }
    impl TimingFixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let temp = std::env::temp_dir().canonicalize().unwrap();
            let root = temp.join(format!(
                "qs-timing-{}-{}-{}",
                std::process::id(),
                crate::timeutil::now_ms(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self { root, temp }
        }
        fn write(&self, name: &str, model: &str) -> PathBuf {
            let path = self.root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut rows =
                vec![serde_json::json!({"type":"turn_context","payload":{"model":model}})];
            for turn in 0..3 {
                let at = chrono::DateTime::from_timestamp_millis(
                    crate::timeutil::now_ms() - 60_000 + turn * 10_000,
                )
                .unwrap();
                let finished = at + chrono::Duration::seconds(10);
                rows.push(serde_json::json!({"type":"event_msg","timestamp":at.to_rfc3339(),"payload":{"type":"task_started"}}));
                rows.push(serde_json::json!({"type":"event_msg","timestamp":finished.to_rfc3339(),"payload":{"type":"token_count","info":{"total_token_usage":{"output_tokens":100*(turn+1)}}}}));
                rows.push(serde_json::json!({"type":"event_msg","timestamp":finished.to_rfc3339(),"payload":{"type":"task_complete","turn_id":turn.to_string(),"time_to_first_token_ms":1000}}));
            }
            let content = rows
                .into_iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            std::fs::write(&path, content).unwrap();
            path
        }
        fn read(&self, limits: TimingLimits) -> TimingReport {
            read_timings_with_limits(
                Provider::Codex,
                &self.root,
                crate::timeutil::now_ms(),
                limits,
            )
        }
    }
    impl Drop for TimingFixture {
        fn drop(&mut self) {
            assert!(self.root.is_absolute() && self.root.parent() == Some(self.temp.as_path()));
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn bounded_timing_reader_keeps_complete_measured_replies_and_skips_old_files() {
        let fixture = TimingFixture::new();
        fixture.write("nested/current.jsonl", "current");
        let old = fixture.write("old.jsonl", "old");
        std::fs::OpenOptions::new()
            .write(true)
            .open(old)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            .unwrap();
        let report = fixture.read(TimingLimits::default());
        assert!(!report.partial);
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models["current"].replies, 3);
        assert_eq!(report.models["current"].speed(), Some(10.0));
        assert_eq!(report.models["current"].first_token(), Some(1.0));
        assert!(
            !read_timings(
                Provider::Codex,
                &fixture.root.join("missing"),
                crate::timeutil::now_ms()
            )
            .partial
        );
    }

    #[test]
    fn timing_directory_depth_entry_and_file_limits_preserve_other_files() {
        let fixture = TimingFixture::new();
        fixture.write("current.jsonl", "current");
        fixture.write("a/b/deep.jsonl", "deep");
        let report = fixture.read(TimingLimits {
            depth: 1,
            ..TimingLimits::default()
        });
        assert!(report.partial);
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models["current"].replies, 3);
        let report = fixture.read(TimingLimits {
            entries: 0,
            ..TimingLimits::default()
        });
        assert!(report.partial && report.models.is_empty());
        let report = fixture.read(TimingLimits {
            files: 1,
            ..TimingLimits::default()
        });
        assert!(report.partial);
        assert_eq!(report.models.values().map(|m| m.replies).sum::<usize>(), 3);
    }

    #[test]
    fn timing_byte_and_line_limits_keep_complete_files_without_guessing_the_rest() {
        let fixture = TimingFixture::new();
        let good = fixture.write("a-good.jsonl", "good");
        let size = std::fs::metadata(good).unwrap().len();
        std::fs::write(
            fixture.root.join("b-large.jsonl"),
            vec![b'x'; size as usize + 1],
        )
        .unwrap();
        let report = fixture.read(TimingLimits {
            file_bytes: size,
            ..TimingLimits::default()
        });
        assert!(report.partial);
        assert_eq!(report.models["good"].replies, 3);
        std::fs::remove_file(fixture.root.join("b-large.jsonl")).unwrap();
        fixture.write("c-other.jsonl", "other");
        let report = fixture.read(TimingLimits {
            total_bytes: size,
            ..TimingLimits::default()
        });
        assert!(report.partial);
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models["good"].replies, 3);
        let report = fixture.read(TimingLimits {
            line_bytes: 8,
            ..TimingLimits::default()
        });
        assert!(report.partial && report.models.is_empty());
    }

    #[test]
    fn damaged_or_excessive_timing_rows_do_not_contaminate_complete_files() {
        use std::io::Write;
        let fixture = TimingFixture::new();
        fixture.write("good.jsonl", "good");
        let broken = fixture.write("broken.jsonl", "broken");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&broken)
            .unwrap()
            .write_all(b"{torn json\n")
            .unwrap();
        let report = fixture.read(TimingLimits::default());
        assert!(report.partial);
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models["good"].replies, 3);
        std::fs::write(broken, "\n".repeat(100_001)).unwrap();
        let report = fixture.read(TimingLimits::default());
        assert!(report.partial);
        assert_eq!(report.models.len(), 1);
        let control = std::sync::Arc::new(crate::scan::Control::default());
        let stop = control.clone();
        let result = crate::scan::run(
            control,
            1,
            |_| {},
            || {
                stop.cancel();
                fixture.read(TimingLimits::default())
            },
        );
        assert!(matches!(result, Err(crate::scan::Cancelled)));
        assert_eq!(
            fixture.read(TimingLimits::default()).models["good"].replies,
            3
        );
    }

    #[test]
    fn claude_streaming_is_one_timed_reply_and_has_no_invented_first_token() {
        let entries = [
            serde_json::json!({"type":"user","uuid":"u","timestamp":"2026-10-03T00:00:00Z"}),
            serde_json::json!({"type":"assistant","parentUuid":"u","timestamp":"2026-10-03T00:00:10Z","message":{"id":"a","model":"alpha","usage":{"output_tokens":1}}}),
            serde_json::json!({"type":"assistant","parentUuid":"u","timestamp":"2026-10-03T00:00:20Z","message":{"id":"a","model":"alpha","usage":{"output_tokens":100}}}),
        ];
        let timing = parse(
            Provider::ClaudeCode,
            entries.into_iter().map(|v| v.to_string()),
            0,
        );
        assert_eq!(timing["alpha"].replies, 1);
        assert_eq!(timing["alpha"].output, 100);
        assert_eq!(timing["alpha"].seconds, 20.0);
        assert_eq!(timing["alpha"].speed(), None);
        assert_eq!(timing["alpha"].first_token(), None);
    }
    #[test]
    fn first_token_needs_three_measured_turns() {
        let entries: Vec<_> = (0..3).map(|turn| serde_json::json!({"type":"event_msg","timestamp":"2026-10-03T00:00:10Z","payload":{"type":"task_complete","turn_id":format!("{turn}"),"time_to_first_token_ms":2000}})).collect();
        let context = serde_json::json!({"type":"turn_context","payload":{"model":"gpt-6-sol"}});
        let timing = parse(
            Provider::Codex,
            std::iter::once(context)
                .chain(entries)
                .map(|v| v.to_string()),
            0,
        );
        assert_eq!(timing["gpt-6-sol"].first_token(), Some(2.0));
        assert_eq!(timing["gpt-6-sol"].speed(), None);
    }

    #[test]
    fn future_timing_records_do_not_enter_the_last_24_hours() {
        let fixture = TimingFixture::new();
        fixture.write("good.jsonl", "current");
        let future = fixture.write("future.jsonl", "future");
        let rows = std::fs::read_to_string(&future).unwrap();
        let rows = rows
            .lines()
            .map(|line| {
                let mut row: Value = serde_json::from_str(line).unwrap();
                if let Some(stamp) = row.get_mut("timestamp") {
                    let at = stamp
                        .as_str()
                        .and_then(crate::timeutil::parse_iso8601_ms)
                        .unwrap();
                    *stamp = Value::String(
                        chrono::DateTime::from_timestamp_millis(at + 86_400_000)
                            .unwrap()
                            .to_rfc3339(),
                    );
                }
                row.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(future, rows).unwrap();
        let report = fixture.read(TimingLimits::default());
        assert!(!report.partial);
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models["current"].speed(), Some(10.0));
        assert_eq!(report.models["current"].first_token(), Some(1.0));
    }

    #[test]
    fn oversized_claude_outputs_are_a_timing_gap_instead_of_an_overflow() {
        let fixture = TimingFixture::new();
        let now = crate::timeutil::now_ms();
        let mut rows = Vec::new();
        for turn in 0..3 {
            let sent =
                chrono::DateTime::from_timestamp_millis(now - 60_000 + turn * 10_000).unwrap();
            let finished = sent + chrono::Duration::seconds(5);
            rows.push(serde_json::json!({"type":"user","uuid":format!("u{turn}"),"timestamp":sent.to_rfc3339()}).to_string());
            rows.push(serde_json::json!({"type":"assistant","parentUuid":format!("u{turn}"),"timestamp":finished.to_rfc3339(),"message":{"id":format!("a{turn}"),"model":"overflow","usage":{"output_tokens":i64::MAX}}}).to_string());
        }
        std::fs::write(fixture.root.join("overflow.jsonl"), rows.join("\n")).unwrap();
        let report = read_timings(Provider::ClaudeCode, &fixture.root, now);
        assert!(report.partial);
        assert!(report.models.is_empty());
    }

    #[test]
    fn timing_window_includes_both_edges_and_excludes_one_millisecond_outside() {
        let now = crate::timeutil::parse_iso8601_ms("2026-10-04T12:00:00Z").unwrap();
        let cutoff = now - 86_400_000;
        let stamp = |at| {
            chrono::DateTime::from_timestamp_millis(at)
                .unwrap()
                .to_rfc3339()
        };
        let mut codex = vec![serde_json::json!({"type":"turn_context","payload":{"model":"edge"}})];
        let mut claude = Vec::new();
        for (index, finished) in [cutoff - 1, cutoff, now, now + 1].into_iter().enumerate() {
            for sample in 0..3 {
                let id = format!("{index}-{sample}");
                codex.push(serde_json::json!({"type":"event_msg","timestamp":stamp(finished),"payload":{"type":"task_complete","turn_id":id,"time_to_first_token_ms":1000}}));
                claude.push(
                    serde_json::json!({"type":"user","uuid":id,"timestamp":stamp(finished-5000)}),
                );
                claude.push(serde_json::json!({"type":"assistant","parentUuid":id,"timestamp":stamp(finished),"message":{"id":id,"model":"edge","usage":{"output_tokens":100}}}));
            }
        }
        let report = parse_checked(
            Provider::Codex,
            codex.into_iter().map(|v| v.to_string()),
            cutoff,
            now,
        );
        assert!(!report.partial);
        assert_eq!(report.models["edge"].first_token_turns, 6);
        assert_eq!(report.models["edge"].first_token(), Some(1.0));
        let report = parse_checked(
            Provider::ClaudeCode,
            claude.into_iter().map(|v| v.to_string()),
            cutoff,
            now,
        );
        assert!(!report.partial);
        assert_eq!(report.models["edge"].replies, 6);
        assert_eq!(report.models["edge"].output, 600);
        assert_eq!(report.models["edge"].speed(), Some(20.0));
    }

    #[test]
    fn invalid_codex_counters_exclude_that_file_and_repair_restores_coverage() {
        let fixture = TimingFixture::new();
        fixture.write("a-good.jsonl", "good");
        let bad = fixture.write("b-invalid.jsonl", "invalid");
        let original = std::fs::read_to_string(&bad).unwrap();
        for invalid in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("100"),
            Value::Null,
            serde_json::json!(MAX_TIMING_OUTPUT + 1),
            serde_json::json!(u64::MAX),
        ] {
            let mut changed = false;
            let rows = original
                .lines()
                .map(|line| {
                    let mut row: Value = serde_json::from_str(line).unwrap();
                    if !changed {
                        if let Some(counter) =
                            row.pointer_mut("/payload/info/total_token_usage/output_tokens")
                        {
                            *counter = invalid.clone();
                            changed = true;
                        }
                    }
                    row.to_string()
                })
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&bad, rows).unwrap();
            let report = fixture.read(TimingLimits::default());
            assert!(report.partial, "{invalid}");
            assert_eq!(report.models.len(), 1);
            assert_eq!(report.models["good"].speed(), Some(10.0));
        }
        std::fs::write(&bad, &original).unwrap();
        let report = fixture.read(TimingLimits::default());
        assert!(!report.partial);
        assert_eq!(report.models["invalid"].speed(), Some(10.0));
    }

    #[test]
    fn codex_counter_reset_is_unknown_but_zero_and_the_read_limit_are_valid() {
        let now = crate::timeutil::parse_iso8601_ms("2026-10-04T12:00:00Z").unwrap();
        let stamp = |at| {
            chrono::DateTime::from_timestamp_millis(at)
                .unwrap()
                .to_rfc3339()
        };
        let rows = |counters: &[i64]| {
            let mut rows = vec![
                serde_json::json!({"type":"turn_context","payload":{"model":"edge"}}),
                serde_json::json!({"type":"event_msg","timestamp":stamp(now-10000),"payload":{"type":"task_started"}}),
            ];
            for &counter in counters {
                rows.push(serde_json::json!({"type":"event_msg","timestamp":stamp(now),"payload":{"type":"token_count","info":{"total_token_usage":{"output_tokens":counter}}}}));
            }
            rows.into_iter().map(|v| v.to_string())
        };
        let report = parse_checked(Provider::Codex, rows(&[100, 0, 50]), 0, now);
        assert!(report.partial);
        let report = parse_checked(Provider::Codex, rows(&[0, MAX_TIMING_OUTPUT]), 0, now);
        assert!(!report.partial);
        assert_eq!(report.models["edge"].output, MAX_TIMING_OUTPUT);
        assert_eq!(report.models["edge"].replies, 1);
        assert_eq!(report.models["edge"].speed(), None);
    }

    #[test]
    fn timing_aggregate_overflow_leaves_the_complete_snapshot_unchanged() {
        let mut timing = Timing {
            output: i64::MAX - 5,
            seconds: 30.0,
            replies: 3,
            first_token_seconds: 3.0,
            first_token_turns: 3,
        };
        let original = timing.clone();
        assert!(!timing.merge(&Timing {
            output: 10,
            seconds: 10.0,
            replies: 1,
            first_token_seconds: 1.0,
            first_token_turns: 1
        }));
        assert!(!timing.reply(50, 0, 5000));
        assert_eq!(timing.output, original.output);
        assert_eq!(timing.seconds, original.seconds);
        assert_eq!(timing.replies, original.replies);
        assert_eq!(timing.first_token_seconds, original.first_token_seconds);
        assert_eq!(timing.first_token_turns, original.first_token_turns);
        assert!(timing.merge(&Timing {
            output: 5,
            ..Timing::default()
        }));
        assert_eq!(timing.output, i64::MAX);
        let mut turns = Timing {
            first_token_turns: usize::MAX,
            ..Timing::default()
        };
        assert!(!turns.merge(&Timing {
            output: 100,
            first_token_turns: 1,
            ..Timing::default()
        }));
        assert_eq!(turns.output, 0);
        let mut seconds = Timing {
            seconds: f64::MAX,
            ..Timing::default()
        };
        assert!(!seconds.merge(&Timing {
            output: 100,
            seconds: f64::MAX,
            ..Timing::default()
        }));
        assert_eq!(seconds.output, 0);
    }
}
