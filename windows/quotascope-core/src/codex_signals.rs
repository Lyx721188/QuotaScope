//! Passive request-side facts from Codex rollouts. These records do not name
//! the model the server actually executed. The lattice rule is a heuristic.
//! Adapted from Pulse 3696a65, Copyright (c) 2026 qunqin24, Apache-2.0.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const ALGORITHM: &str = "codex-signals-v1";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub responses: u64,
    pub reached: u64,
    pub hits: u64,
}
impl Counts {
    pub fn measurable(&self) -> bool {
        self.reached >= 20
    }
    pub fn concentrated(&self) -> bool {
        self.measurable() && self.hits >= 5 && self.hits as f64 / self.reached as f64 >= 0.05
    }
    pub fn share(&self) -> Option<f64> {
        (self.reached > 0).then(|| self.hits as f64 / self.reached as f64)
    }
    fn add(&mut self, reasoning: i64) {
        self.responses += 1;
        self.reached += u64::from(reasoning >= 516);
        self.hits += u64::from(on_lattice(reasoning));
    }
}
pub fn on_lattice(reasoning: i64) -> bool {
    reasoning >= 516 && reasoning % 518 == 516
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Model { asked: String, recorded: String },
    Effort { asked: String, recorded: String },
    Context { previous: i64, recorded: i64 },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub at: i64,
    pub order: usize,
    pub kind: Kind,
}

#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// Calendar dates compact repeated responses, instead of retaining text
    /// or one allocation per response. Filtering days reuses the same facts.
    pub days: BTreeMap<chrono::NaiveDate, BTreeMap<String, Counts>>,
    pub changes: Vec<Change>,
    pub last_at: Option<i64>,
    pub judged: bool,
    pub partial: bool,
}
#[derive(Clone, Default)]
struct Settings {
    model: Option<String>,
    effort: Option<String>,
}
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .map(str::to_string)
}
fn date(value: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|d| d.timestamp_millis())
}
fn version(value: &Value) -> Option<(u32, u32, u32)> {
    let mut parts = value.as_str()?.split('-').next()?.split('.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}
fn effort(value: &str) -> Option<usize> {
    // Unknown efforts, including max/ultra, have no invented rank.
    ["none", "minimal", "low", "medium", "high", "xhigh"]
        .iter()
        .position(|known| *known == value)
}
fn own_model(model: &str) -> bool {
    model.contains("auto-review")
}

pub fn parse(lines: impl Iterator<Item = String>) -> Facts {
    let mut facts = Facts::default();
    let mut header_seen = false;
    let mut own_session = false;
    let mut replay_until = None;
    let mut model = None;
    let mut applied = Settings::default();
    let mut at_start = Settings::default();
    let mut compared_any = false;
    let mut helper_turns = BTreeSet::new();
    let mut active_helper = false;
    let mut last_totals = None;
    let mut last_context = None;
    let mut window: Option<(i64, Option<String>)> = None;
    for line in lines {
        if !crate::scan::checkpoint() {
            facts.partial = true;
            break;
        }
        if ![
            "\"session_meta\"",
            "\"turn_context\"",
            "\"thread_settings_applied\"",
            "\"task_started\"",
            "\"token_count\"",
        ]
        .iter()
        .any(|wanted| line.contains(wanted))
        {
            continue;
        }
        let Ok(root) = serde_json::from_str::<Value>(&line) else {
            facts.partial = true;
            continue;
        };
        let payload = &root["payload"];
        let at = date(&root["timestamp"]);
        if let Some(at) = at {
            facts.last_at = Some(facts.last_at.map_or(at, |old| old.max(at)));
        }
        let kind = root["type"].as_str();
        if kind != Some("session_meta")
            && replay_until.is_some_and(|end| at.is_none_or(|at| at <= end))
        {
            continue;
        }
        match kind {
            Some("session_meta") => {
                window = None;
                at_start = Settings::default();
                last_context = None;
                if header_seen {
                    continue;
                }
                header_seen = true;
                facts.judged = version(&payload["cli_version"]).is_some_and(|v| v >= (0, 144, 0));
                own_session =
                    payload["source"].is_object() || payload["parent_thread_id"].is_string();
                if payload["forked_from_id"].is_string() {
                    if let Some(at) = at {
                        replay_until = at.checked_add(2000);
                    } else {
                        own_session = true;
                        facts.partial = true;
                    }
                }
            }
            Some("turn_context") => {
                let recorded_model = text(&payload["model"]);
                let recorded_effort = text(&payload["effort"]).or_else(|| {
                    text(&payload["collaboration_mode"]["settings"]["reasoning_effort"])
                });
                let context = (
                    text(&payload["turn_id"]),
                    recorded_model.clone(),
                    recorded_effort.clone(),
                );
                if last_context.as_ref() == Some(&context) {
                    continue;
                }
                last_context = Some(context);
                active_helper |=
                    text(&payload["turn_id"]).is_some_and(|id| helper_turns.contains(&id));
                if let Some(recorded) = &recorded_model {
                    model = Some(recorded.clone());
                }
                if active_helper || own_session {
                    continue;
                }
                // An observed previous turn is not an authoritative user
                // choice. Only settings already applied at task start may
                // be compared, including when the first applied event is late.
                let asked_model = at_start.model.as_ref();
                let asked_effort = at_start.effort.as_ref();
                if facts.judged {
                    if let (Some(asked), Some(recorded), Some(at)) =
                        (asked_model, recorded_model.as_ref(), at)
                    {
                        compared_any |= !own_model(asked) && !own_model(recorded);
                        if asked != recorded && !own_model(asked) && !own_model(recorded) {
                            facts.changes.push(Change {
                                at,
                                order: facts.changes.len(),
                                kind: Kind::Model {
                                    asked: asked.clone(),
                                    recorded: recorded.clone(),
                                },
                            });
                        }
                    }
                    if let (Some(asked), Some(recorded), Some(at)) =
                        (asked_effort, recorded_effort.as_ref(), at)
                    {
                        compared_any |= model.as_deref().is_some_and(|m| !own_model(m))
                            && effort(asked).is_some()
                            && effort(recorded).is_some();
                        if model.as_deref().is_some_and(|m| !own_model(m))
                            && effort(asked)
                                .zip(effort(recorded))
                                .is_some_and(|(asked, recorded)| recorded < asked)
                        {
                            facts.changes.push(Change {
                                at,
                                order: facts.changes.len(),
                                kind: Kind::Effort {
                                    asked: asked.clone(),
                                    recorded: recorded.clone(),
                                },
                            });
                        }
                    }
                }
            }
            Some("event_msg") => match payload["type"].as_str() {
                Some("task_started") => {
                    last_context = None;
                    model = None;
                    active_helper = text(&payload["turn_id"])
                        .zip(text(&payload["root_turn_id"]))
                        .is_some_and(|(id, root)| {
                            if id != root {
                                helper_turns.insert(id);
                                true
                            } else {
                                false
                            }
                        });
                    at_start = applied.clone();
                }
                Some("thread_settings_applied") => {
                    if active_helper || own_session {
                        continue;
                    }
                    let settings = &payload["thread_settings"];
                    if settings.get("model").is_some() {
                        applied.model = text(&settings["model"]);
                    }
                    if settings.get("reasoning_effort").is_some() {
                        applied.effort = text(&settings["reasoning_effort"]);
                    }
                }
                Some("token_count") => {
                    if active_helper || own_session || model.as_deref().is_some_and(own_model) {
                        continue;
                    }
                    let info = &payload["info"];
                    if facts.judged && at_start.model.is_some() {
                        if let Some(size) = info["model_context_window"].as_i64().filter(|n| *n > 0)
                        {
                            if let (Some((old, old_model)), Some(at)) = (&window, at) {
                                if size < *old && old_model == &model && model.is_some() {
                                    facts.changes.push(Change {
                                        at,
                                        order: facts.changes.len(),
                                        kind: Kind::Context {
                                            previous: *old,
                                            recorded: size,
                                        },
                                    });
                                }
                            }
                            window = Some((size, model.clone()));
                        }
                    }
                    // Codex emits initial context/rate-limit messages with
                    // no completed usage. They are not missing responses.
                    if info["last_token_usage"].is_null() {
                        continue;
                    }
                    let totals = info["total_token_usage"]
                        .as_object()
                        .map(|usage| {
                            [
                                "input_tokens",
                                "output_tokens",
                                "reasoning_output_tokens",
                                "total_tokens",
                            ]
                            .map(|key| usage.get(key).and_then(Value::as_i64).filter(|n| *n >= 0))
                        })
                        .filter(|totals| totals.iter().any(Option::is_some));
                    if let Some(totals) = totals {
                        if last_totals == Some(totals) {
                            continue;
                        }
                        last_totals = Some(totals);
                    } else {
                        // A response without a stable running-count identity
                        // cannot safely be distinguished from a replay.
                        facts.partial = true;
                        continue;
                    }
                    let reasoning = info["last_token_usage"]["reasoning_output_tokens"].as_i64();
                    if let (Some(reasoning), Some(model), Some(at)) =
                        (reasoning, model.as_ref(), at)
                    {
                        if reasoning > 0 {
                            if let Some(day) = chrono::DateTime::from_timestamp_millis(at)
                                .map(|d| d.with_timezone(&chrono::Local).date_naive())
                            {
                                facts
                                    .days
                                    .entry(day)
                                    .or_default()
                                    .entry(model.clone())
                                    .or_default()
                                    .add(reasoning);
                            }
                        } else if reasoning < 0 {
                            facts.partial = true;
                        }
                    } else {
                        facts.partial = true;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    facts.judged &= compared_any && !own_session;
    if !facts.judged {
        facts.changes.clear();
    }
    facts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn line(kind: &str, payload: Value, second: i64) -> String {
        json!({"timestamp": chrono::DateTime::from_timestamp(1_790_000_000 + second, 0).unwrap().to_rfc3339(), "type":kind, "payload":payload}).to_string()
    }
    fn meta(ver: &str) -> String {
        line("session_meta", json!({"cli_version":ver}), 0)
    }
    fn start(id: &str, root: &str, second: i64) -> String {
        line(
            "event_msg",
            json!({"type":"task_started","turn_id":id,"root_turn_id":root}),
            second,
        )
    }
    fn applied(model: &str, effort: &str, second: i64) -> String {
        line(
            "event_msg",
            json!({"type":"thread_settings_applied","thread_settings":{"model":model,"reasoning_effort":effort}}),
            second,
        )
    }
    fn context(id: &str, model: &str, effort: &str, second: i64) -> String {
        line(
            "turn_context",
            json!({"turn_id":id,"model":model,"effort":effort}),
            second,
        )
    }
    fn count(reasoning: Option<i64>, total: i64, window: i64, second: i64) -> String {
        line(
            "event_msg",
            json!({"type":"token_count","info":{"model_context_window":window,"last_token_usage":{"reasoning_output_tokens":reasoning},"total_token_usage":{"input_tokens":total,"output_tokens":total,"reasoning_output_tokens":total,"total_tokens":total}}}),
            second,
        )
    }
    fn read(lines: Vec<String>) -> Facts {
        parse(lines.into_iter())
    }
    fn response_count(facts: &Facts) -> u64 {
        facts
            .days
            .values()
            .flat_map(|day| day.values())
            .map(|counts| counts.responses)
            .sum()
    }

    #[test]
    fn initial_null_usage_is_not_an_incomplete_response_or_a_zero_sample() {
        let facts = read(vec![
            meta("0.146.0"),
            context("root", "model-a", "high", 1),
            line(
                "event_msg",
                json!({"type":"token_count","info":{"last_token_usage":null,"total_token_usage":null,"model_context_window":10000}}),
                2,
            ),
            count(Some(516), 100, 10000, 3),
        ]);
        assert!(!facts.partial);
        assert_eq!(response_count(&facts), 1);
    }

    #[test]
    fn lattice_and_denominator_boundaries_are_explicit_and_overflow_safe() {
        for n in [516, 1034, 1552] {
            assert!(on_lattice(n));
        }
        for n in [-1, 0, 514, 517, 1036, i64::MAX] {
            assert!(!on_lattice(n));
        }
        assert!(!Counts {
            responses: 100,
            reached: 19,
            hits: 19
        }
        .concentrated());
        assert!(!Counts {
            responses: 100,
            reached: 20,
            hits: 4
        }
        .concentrated());
        assert!(Counts {
            responses: 100,
            reached: 100,
            hits: 5
        }
        .concentrated());
        assert!(!Counts {
            responses: 200,
            reached: 101,
            hits: 5
        }
        .concentrated());
        assert_eq!(Counts::default().share(), None);
    }

    #[test]
    fn responses_use_recorded_models_and_repeated_totals_do_not_count_twice() {
        let facts = read(vec![
            meta("0.120.0"),
            context("a", "model-a", "high", 1),
            count(Some(516), 100, 272000, 2),
            count(Some(516), 100, 272000, 3),
            count(Some(0), 150, 272000, 4),
            context("b", "model-b", "high", 5),
            count(Some(700), 200, 272000, 6),
        ]);
        assert_eq!(response_count(&facts), 2);
        let counts: Vec<_> = facts.days.values().flat_map(|d| d.values()).collect();
        assert_eq!(counts.iter().map(|c| c.reached).sum::<u64>(), 2);
        assert_eq!(counts.iter().map(|c| c.hits).sum::<u64>(), 1);
        assert!(!facts.judged && facts.changes.is_empty());
    }

    #[test]
    fn user_changes_and_mid_turn_settings_are_not_retroactive() {
        let facts = read(vec![
            meta("0.146.0-alpha.3"),
            applied("model-a", "high", 0),
            start("a", "a", 1),
            context("a", "model-a", "high", 1),
            applied("model-b", "medium", 2),
            context("a", "model-a", "high", 3),
            start("b", "b", 4),
            context("b", "model-b", "medium", 4),
        ]);
        assert!(facts.judged);
        assert!(facts.changes.is_empty());
    }

    #[test]
    fn late_first_settings_and_explicitly_cleared_choices_do_not_judge_unanchored_turns() {
        let facts = read(vec![
            meta("0.146.0"),
            start("a", "a", 1),
            context("a", "model-a", "high", 1),
            start("b", "b", 2),
            context("b", "model-b", "low", 2),
            applied("model-b", "low", 3),
            start("c", "c", 4),
            context("c", "model-b", "low", 4),
            line(
                "event_msg",
                json!({"type":"thread_settings_applied","thread_settings":{"model":null,"reasoning_effort":null}}),
                5,
            ),
            start("d", "d", 6),
            context("d", "model-c", "none", 6),
        ]);
        assert!(facts.judged);
        assert!(facts.changes.is_empty());
        let missing_start = read(vec![
            meta("0.146.0"),
            applied("model-a", "high", 0),
            context("a", "model-b", "low", 1),
        ]);
        assert!(!missing_start.judged && missing_start.changes.is_empty());
    }

    #[test]
    fn unexpected_model_and_known_lower_effort_are_recorded_once_per_context() {
        let facts = read(vec![
            meta("0.146.0"),
            applied("model-a", "high", 0),
            start("a", "a", 1),
            context("a", "model-b", "medium", 2),
            context("a", "model-b", "medium", 3),
        ]);
        assert_eq!(facts.changes.len(), 2);
        assert!(
            matches!(&facts.changes[0].kind,Kind::Model{asked,recorded} if asked=="model-a" && recorded=="model-b")
        );
        assert!(
            matches!(&facts.changes[1].kind,Kind::Effort{asked,recorded} if asked=="high" && recorded=="medium")
        );
    }

    #[test]
    fn unknown_efforts_and_unsupported_sessions_are_not_ranked_or_judged() {
        for (asked, recorded) in [("ultra", "high"), ("max", "low"), ("high", "future-effort")] {
            let facts = read(vec![
                meta("0.146.0"),
                applied("model-a", asked, 0),
                start("a", "a", 1),
                context("a", "model-a", recorded, 2),
            ]);
            assert!(facts.changes.is_empty());
        }
        for header in [meta("0.119.0"), meta("dev"), meta("0.146.0")] {
            let facts = read(vec![
                header,
                start("a", "a", 1),
                context("a", "model-a", "high", 1),
                start("b", "b", 2),
                context("b", "model-b", "low", 2),
            ]);
            assert!(!facts.judged && facts.changes.is_empty());
        }
    }

    #[test]
    fn helpers_reviewers_and_missing_contexts_do_not_pollute_main_response_samples() {
        let facts = read(vec![
            meta("0.146.0"),
            applied("model-a", "high", 0),
            start("helper", "root", 1),
            context("helper", "small", "low", 1),
            count(Some(516), 100, 128000, 2),
            start("root", "root", 3),
            count(Some(516), 150, 128000, 3),
            context("root", "model-a", "high", 4),
            count(Some(700), 200, 272000, 5),
            start("review", "review", 6),
            context("review", "model-auto-review", "low", 6),
            count(Some(516), 300, 128000, 7),
        ]);
        assert_eq!(response_count(&facts), 1);
        assert!(facts.changes.is_empty());
        let helper = read(vec![
            line(
                "session_meta",
                json!({"cli_version":"0.146.0","source":{"subagent":{}}}),
                0,
            ),
            applied("model-a", "high", 0),
            start("a", "a", 1),
            context("a", "model-b", "low", 1),
            count(Some(516), 100, 128000, 2),
        ]);
        assert_eq!(response_count(&helper), 0);
        assert!(!helper.judged);
    }

    #[test]
    fn context_window_requires_same_known_model_and_resumed_headers_reset_comparison() {
        let facts = read(vec![
            meta("0.146.0"),
            applied("model-a", "high", 0),
            start("a", "a", 1),
            context("a", "model-a", "high", 1),
            count(Some(10), 10, 272000, 2),
            count(Some(10), 20, 128000, 3),
            meta("0.146.0"),
            count(Some(10), 30, 64000, 4),
        ]);
        assert_eq!(facts.changes.len(), 1);
        assert_eq!(
            facts.changes[0].kind,
            Kind::Context {
                previous: 272000,
                recorded: 128000
            }
        );
    }

    #[test]
    fn fork_replay_and_unidentifiable_counts_are_excluded_without_inventing_zero() {
        let facts = read(vec![
            line(
                "session_meta",
                json!({"cli_version":"0.146.0","forked_from_id":"parent"}),
                100,
            ),
            context("old", "model-a", "high", 100),
            count(Some(516), 100, 272000, 101),
            applied("model-b", "high", 159),
            start("new", "new", 160),
            context("new", "model-b", "high", 160),
            count(None, 150, 272000, 161),
            count(Some(700), 200, 272000, 170),
        ]);
        assert_eq!(response_count(&facts), 1);
        assert!(facts.partial && facts.judged && facts.changes.is_empty());
        let bad = read(vec![
            meta("0.146.0"),
            context("a", "model-a", "high", 1),
            line(
                "event_msg",
                json!({"type":"token_count","info":{"last_token_usage":{"reasoning_output_tokens":516},"total_token_usage":{}}}),
                2,
            ),
        ]);
        assert_eq!(response_count(&bad), 0);
        assert!(bad.partial);
    }
}
