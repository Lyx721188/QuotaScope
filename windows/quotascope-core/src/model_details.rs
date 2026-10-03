//! Account model shares and measured recent response timings. Missing timing
//! stays unavailable; request-to-finish includes first-token wait.
use crate::ledger::{TokenTally, UsageLedger};
use crate::model::Provider;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Timing {
    pub output: i64,
    pub seconds: f64,
    pub replies: usize,
    pub first_token_seconds: f64,
    pub first_token_turns: usize,
}

impl Timing {
    fn reply(&mut self, output: i64, sent: i64, finished: i64) {
        let seconds = finished.saturating_sub(sent) as f64 / 1000.0;
        if output >= 50 && seconds > 0.0 && seconds <= 1200.0 {
            self.output += output;
            self.seconds += seconds;
            self.replies += 1;
        }
    }
    fn merge(&mut self, other: &Self) {
        self.output += other.output;
        self.seconds += other.seconds;
        self.replies += other.replies;
        self.first_token_seconds += other.first_token_seconds;
        self.first_token_turns += other.first_token_turns;
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

pub fn read_timings(provider: Provider, root: &Path, now: i64) -> BTreeMap<String, Timing> {
    fn visit(provider: Provider, root: &Path, cutoff: i64, out: &mut BTreeMap<String, Timing>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            if !crate::scan::checkpoint() {
                return;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                visit(provider, &entry.path(), cutoff, out);
            } else if kind.is_file()
                && entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl")
            {
                let modified = entry
                    .metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|t| t.as_millis() as i64);
                if !modified.is_some_and(|at| at >= cutoff) {
                    continue;
                }
                if let Ok(file) = std::fs::File::open(entry.path()) {
                    let mut reader = crate::scan::LineReader::new(file, Default::default(), 0);
                    let scanned = parse(provider, reader.by_ref(), cutoff);
                    if reader.finish().is_ok() {
                        for (model, timing) in scanned {
                            out.entry(model).or_default().merge(&timing);
                        }
                    }
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    visit(provider, root, now.saturating_sub(86_400_000), &mut out);
    out
}

pub fn parse(
    provider: Provider,
    lines: impl Iterator<Item = String>,
    cutoff: i64,
) -> BTreeMap<String, Timing> {
    let mut out: BTreeMap<String, Timing> = BTreeMap::new();
    let mut sent = None;
    let mut parents: HashMap<String, (Option<String>, Option<i64>, bool)> = HashMap::new();
    let mut replies: HashMap<String, (String, i64, i64, i64)> = HashMap::new();
    let mut model: Option<String> = None;
    let mut previous_output = 0;
    let mut pending: Option<(String, i64, i64, i64)> = None;
    let mut seen_turns = std::collections::HashSet::new();
    let finish = |pending: &mut Option<(String, i64, i64, i64)>,
                  out: &mut BTreeMap<String, Timing>| {
        if let Some((model, sent, finished, output)) = pending.take() {
            if finished >= cutoff {
                out.entry(model).or_default().reply(output, sent, finished);
            }
        }
    };
    for line in lines {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(root) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
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
            let output = root
                .pointer("/message/usage/output_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0);
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
                finish(&mut pending, &mut out);
                sent = at;
            }
            if event == Some("token_count") {
                let output = payload
                    .pointer("/info/total_token_usage/output_tokens")
                    .and_then(Value::as_i64);
                if let Some(output) = output {
                    let delta = output.saturating_sub(previous_output).max(0);
                    previous_output = output;
                    if let (Some(name), Some(sent), Some(at)) = (&model, sent, at) {
                        let record = pending.get_or_insert((name.clone(), sent, at, 0));
                        record.2 = at;
                        record.3 += delta;
                    }
                }
            }
            if event == Some("task_complete") {
                finish(&mut pending, &mut out);
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
                    if at >= cutoff
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
    finish(&mut pending, &mut out);
    for (_, (model, sent, finished, output)) in replies {
        if finished >= cutoff {
            out.entry(model).or_default().reply(output, sent, finished);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
