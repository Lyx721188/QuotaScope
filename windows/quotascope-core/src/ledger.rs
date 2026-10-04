//! A provider's history, worked out from the local logs and databases its
//! client leaves on this machine — ported from `UsageLedger.swift`.
//!
//! Worth being clear about what this is and isn't. The providers report
//! *limits*, not spending, and neither publishes a per-day history — so the
//! only place the day-by-day story exists is the transcripts on disk. That
//! makes this local by nature: work done on another machine isn't here, and
//! neither is anything the CLI has since pruned.
//!
//! The money is likewise a translation, not a bill. Both tools are used on a
//! subscription, so nothing here was charged per token; the figure is what
//! the same tokens would cost at the providers' published API rates, which is
//! the only defensible way to put a number on it.
//!
//! Scanning is kept off the price list on purpose: the file cache holds
//! *tokens per model per quarter-hour*, and money is worked out afterwards. A
//! price change then costs nothing to apply, where caching the money would
//! have meant rescanning a few hundred megabytes to pick it up.

use crate::model::Provider;
use crate::model_prices::{self, ModelPrice};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Tokens of each kind, which is what a price list needs to become money.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTally {
    /// Fresh input — what wasn't served from the prompt cache.
    #[serde(default)]
    pub input: i64,
    #[serde(default)]
    pub cache_write: i64,
    #[serde(default)]
    pub cache_read: i64,
    #[serde(default)]
    pub output: i64,
}

impl TokenTally {
    pub fn total(&self) -> i64 {
        self.input + self.cache_write + self.cache_read + self.output
    }

    /// Rates are per million tokens. A missing cache rate falls back to the
    /// plain input rate — that is the provider's own arrangement for models
    /// that don't price the cache separately, not a guess.
    ///
    /// **The split is the only formula.** A day's money and the per-model
    /// money it is built from come out of one arithmetic instead of two that
    /// would eventually disagree.
    pub fn cost_breakdown(&self, price: &ModelPrice) -> TokenCost {
        TokenCost {
            input: self.input as f64 * price.input / 1_000_000.0,
            cache_write: self.cache_write as f64 * price.cache_write.unwrap_or(price.input)
                / 1_000_000.0,
            cache_read: self.cache_read as f64 * price.cache_read.unwrap_or(price.input)
                / 1_000_000.0,
            output: self.output as f64 * price.output / 1_000_000.0,
        }
    }

    pub fn cost(&self, price: &ModelPrice) -> f64 {
        self.cost_breakdown(price).total()
    }
}

impl std::ops::Add for TokenTally {
    type Output = TokenTally;

    fn add(self, rhs: TokenTally) -> TokenTally {
        TokenTally {
            input: self.input + rhs.input,
            cache_write: self.cache_write + rhs.cache_write,
            cache_read: self.cache_read + rhs.cache_read,
            output: self.output + rhs.output,
        }
    }
}

impl std::ops::AddAssign for TokenTally {
    fn add_assign(&mut self, rhs: TokenTally) {
        self.input += rhs.input;
        self.cache_write += rhs.cache_write;
        self.cache_read += rhs.cache_read;
        self.output += rhs.output;
    }
}

/// Money by kind of token, at one model's own rates. The split is kept rather
/// than only a total because a model's input, cache and output rates differ by
/// an order of magnitude.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct TokenCost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub cache_write: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub output: f64,
}

impl TokenCost {
    pub fn total(&self) -> f64 {
        self.input + self.cache_write + self.cache_read + self.output
    }
}

impl std::ops::Add for TokenCost {
    type Output = TokenCost;

    fn add(self, rhs: TokenCost) -> TokenCost {
        TokenCost {
            input: self.input + rhs.input,
            cache_write: self.cache_write + rhs.cache_write,
            cache_read: self.cache_read + rhs.cache_read,
            output: self.output + rhs.output,
        }
    }
}

impl std::ops::AddAssign for TokenCost {
    fn add_assign(&mut self, rhs: TokenCost) {
        self.input += rhs.input;
        self.cache_write += rhs.cache_write;
        self.cache_read += rhs.cache_read;
        self.output += rhs.output;
    }
}

/// A quarter of an hour's work. Days are what the card shows, but a five-hour
/// limit opens and closes inside one, so the totals are kept at a resolution
/// fine enough to answer "since this window opened".
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    /// Epoch milliseconds of the quarter-hour's start.
    pub start_ms: i64,
    pub tokens: i64,
    pub cost: f64,
    pub unpriced_tokens: i64,
    /// The quarter-hour's tokens split by raw model id, where the reader kept
    /// them.
    pub models: BTreeMap<String, TokenTally>,
    /// The same split in money: what each model contributed to `cost`. These
    /// sum to `cost`, and a model with no published rate is absent from it
    /// exactly as its tokens are absent from `cost`. A window scoped to one
    /// model group is priced from this and never from `cost` itself.
    pub costs: BTreeMap<String, f64>,
}

/// One day's work, priced.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerDay {
    /// The local calendar day, at midnight.
    pub date: NaiveDate,
    pub tokens: i64,
    pub cost: f64,
    /// Tokens spent on models with no published price. They count towards
    /// `tokens` but not `cost`, so the two can be read honestly side by side.
    pub unpriced_tokens: i64,
    /// Tokens by model, so "which model is doing the work" can be answered
    /// over any span rather than only the one totalled at scan time.
    pub models: BTreeMap<String, i64>,
    /// The same day split by kind of token — fresh input, cache written, cache
    /// read, output. It is what separates "I sent a lot" from "I re-read a
    /// lot", which are priced an order of magnitude apart.
    pub tally: TokenTally,
    /// The same day split by raw model id, each id with its own tally.
    pub model_tallies: BTreeMap<String, TokenTally>,
    /// The same day split by raw model id, each id with what its own tokens
    /// cost at that model's own rates. A model absent here is one with no
    /// published price — counted, never priced, never patched with a zero.
    pub model_costs: BTreeMap<String, TokenCost>,
}

/// The transcript ledger for one provider: days for the card, quarter-hours
/// for the value estimate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageLedger {
    /// Ascending by date, gaps closed so the chart reads as a calendar.
    pub days: Vec<LedgerDay>,
    pub earliest: Option<NaiveDate>,
    /// Models seen in the logs that models.dev has no price for.
    pub unpriced_models: Vec<String>,
    /// How each model id is written by its provider, where the price table
    /// says.
    pub model_names: BTreeMap<String, String>,
    /// Ascending by start time. Only slots with work in them.
    pub slots: Vec<Slot>,
    /// At least one source omits token fields whose meaning is not established.
    pub has_partial_records: bool,
}

impl UsageLedger {
    pub fn cache_hit_rate_calendar(&self, days: i64, today: NaiveDate) -> Option<f64> {
        if days <= 0 {
            return None;
        }
        let start = today.checked_sub_days(chrono::Days::new((days - 1) as u64))?;
        let selected: Vec<_> = self
            .days
            .iter()
            .filter(|d| d.date >= start && d.date <= today)
            .collect();
        let tokens = selected
            .iter()
            .try_fold(0_i64, |sum, d| sum.checked_add(d.tokens))?;
        let tally = selected.iter().try_fold(TokenTally::default(), |sum, d| {
            Some(TokenTally {
                input: sum.input.checked_add(d.tally.input)?,
                cache_write: sum.cache_write.checked_add(d.tally.cache_write)?,
                cache_read: sum.cache_read.checked_add(d.tally.cache_read)?,
                output: sum.output.checked_add(d.tally.output)?,
            })
        })?;
        let input = tally
            .input
            .checked_add(tally.cache_write)?
            .checked_add(tally.cache_read)?;
        if input <= 0
            || tally.input < 0
            || tally.cache_write < 0
            || tally.cache_read < 0
            || tally.output < 0
            || input.checked_add(tally.output)? != tokens
        {
            return None;
        }
        Some(tally.cache_read as f64 / input as f64)
    }

    pub fn empty() -> UsageLedger {
        UsageLedger::default()
    }

    /// What has gone through since a moment — the figure a rate-limit window
    /// needs. A slot straddling the boundary counts in full, so this can run a
    /// few minutes' work high; at fifteen-minute steps that is well inside the
    /// rounding the providers' own percentages carry.
    pub fn spend_since(&self, start_ms: i64) -> (i64, f64) {
        let mut tokens = 0;
        let mut cost = 0.0;
        for slot in &self.slots {
            if slot.start_ms < start_ms {
                continue;
            }
            tokens += slot.tokens;
            cost += slot.cost;
        }
        (tokens, cost)
    }

    /// Prorate the two boundary quarter-hours; never include work after the
    /// quota was observed. Timing inside a bucket is an approximation.
    pub fn cost_between(&self, start_ms: i64, end_ms: i64) -> f64 {
        self.slots
            .iter()
            .map(|s| s.cost * slot_share(s.start_ms, start_ms, end_ms))
            .sum()
    }

    pub fn tokens_between(&self, start_ms: i64, end_ms: i64) -> f64 {
        self.slots
            .iter()
            .map(|s| s.tokens as f64 * slot_share(s.start_ms, start_ms, end_ms))
            .sum()
    }

    pub fn today(&self) -> Option<&LedgerDay> {
        let today = Local::now().date_naive();
        self.days.last().filter(|day| day.date == today)
    }

    pub fn total_over_last(&self, count: usize) -> (i64, f64) {
        self.days
            .iter()
            .rev()
            .take(count)
            .fold((0, 0.0), |(tokens, cost), day| {
                (tokens + day.tokens, cost + day.cost)
            })
    }

    pub fn all_time(&self) -> (i64, f64) {
        self.days.iter().fold((0, 0.0), |(tokens, cost), day| {
            (tokens + day.tokens, cost + day.cost)
        })
    }

    pub fn recent(&self, count: usize) -> &[LedgerDay] {
        let from = self.days.len().saturating_sub(count);
        &self.days[from..]
    }

    /// How much of the input over a span was served from the prompt cache:
    /// cache reads over every input token — fresh, written to the cache, and
    /// read from it. Output is not input and is left out.
    ///
    /// `None` when any token in the span could not be vouched for by kind
    /// (dividing around it would state a rate for part of the work as though
    /// it were the whole), and `None` with no input at all.
    pub fn cache_hit_rate(&self, over_last: usize) -> Option<f64> {
        let span = self.recent(over_last);
        let tally = span
            .iter()
            .fold(TokenTally::default(), |acc, day| acc + day.tally);
        let tokens: i64 = span.iter().map(|day| day.tokens).sum();
        let input = tally.input + tally.cache_write + tally.cache_read;
        if input <= 0 || tally.total() != tokens {
            return None;
        }
        Some(tally.cache_read as f64 / input as f64)
    }

    /// The heaviest day in a span. Scoped rather than all-time so it sits
    /// beside the other figures on the card without quietly changing the
    /// window they all share.
    pub fn busiest_day(&self, over_last: usize) -> Option<&LedgerDay> {
        self.recent(over_last)
            .iter()
            .max_by(|a, b| a.tokens.cmp(&b.tokens))
    }

    /// The model most of the work went through, and how much of it. Falls back
    /// to the whole history when the recent window is quiet, so the line
    /// doesn't vanish after a week off.
    pub fn top_model(&self, over_last: usize) -> Option<(String, f64)> {
        let window: &[LedgerDay] = if self.recent(over_last).iter().any(|day| day.tokens > 0) {
            self.recent(over_last)
        } else {
            &self.days
        };

        let mut totals: BTreeMap<String, i64> = BTreeMap::new();
        for day in window {
            for (model, tokens) in &day.models {
                *totals.entry(model.clone()).or_insert(0) += tokens;
            }
        }

        let overall: i64 = totals.values().sum();
        let (leader, value) = totals.into_iter().max_by_key(|(_, v)| *v)?;
        if overall <= 0 {
            return None;
        }
        let name = self.model_names.get(&leader).cloned().unwrap_or(leader);
        Some((name, value as f64 / overall as f64))
    }
}

/// What one transcript says about itself: its counts by quarter-hour and
/// model, beside what it was called and where it ran.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scanned {
    /// Slot key (`yyyy-MM-dd HH:mm`, local) → model → tally.
    #[serde(default)]
    pub days: BTreeMap<String, BTreeMap<String, TokenTally>>,
    /// The conversation's own name, where the CLI keeps one.
    #[serde(default)]
    pub title: Option<String>,
    /// The directory it ran in, as the transcript states it. **Not decoded
    /// from the folder name**: Claude Code names its project folders for the
    /// path with every separator replaced by a dash, which cannot be reversed
    /// — a folder whose own name contains a dash is indistinguishable from a
    /// separator.
    #[serde(default)]
    pub cwd: Option<String>,
}

/// The quarter-hour a moment falls in, floored on the absolute epoch and then
/// written in **local** time — "today" means the user's today.
pub(crate) fn slot_key_from_ms(ms: i64) -> String {
    const QUARTER_MS: i64 = 15 * 60 * 1000;
    let floored = (ms / QUARTER_MS) * QUARTER_MS;
    let at = Local
        .timestamp_millis_opt(floored)
        .single()
        .unwrap_or_else(|| {
            Local
                .timestamp_millis_opt(floored)
                .earliest()
                .expect("valid time")
        });
    at.format("%Y-%m-%d %H:%M").to_string()
}

fn parse_iso8601(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp_millis())
}

fn slot_key_to_ms(key: &str) -> Option<i64> {
    let naive: NaiveDateTime = NaiveDateTime::parse_from_str(key, "%Y-%m-%d %H:%M").ok()?;
    let at = naive
        .and_local_timezone(Local)
        .single()
        .or_else(|| naive.and_local_timezone(Local).earliest())?;
    Some(at.timestamp_millis())
}

fn day_of_ms(ms: i64) -> NaiveDate {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(|| {
            Local
                .timestamp_millis_opt(ms)
                .earliest()
                .expect("valid time")
        })
        .date_naive()
}

/// The opening prompt, cut to something a row can hold.
///
/// **Not the whole message.** These are the user's own words and a row is one
/// line; the point is to tell one conversation from another, which the first
/// few words do.
pub fn title_from(text: &str) -> Option<String> {
    let cleaned = text.replace('\n', " ").trim().to_string();
    if cleaned.is_empty() {
        return None;
    }
    // A pasted file or a command envelope is not a title.
    if cleaned.starts_with('<') || cleaned.starts_with("Caveat:") {
        return None;
    }
    let count = cleaned.chars().count();
    if count <= 70 {
        Some(cleaned)
    } else {
        let mut cut: String = cleaned.chars().take(69).collect();
        cut.push('…');
        Some(cut)
    }
}

/// The first run of text in a message body, which is a string in the simple
/// case and an array of typed parts in the rich one.
fn text_in(message: Option<&serde_json::Value>) -> Option<String> {
    let message = message?;
    if let Some(text) = message.as_str() {
        return Some(text.to_string());
    }
    let parts = message.as_array()?;
    for part in parts {
        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// JSON numbers arrive as integers or floats depending on what the CLI wrote;
/// both are the same count. Truncation towards zero matches the Swift port.
fn int(value: Option<&serde_json::Value>) -> i64 {
    match value {
        Some(v) => v
            .as_i64()
            .or_else(|| v.as_u64().map(|u| u as i64))
            .or_else(|| v.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        None => 0,
    }
}

/// Cheap substring test, so only the handful of lines that can carry counts
/// are handed to the JSON parser.
fn contains(line: &str, needle: &str) -> bool {
    line.contains(needle)
}

/// Claude Code writes one JSON object per message, each assistant reply
/// carrying the token counts for the request that produced it.
pub fn parse_claude_code(lines: impl Iterator<Item = String>) -> Scanned {
    let mut scanned = Scanned::default();
    let mut days: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    // One streamed reply can be written repeatedly with growing counters.
    // Keep field-wise maxima within this file. Cross-file deduplication is
    // deliberately not part of this parser.
    let mut replies: HashMap<String, (String, String, TokenTally)> = HashMap::new();

    for line in lines {
        if !crate::scan::checkpoint() {
            break;
        }
        // What the session is called and where it ran. **A user-set title can
        // arrive long after the opening prompt** — Claude Code writes
        // `customTitle` when the conversation is renamed — so that one is
        // looked for on every line and the last valid one wins. `cwd` and the
        // opening prompt are only needed once each, which keeps the ordinary
        // line from being parsed twice.
        let renamed = contains(&line, "\"customTitle\"");
        if renamed || scanned.title.is_none() || scanned.cwd.is_none() {
            if let Ok(root) = serde_json::from_str::<serde_json::Value>(&line) {
                if scanned.cwd.is_none() {
                    if let Some(cwd) = root.get("cwd").and_then(|c| c.as_str()) {
                        if !cwd.is_empty() {
                            scanned.cwd = Some(cwd.to_string());
                        }
                    }
                }
                // The user's own name outranks the opening prompt, and a
                // later rename outranks an earlier one. An unreadable custom
                // title (empty, or an envelope) leaves the title that was
                // already found rather than clearing it.
                if renamed {
                    if let Some(custom) = root.get("customTitle").and_then(|c| c.as_str()) {
                        if let Some(title) = title_from(custom) {
                            scanned.title = Some(title);
                        }
                    }
                } else if scanned.title.is_none()
                    && root.get("type").and_then(|t| t.as_str()) == Some("user")
                    && root.get("isSidechain").and_then(|s| s.as_bool()) != Some(true)
                {
                    let message = root.get("message");
                    if let Some(text) = text_in(message.and_then(|m| m.get("content"))) {
                        if let Some(title) = title_from(&text) {
                            scanned.title = Some(title);
                        }
                    }
                }
            }
        }

        if !contains(&line, "\"usage\"") {
            continue;
        }
        let Ok(root) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(message) = root.get("message") else {
            continue;
        };
        let Some(usage) = message.get("usage") else {
            continue;
        };
        let Some(model) = message.get("model").and_then(|m| m.as_str()) else {
            continue;
        };
        // Placeholders Claude Code writes for its own errors; no request was
        // made, so there is nothing to price.
        if model == "<synthetic>" {
            continue;
        }
        let Some(timestamp) = root.get("timestamp").and_then(|t| t.as_str()) else {
            continue;
        };
        let Some(at_ms) = parse_iso8601(timestamp) else {
            continue;
        };

        let mut tally = TokenTally {
            input: int(usage.get("input_tokens")).max(0),
            cache_write: int(usage.get("cache_creation_input_tokens")).max(0),
            cache_read: int(usage.get("cache_read_input_tokens")).max(0),
            output: int(usage.get("output_tokens")).max(0),
        };
        if tally.total() <= 0 {
            continue;
        }

        let mut key = slot_key_from_ms(at_ms);
        let mut model = model.to_string();
        if let Some(id) = message
            .get("id")
            .and_then(|i| i.as_str())
            .filter(|id| !id.is_empty())
        {
            let reply = replies
                .entry(id.to_string())
                .or_insert_with(|| (key.clone(), model.clone(), TokenTally::default()));
            key = reply.0.clone();
            model = reply.1.clone();
            let previous = reply.2;
            let merged = TokenTally {
                input: previous.input.max(tally.input),
                cache_write: previous.cache_write.max(tally.cache_write),
                cache_read: previous.cache_read.max(tally.cache_read),
                output: previous.output.max(tally.output),
            };
            tally = TokenTally {
                input: merged.input - previous.input,
                cache_write: merged.cache_write - previous.cache_write,
                cache_read: merged.cache_read - previous.cache_read,
                output: merged.output - previous.output,
            };
            reply.2 = merged;
        }
        *days.entry(key).or_default().entry(model).or_default() += tally;
    }

    scanned.days = days;
    scanned
}

/// Codex reports a running total for the session rather than a figure per
/// turn, so each reading is differenced against the one before it. The
/// running total only ever climbs, which makes the differences safe to add up
/// — and it sidesteps the duplicate readings that summing Codex's own
/// per-turn field would double-count.
pub fn parse_codex(lines: impl Iterator<Item = String>) -> Scanned {
    parse_codex_with_state(lines, Scanned::default(), &mut CodexState::default())
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CodexState {
    model: Option<String>,
    previous: Option<[i64; 4]>,
}

fn parse_codex_with_state(
    lines: impl Iterator<Item = String>,
    mut scanned: Scanned,
    state: &mut CodexState,
) -> Scanned {
    let mut days = std::mem::take(&mut scanned.days);

    for line in lines {
        if !crate::scan::checkpoint() {
            break;
        }
        if scanned.title.is_none() || scanned.cwd.is_none() {
            // The directory is stated once in the session header; the opening
            // prompt is a `response_item` whose payload is a message with the
            // user's role on it — **not** an `event_msg`, which is what the
            // first attempt looked for and why every Codex session came out
            // unnamed.
            if contains(&line, "\"cwd\"") || contains(&line, "\"role\":\"user\"") {
                if let Ok(root) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(payload) = root.get("payload") {
                        if scanned.cwd.is_none() {
                            if let Some(cwd) = payload.get("cwd").and_then(|c| c.as_str()) {
                                if !cwd.is_empty() {
                                    scanned.cwd = Some(cwd.to_string());
                                }
                            }
                        }
                        if scanned.title.is_none()
                            && payload.get("type").and_then(|t| t.as_str()) == Some("message")
                            && payload.get("role").and_then(|r| r.as_str()) == Some("user")
                        {
                            if let Some(text) = text_in(payload.get("content")) {
                                if let Some(title) = title_from(&text) {
                                    scanned.title = Some(title);
                                }
                            }
                        }
                    }
                }
            }
        }

        let is_count = contains(&line, "\"token_count\"");
        if !is_count && !contains(&line, "\"model\"") {
            continue;
        }
        let Ok(root) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };

        let payload = root.get("payload").cloned().unwrap_or_default();

        // The model can change mid-session; usage is attributed to whichever
        // was in force when the reading was taken.
        if let Some(named) = payload.get("model").and_then(|m| m.as_str()) {
            state.model = Some(named.to_string());
        }

        if !is_count
            || payload.get("type").and_then(|t| t.as_str()) != Some("token_count")
            || state.model.is_none()
        {
            continue;
        }
        let Some(totals) = payload
            .get("info")
            .and_then(|info| info.get("total_token_usage"))
        else {
            continue;
        };
        let Some(timestamp) = root.get("timestamp").and_then(|t| t.as_str()) else {
            continue;
        };
        let Some(at_ms) = parse_iso8601(timestamp) else {
            continue;
        };

        let current = [
            int(totals.get("input_tokens")),
            int(totals.get("cached_input_tokens")),
            int(totals.get("cache_write_input_tokens")),
            int(totals.get("output_tokens")),
        ];
        let previous_totals = state.previous.unwrap_or([0; 4]);
        let delta: [i64; 4] = std::array::from_fn(|i| (current[i] - previous_totals[i]).max(0));
        state.previous = Some(current);

        // Codex counts cached tokens inside its input figure; the price list
        // treats them as two separate rates.
        let tally = TokenTally {
            input: (delta[0] - delta[1]).max(0),
            cache_write: delta[2],
            cache_read: delta[1],
            output: delta[3],
        };
        if tally.total() <= 0 {
            continue;
        }

        let key = slot_key_from_ms(at_ms);
        *days
            .entry(key)
            .or_default()
            .entry(state.model.clone().expect("guarded above"))
            .or_default() += tally;
    }

    scanned.days = days;
    scanned
}

/// What has already been counted, so opening a card a second time doesn't
/// re-read a few hundred megabytes of transcripts. Unchanged stamps reuse
/// the entry; a growing Codex file also verifies its entire previous prefix
/// before resuming the cumulative counters at a newline boundary.
#[derive(Debug, Default, Serialize, Deserialize)]
struct FileCache {
    files: BTreeMap<String, CachedEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedEntry {
    stamp: Stamp,
    days: BTreeMap<String, BTreeMap<String, TokenTally>>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex: Option<CodexCheckpoint>,
}

/// Parser state at a newline boundary, guarded by the entire old prefix.
#[derive(Debug, Serialize, Deserialize)]
struct CodexCheckpoint {
    version: u8,
    offset: u64,
    digest: u64,
    state: CodexState,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Stamp {
    size: i64,
    // Integer nanoseconds survive JSON roundtrips exactly. Older caches
    // contain only floating seconds: read them, then refresh their stamps.
    #[serde(default)]
    modified_ns: Option<u64>,
}

impl Stamp {
    fn of(path: &Path) -> Option<Stamp> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some(Stamp {
            size: meta.len() as i64,
            modified_ns: Some(modified.try_into().ok()?),
        })
    }
}

impl FileCache {
    fn load(provider: Provider) -> FileCache {
        if crate::settings::with(|s| s.statistics_cache_limit_mb == 0) {
            return FileCache::default();
        }
        let path = data_dir().join(ledger_cache_name(provider));
        crate::scan::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    fn save(&self, provider: Provider) {
        if !crate::scan::checkpoint() {
            return;
        }
        if let Ok(data) = serde_json::to_vec(self) {
            if !crate::scan::checkpoint() {
                return;
            }
            let _ = crate::statistics_cache::write(&ledger_cache_name(provider), &data);
        }
    }
}

fn ledger_cache_name(provider: Provider) -> String {
    let version = if provider == Provider::ClaudeCode {
        5
    } else {
        4
    };
    format!("ledger-{version}-{}.json", provider.raw())
}

pub fn slot_share(slot_ms: i64, start_ms: i64, end_ms: i64) -> f64 {
    let overlap = slot_ms
        .saturating_add(900_000)
        .min(end_ms)
        .saturating_sub(slot_ms.max(start_ms));
    (overlap as f64 / 900_000.0).clamp(0.0, 1.0)
}

fn data_dir() -> PathBuf {
    crate::data_dir()
}

/// The local history store a provider leaves on this machine, when it has one.
// The CLI uses a separate directory and is deliberately not mixed in.
fn antigravity_roots() -> Vec<PathBuf> {
    let root = std::env::var_os("GEMINI_CLI_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| crate::model::home_path(".gemini"));
    let current = root.join("antigravity").join("conversations");
    let roots: Vec<_> = [
        current.clone(),
        root.join("antigravity-ide").join("conversations"),
    ]
    .into_iter()
    .filter(|path| path.is_dir())
    .collect();
    if roots.is_empty() {
        vec![current]
    } else {
        roots
    }
}

pub fn transcript_root(provider: Provider) -> Option<PathBuf> {
    if provider == Provider::Antigravity {
        return antigravity_roots().into_iter().next();
    }
    let written = match provider {
        Provider::ClaudeCode => ".claude/projects",
        Provider::Codex => ".codex/sessions",
        // None of the other providers leaves a transcript this reads.
        _ => return None,
    };
    Some(crate::model::home_path(written))
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    collect_files(dir, "jsonl", out);
}

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>) {
    collect_files(dir, "json", out);
}

fn collect_files(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    if !crate::scan::checkpoint() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !crate::scan::checkpoint() {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, extension, out);
        } else if path
            .extension()
            .map(|candidate| candidate == extension)
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

fn read_entry(
    path: &Path,
    provider: Provider,
    stamp: Stamp,
    known: Option<CachedEntry>,
) -> std::io::Result<CachedEntry> {
    use std::collections::hash_map::DefaultHasher;
    use std::io::{Read, Seek, SeekFrom};

    if known.as_ref().is_some_and(|k| k.stamp == stamp) {
        return Ok(known.expect("matched above"));
    }
    let mut file = std::fs::File::open(path)?;
    let mut resume = None;
    if provider == Provider::Codex {
        if let Some(mut known) = known {
            if let Some(cursor) = known.codex.take().filter(|c| {
                c.version == 1
                    && known.stamp.size >= 0
                    && c.offset == known.stamp.size as u64
                    && stamp.size > known.stamp.size
            }) {
                if let Some(digest) =
                    crate::scan::verify_prefix(&mut file, cursor.offset, cursor.digest)?
                {
                    resume = Some((
                        Scanned {
                            days: known.days,
                            title: known.title,
                            cwd: known.cwd,
                        },
                        cursor.state,
                        digest,
                        cursor.offset,
                    ));
                }
            }
        }
    }
    let (scanned, mut state, digest, offset) = match resume {
        Some(resume) => resume,
        None => {
            file.seek(SeekFrom::Start(0))?;
            (
                Scanned::default(),
                CodexState::default(),
                DefaultHasher::new(),
                0,
            )
        }
    };
    let mut lines =
        crate::scan::LineReader::new(file.take(stamp.size as u64 - offset), digest, offset);
    let scanned = match provider {
        Provider::ClaudeCode => parse_claude_code(lines.by_ref()),
        Provider::Codex => parse_codex_with_state(lines.by_ref(), scanned, &mut state),
        _ => return Err(std::io::Error::other("unsupported transcript")),
    };
    let (end, digest, newline) = lines.finish()?;
    if end != stamp.size as u64 {
        return Err(std::io::Error::other("transcript changed during scan"));
    }
    if Stamp::of(path).as_ref() != Some(&stamp) {
        // A live session may append while we parse. Accept the bounded
        // snapshot only if those exact bytes are still intact afterward.
        let mut current = std::fs::File::open(path)?;
        if crate::scan::verify_prefix(&mut current, end, digest)?.is_none() {
            return Err(std::io::Error::other("transcript changed during scan"));
        }
    }
    Ok(CachedEntry {
        stamp,
        days: scanned.days,
        title: scanned.title,
        cwd: scanned.cwd,
        codex: (provider == Provider::Codex && newline).then_some(CodexCheckpoint {
            version: 1,
            offset: end,
            digest,
            state,
        }),
    })
}

const ANTIGRAVITY_MAX_BLOB_BYTES: i64 = 1_048_576;
const ANTIGRAVITY_ROUTING_MODEL: &str = "gemini-default";
// These usage fields have observed meanings; field 1 is intentionally omitted.

#[derive(Clone)]
enum AntigravityWireValue {
    Varint(u64),
    Fixed64,
    Bytes(Vec<u8>),
    Fixed32,
}

#[derive(Default)]
struct AntigravityMessage {
    fields: BTreeMap<u32, Vec<AntigravityWireValue>>,
}

impl AntigravityMessage {
    fn first(&self, field: u32) -> Option<&AntigravityWireValue> {
        self.fields.get(&field)?.first()
    }

    fn varint(&self, field: u32) -> Option<u64> {
        match self.first(field)? {
            AntigravityWireValue::Varint(value) => Some(*value),
            _ => None,
        }
    }

    fn text(&self, field: u32) -> Option<String> {
        let AntigravityWireValue::Bytes(bytes) = self.first(field)? else {
            return None;
        };
        let text = std::str::from_utf8(bytes).ok()?.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    fn nested(&self, field: u32) -> Option<AntigravityMessage> {
        let AntigravityWireValue::Bytes(bytes) = self.first(field)? else {
            return None;
        };
        decode_antigravity_message(bytes)
    }
}

fn read_antigravity_varint(bytes: &[u8], offset: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*offset)?;
        *offset += 1;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn decode_antigravity_message(bytes: &[u8]) -> Option<AntigravityMessage> {
    if bytes.len() > ANTIGRAVITY_MAX_BLOB_BYTES as usize {
        return None;
    }
    let mut message = AntigravityMessage::default();
    let mut offset = 0;
    let mut field_count = 0;
    while offset < bytes.len() {
        field_count += 1;
        if field_count > 4096 {
            return None;
        }
        let tag = read_antigravity_varint(bytes, &mut offset)?;
        let field = u32::try_from(tag >> 3).ok()?;
        if field == 0 {
            return None;
        }
        let value = match tag & 7 {
            0 => AntigravityWireValue::Varint(read_antigravity_varint(bytes, &mut offset)?),
            1 => {
                let end = offset.checked_add(8)?;
                bytes.get(offset..end)?;
                offset = end;
                AntigravityWireValue::Fixed64
            }
            2 => {
                let length = usize::try_from(read_antigravity_varint(bytes, &mut offset)?).ok()?;
                let end = offset.checked_add(length)?;
                let value = bytes.get(offset..end)?.to_vec();
                offset = end;
                AntigravityWireValue::Bytes(value)
            }
            5 => {
                let end = offset.checked_add(4)?;
                bytes.get(offset..end)?;
                offset = end;
                AntigravityWireValue::Fixed32
            }
            // Groups and unknown wire types are rejected as a whole message.
            _ => return None,
        };
        message.fields.entry(field).or_default().push(value);
    }
    Some(message)
}

fn antigravity_timestamp(message: &AntigravityMessage) -> Option<i64> {
    let seconds = message.varint(1)?;
    let nanos = message.varint(2).unwrap_or(0);
    if seconds == 0 || nanos >= 1_000_000_000 {
        return None;
    }
    let millis = i64::try_from(seconds)
        .ok()?
        .checked_mul(1_000)?
        .checked_add(i64::try_from(nanos / 1_000_000).ok()?)?;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)?;
    Some(millis)
}

#[derive(Default)]
struct AntigravityStepTimes {
    by_response: HashMap<String, i64>,
    by_index: HashMap<i64, i64>,
}

struct AntigravityGeneration {
    index: i64,
    model: Option<String>,
    label: Option<String>,
    tally: Option<TokenTally>,
    response_id: Option<String>,
    timestamp_ms: Option<i64>,
}

fn antigravity_count(message: &AntigravityMessage, field: u32) -> i64 {
    message.varint(field).unwrap_or(0).min(i64::MAX as u64) as i64
}

fn antigravity_generation(index: i64, bytes: &[u8]) -> Option<AntigravityGeneration> {
    let root = decode_antigravity_message(bytes)?;
    let chat = root.nested(1)?;
    let usage = chat.nested(4);
    let tally = usage.as_ref().map(|usage| TokenTally {
        input: antigravity_count(usage, 2),
        cache_write: 0,
        cache_read: antigravity_count(usage, 5),
        output: antigravity_count(usage, 9).saturating_add(antigravity_count(usage, 10)),
    });
    let timestamp_ms = chat
        .nested(9)
        .and_then(|response| response.nested(4))
        .and_then(|timestamp| antigravity_timestamp(&timestamp));
    Some(AntigravityGeneration {
        index,
        model: chat.text(19),
        label: chat.text(21),
        tally,
        response_id: usage.as_ref().and_then(|usage| usage.text(11)),
        timestamp_ms,
    })
}

fn antigravity_steps(connection: &rusqlite::Connection) -> AntigravityStepTimes {
    let mut times = AntigravityStepTimes::default();
    let Ok(mut statement) = connection.prepare(
        "SELECT metadata FROM steps \
         WHERE step_type = 15 AND length(metadata) <= ?1",
    ) else {
        return times;
    };
    let Ok(rows) =
        statement.query_map([ANTIGRAVITY_MAX_BLOB_BYTES], |row| row.get::<_, Vec<u8>>(0))
    else {
        return times;
    };
    for bytes in rows.flatten() {
        if !crate::scan::checkpoint() {
            break;
        }
        let Some(step) = decode_antigravity_message(&bytes) else {
            continue;
        };
        let Some(timestamp_ms) = step
            .nested(1)
            .and_then(|timestamp| antigravity_timestamp(&timestamp))
        else {
            continue;
        };
        if let Some(response_id) = step.nested(9).and_then(|response| response.text(11)) {
            times.by_response.insert(response_id, timestamp_ms);
        }
        if let Some(index) = step.nested(20).and_then(|generation| generation.varint(3)) {
            if let Ok(index) = i64::try_from(index) {
                times.by_index.insert(index, timestamp_ms);
            }
        }
    }
    times
}

fn antigravity_anchor(connection: &rusqlite::Connection) -> Option<i64> {
    let bytes = connection
        .query_row(
            "SELECT data FROM trajectory_metadata_blob \
             WHERE length(data) <= ?1 LIMIT 1",
            [ANTIGRAVITY_MAX_BLOB_BYTES],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .ok()?;
    decode_antigravity_message(&bytes)?
        .nested(2)
        .and_then(|timestamp| antigravity_timestamp(&timestamp))
}

fn antigravity_generations(connection: &rusqlite::Connection) -> Vec<AntigravityGeneration> {
    let Ok(mut statement) = connection.prepare(
        "SELECT idx, data FROM gen_metadata \
         WHERE length(data) <= ?1 ORDER BY idx",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([ANTIGRAVITY_MAX_BLOB_BYTES], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|(index, bytes)| antigravity_generation(index, &bytes))
        .collect()
}

fn antigravity_model(
    generation: &AntigravityGeneration,
    labels: &HashMap<String, HashSet<String>>,
    sole_model: Option<&str>,
) -> String {
    if let Some(model) = generation
        .model
        .as_deref()
        .filter(|model| *model != ANTIGRAVITY_ROUTING_MODEL)
    {
        return model.to_string();
    }
    if let Some(model) = generation
        .label
        .as_ref()
        .and_then(|label| labels.get(label))
        .filter(|models| models.len() == 1)
        .and_then(|models| models.iter().next())
    {
        return model.clone();
    }
    if let Some(model) = sole_model {
        return model.to_string();
    }
    // The label is Antigravity's own name for what it called — "Gemini 3.7
    // Flash (High)" — so it reaches the price table as a name, and the row
    // keeps the tier the interface showed rather than folding several models
    // into one bucket. A label that is not a name says nothing about the
    // model, and is recorded as such.
    generation
        .label
        .clone()
        .filter(|label| label.contains(' '))
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_antigravity_database(path: &Path, seen: &mut HashSet<String>) -> Scanned {
    let Ok(connection) =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return Scanned::default();
    };
    let _ = connection.busy_timeout(Duration::from_millis(250));
    let anchor = antigravity_anchor(&connection);
    let steps = antigravity_steps(&connection);
    let generations = antigravity_generations(&connection);

    let mut labels: HashMap<String, HashSet<String>> = HashMap::new();
    let mut models = HashSet::new();
    for generation in &generations {
        if !crate::scan::checkpoint() {
            break;
        }
        if let Some(model) = generation
            .model
            .as_ref()
            .filter(|model| *model != ANTIGRAVITY_ROUTING_MODEL)
        {
            models.insert(model.clone());
            if let Some(label) = &generation.label {
                labels
                    .entry(label.clone())
                    .or_default()
                    .insert(model.clone());
            }
        }
    }
    let sole_model = (models.len() == 1)
        .then(|| models.iter().next().map(String::as_str))
        .flatten();
    let session = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let mut days = BTreeMap::new();

    for generation in generations {
        if !crate::scan::checkpoint() {
            break;
        }
        let Some(tally) = generation.tally.filter(|tally| tally.total() > 0) else {
            continue;
        };
        if let Some(response_id) = &generation.response_id {
            let key = format!("{session}:{response_id}");
            if !seen.insert(key) {
                continue;
            }
        }
        let event_time = generation
            .timestamp_ms
            .or_else(|| {
                generation
                    .response_id
                    .as_ref()
                    .and_then(|response| steps.by_response.get(response).copied())
            })
            .or_else(|| steps.by_index.get(&generation.index).copied());
        let Some(timestamp_ms) = event_time.or(anchor) else {
            continue;
        };
        let model = antigravity_model(&generation, &labels, sole_model);
        *days
            .entry(slot_key_from_ms(timestamp_ms))
            .or_insert_with(BTreeMap::new)
            .entry(model)
            .or_insert_with(TokenTally::default) += tally;
    }

    Scanned {
        days,
        title: None,
        cwd: None,
    }
}

fn scan_antigravity() -> BTreeMap<String, BTreeMap<String, TokenTally>> {
    let mut files = Vec::new();
    for root in antigravity_roots() {
        if !crate::scan::checkpoint() {
            break;
        }
        collect_files(&root, "db", &mut files);
    }
    files.sort();

    // An open database can append to its WAL while the main database's size
    // and modification time stay put, so scan SQLite files without file stamps.
    let mut seen = HashSet::new();
    let mut buckets = BTreeMap::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        for (slot, models) in parse_antigravity_database(&file, &mut seen).days {
            for (model, tally) in models {
                *buckets
                    .entry(slot.clone())
                    .or_insert_with(BTreeMap::new)
                    .entry(model)
                    .or_insert_with(TokenTally::default) += tally;
            }
        }
    }
    buckets
}

/// Reads every transcript under the provider's root, reusing the on-disk cache
/// for any file whose stamp has not moved.
fn scan(provider: Provider) -> (BTreeMap<String, BTreeMap<String, TokenTally>>, FileCache) {
    let Some(root) = transcript_root(provider) else {
        return (BTreeMap::new(), FileCache::default());
    };
    let scanned = scan_cached(provider, &root, FileCache::load(provider));
    if scanned.changed {
        scanned.cache.save(provider);
    }
    (scanned.buckets, scanned.cache)
}

struct TranscriptScan {
    buckets: BTreeMap<String, BTreeMap<String, TokenTally>>,
    cache: FileCache,
    changed: bool,
}

fn scan_cached(provider: Provider, root: &Path, mut cache: FileCache) -> TranscriptScan {
    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut fresh: BTreeMap<String, CachedEntry> = BTreeMap::new();
    let mut changed = false;

    let mut files: Vec<PathBuf> = Vec::new();
    collect_jsonl(root, &mut files);
    files.sort();

    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Some(stamp) = Stamp::of(&file) else {
            continue;
        };
        let key = file.to_string_lossy().to_string();

        let known = cache.files.remove(&key);
        // read_entry returns this exact entry when its stamp is unchanged.
        // New/edited entries and old entries dropped after a read failure
        // require persistence; a repricing-only pass does not.
        let reused = known.as_ref().is_some_and(|entry| entry.stamp == stamp);
        let had_known = known.is_some();
        let entry = match read_entry(&file, provider, stamp, known) {
            Ok(entry) => {
                changed |= !reused;
                entry
            }
            Err(_) => {
                changed |= had_known;
                continue;
            }
        };

        for (day, models) in &entry.days {
            if !crate::scan::checkpoint() {
                break;
            }
            for (model, tally) in models {
                *buckets
                    .entry(day.clone())
                    .or_default()
                    .entry(model.clone())
                    .or_default() += *tally;
            }
        }
        fresh.insert(key, entry);
    }

    changed |= !cache.files.is_empty(); // deleted or no longer readable files
    cache.files = fresh;
    TranscriptScan {
        buckets,
        cache,
        changed,
    }
}

/// Turns buckets into days, slots and money. **Static and free of instance
/// state**, so every caller prices the same way — two ways of turning tokens
/// into dollars in one app is two figures that eventually disagree.
pub fn priced(
    buckets: &BTreeMap<String, BTreeMap<String, TokenTally>>,
    prices: &BTreeMap<String, ModelPrice>,
    vendor: Option<&str>,
) -> UsageLedger {
    if !crate::scan::checkpoint() || buckets.is_empty() {
        return UsageLedger::empty();
    }

    let mut unpriced: BTreeSet<String> = BTreeSet::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut slots: Vec<Slot> = Vec::new();
    let mut lookup = model_prices::ModelPriceLookup::new(prices);

    // Rolled up as we go: the card wants days, the window estimate wants the
    // raw quarter-hours, and both come out of the same pass.
    let mut day_tokens: HashMap<NaiveDate, i64> = HashMap::new();
    let mut day_cost: HashMap<NaiveDate, f64> = HashMap::new();
    let mut day_unpriced: HashMap<NaiveDate, i64> = HashMap::new();
    let mut day_models: HashMap<NaiveDate, BTreeMap<String, i64>> = HashMap::new();
    let mut day_model_tallies: HashMap<NaiveDate, BTreeMap<String, TokenTally>> = HashMap::new();
    let mut day_model_costs: HashMap<NaiveDate, BTreeMap<String, TokenCost>> = HashMap::new();
    let mut day_tally: HashMap<NaiveDate, TokenTally> = HashMap::new();

    for (key, models) in buckets {
        if !crate::scan::checkpoint() {
            return UsageLedger::empty();
        }
        let Some(start_ms) = slot_key_to_ms(key) else {
            continue;
        };
        let day = day_of_ms(start_ms);

        let mut tokens = 0i64;
        let mut cost = 0.0f64;
        let mut unpriced_tokens = 0i64;
        let mut costs: BTreeMap<String, f64> = BTreeMap::new();

        for (model, tally) in models {
            tokens += tally.total();
            *day_models
                .entry(day)
                .or_default()
                .entry(model.clone())
                .or_insert(0) += tally.total();
            *day_model_tallies
                .entry(day)
                .or_default()
                .entry(model.clone())
                .or_default() += *tally;
            *day_tally.entry(day).or_default() += *tally;

            if let Some(price) = lookup.price(model, vendor) {
                let money = tally.cost_breakdown(&price);
                let total = money.total();
                cost += total;
                *costs.entry(model.clone()).or_default() += total;
                *day_model_costs
                    .entry(day)
                    .or_default()
                    .entry(model.clone())
                    .or_default() += money;
                if let Some(name) = price.name {
                    names.insert(model.clone(), name);
                }
            } else {
                unpriced.insert(model.clone());
                unpriced_tokens += tally.total();
            }
        }

        slots.push(Slot {
            start_ms,
            tokens,
            cost,
            unpriced_tokens,
            models: models.clone(),
            costs,
        });

        *day_tokens.entry(day).or_insert(0) += tokens;
        *day_cost.entry(day).or_insert(0.0) += cost;
        *day_unpriced.entry(day).or_insert(0) += unpriced_tokens;
    }

    let earliest = day_tokens.keys().copied().min();
    let latest = day_tokens.keys().copied().max();
    let (Some(earliest), Some(latest)) = (earliest, latest) else {
        return UsageLedger::empty();
    };

    // Fill the quiet days back in. Without them the bars would sit shoulder to
    // shoulder and a fortnight off would look like a weekend.
    let mut days: Vec<LedgerDay> = Vec::new();
    let mut cursor = earliest;
    while cursor <= latest {
        days.push(LedgerDay {
            date: cursor,
            tokens: day_tokens.get(&cursor).copied().unwrap_or(0),
            cost: day_cost.get(&cursor).copied().unwrap_or(0.0),
            unpriced_tokens: day_unpriced.get(&cursor).copied().unwrap_or(0),
            models: day_models.get(&cursor).cloned().unwrap_or_default(),
            tally: day_tally.get(&cursor).copied().unwrap_or_default(),
            model_tallies: day_model_tallies.get(&cursor).cloned().unwrap_or_default(),
            model_costs: day_model_costs.get(&cursor).cloned().unwrap_or_default(),
        });
        let Some(next) = cursor.succ_opt() else { break };
        cursor = next;
    }

    slots.sort_by_key(|slot| slot.start_ms);

    UsageLedger {
        days,
        earliest: Some(earliest),
        unpriced_models: unpriced.into_iter().collect(),
        model_names: names,
        slots,
        has_partial_records: false,
    }
}

/// Long enough that moving between rings does not rescan, short enough that
/// "Today" is today's. The upstream card keeps the same lifetime.
const LIFETIME: Duration = Duration::from_secs(5 * 60);

static MEMORY: Mutex<Option<(Instant, HashMap<String, UsageLedger>)>> = Mutex::new(None);

type MemoryCache = Option<(Instant, HashMap<String, UsageLedger>)>;

fn expire_memory(cache: &mut MemoryCache, now: Instant, enabled: bool) {
    if !enabled
        || cache
            .as_ref()
            .is_some_and(|(at, _)| now.duration_since(*at) >= LIFETIME)
    {
        *cache = None;
    }
}

fn memory() -> std::sync::MutexGuard<'static, MemoryCache> {
    let mut cache = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    // Eviction also renews the timestamp when the next scan is inserted.
    // Otherwise every read after the first five minutes rebuilds forever.
    expire_memory(&mut cache, Instant::now(), true);
    cache
}

/// Called during maintenance even when nobody is opening a transcript card.
/// Only derived memory is released; files and persisted history stay intact.
pub fn release_expired_memory(enabled: bool) {
    // Maintenance runs on the UI path; a transcript scan may own the lock.
    // Defer eviction instead of making the message loop wait for disk I/O.
    if let Ok(mut cache) = MEMORY.try_lock() {
        expire_memory(&mut cache, Instant::now(), enabled);
    }
    crate::zcode_spend::release_memory(enabled);
}

/// Manual refresh runs this on the scan worker, never on the UI thread.
/// Persisted per-file stamps still avoid parsing unchanged transcripts.
pub fn invalidate_memory() {
    let mut cache = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    if crate::scan::checkpoint() {
        *cache = None;
    }
}

/// Worker-only. Keep the same memory -> disk lock order as ledger scans.
pub fn clear_statistics_cache() -> crate::statistics_cache::Cleanup {
    let mut cache = MEMORY.lock().unwrap_or_else(|e| e.into_inner());
    *cache = None;
    crate::zcode_spend::clear_memory();
    crate::statistics_cache::clear_disk()
}

/// The provider's ledger, read through the in-memory cache. Providers without
/// a transcript root come back empty — including every one whose history is
/// not this machine's to know.
pub fn ledger(provider: Provider) -> UsageLedger {
    let Some(_) = transcript_root(provider) else {
        return UsageLedger::empty();
    };
    let raw = provider.raw().to_string();

    let mut guard = memory();
    if let Some((at, map)) = guard.as_ref() {
        if at.elapsed() < LIFETIME {
            if let Some(known) = map.get(&raw) {
                return known.clone();
            }
        }
    } else {
        *guard = None;
    }

    let built = build(provider);
    let entry = guard.get_or_insert_with(|| (Instant::now(), HashMap::new()));
    if crate::scan::checkpoint() {
        entry.1.insert(raw, built.clone());
    }
    built
}

fn build(provider: Provider) -> UsageLedger {
    let buckets = if provider == Provider::Antigravity {
        scan_antigravity()
    } else {
        scan(provider).0
    };
    if buckets.is_empty() {
        return UsageLedger::empty();
    }
    if !crate::scan::checkpoint() {
        return UsageLedger::empty();
    }
    let prices = model_prices::prices();
    let mut ledger = priced(&buckets, &prices, None);
    ledger.has_partial_records = provider == Provider::Antigravity;
    ledger
}

/// Qwen Code's transcript root. It follows the client's own documented
/// `~/.qwen/projects` layout; nothing is discovered by guessing.
pub fn qwen_root() -> PathBuf {
    crate::model::home_path(".qwen/projects")
}

/// Decodes Qwen Code's normalized Gemini usage object.
///
/// Google documents `promptTokenCount` as including cached content, so the
/// fresh input is prompt minus cache read. A stated total is authoritative:
/// it can prove the cache sits inside or beside the prompt, and a total that
/// proves neither is skipped rather than split on a guess.
fn qwen_usage(metadata: &serde_json::Value) -> Option<TokenTally> {
    let object = metadata.as_object()?;
    let integer = |name: &str| -> Option<i64> {
        match object.get(name)? {
            serde_json::Value::Number(value) => value.as_i64(),
            _ => None,
        }
    };
    let prompt = integer("promptTokenCount");
    let candidates = integer("candidatesTokenCount");
    let thoughts = integer("thoughtsTokenCount");
    let cached = integer("cachedContentTokenCount");
    let total = integer("totalTokenCount")
        .or_else(|| integer("total"))
        .or_else(|| integer("total_tokens"));
    if prompt.is_none()
        && candidates.is_none()
        && thoughts.is_none()
        && cached.is_none()
        && total.is_none()
    {
        return None;
    }

    let prompt = prompt.unwrap_or(0);
    let cached = cached.unwrap_or(0);
    let output = candidates
        .unwrap_or(0)
        .saturating_add(thoughts.unwrap_or(0));
    if [prompt, cached, output]
        .iter()
        .copied()
        .any(|value| value < 0)
    {
        return None;
    }

    if let Some(total) = total {
        if total < 0 {
            return None;
        }
        let included = prompt.checked_add(output)?;
        let disjoint = included.checked_add(cached)?;
        if cached == 0 {
            return if total == included {
                Some(TokenTally {
                    input: prompt,
                    cache_write: 0,
                    cache_read: 0,
                    output,
                })
            } else {
                None
            };
        }
        if total == included && total != disjoint {
            return Some(TokenTally {
                input: prompt.checked_sub(cached)?,
                cache_write: 0,
                cache_read: cached,
                output,
            });
        }
        if total == disjoint && total != included {
            return Some(TokenTally {
                input: prompt,
                cache_write: 0,
                cache_read: cached,
                output,
            });
        }
        return None;
    }

    Some(TokenTally {
        input: prompt.saturating_sub(cached),
        cache_write: 0,
        cache_read: cached,
        output,
    })
}

/// The `<projectPath>` segment Qwen keeps between `projects` and `chats`.
fn qwen_project(path: &Path) -> Option<String> {
    let mut components = path.components().map(|component| component.as_os_str());
    while let Some(component) = components.next() {
        if component == std::ffi::OsStr::new("projects") {
            return components
                .next()
                .filter(|segment| !segment.is_empty())
                .map(|segment| segment.to_string_lossy().into_owned());
        }
    }
    None
}

/// Reads Qwen Code transcripts through the same five-minute ledger cache as
/// the provider readers. Only classified usage is counted; an unreconciled
/// total contributes nothing rather than inventing token kinds.
pub fn qwen_ledger() -> UsageLedger {
    const KEY: &str = "qwen";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let root = qwen_root();
    let mut files = Vec::new();
    collect_jsonl(&root, &mut files);
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(text) = crate::scan::read_to_string(&file) else {
            continue;
        };
        let project = qwen_project(&file).unwrap_or_else(|| {
            file.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "qwen".into())
        });
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        for (index, line) in text.lines().enumerate() {
            if !crate::scan::checkpoint() {
                break;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if row.get("type").and_then(serde_json::Value::as_str) != Some("assistant") {
                continue;
            }
            let Some(metadata) = row.get("usageMetadata") else {
                continue;
            };
            let Some(tally) = qwen_usage(metadata) else {
                continue;
            };
            if tally.total() == 0 {
                continue;
            }
            let Some(model) = row
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_string)
            else {
                continue;
            };
            let Some(at) = row
                .get("timestamp")
                .and_then(serde_json::Value::as_str)
                .and_then(parse_iso8601)
            else {
                continue;
            };
            let session_id = row
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{project}-{stem}"));
            let message_id = row
                .get("id")
                .or_else(|| row.get("messageId"))
                .and_then(|id| {
                    id.as_str()
                        .map(str::to_string)
                        .or_else(|| id.as_i64().map(|value| value.to_string()))
                });
            let identity = match message_id {
                Some(id) => format!("{session_id}:{id}"),
                None => format!("{session_id}:{index}"),
            };
            if !seen.insert(identity) {
                continue;
            }
            *buckets
                .entry(slot_key_from_ms(at))
                .or_default()
                .entry(model)
                .or_default() += tally;
        }
    }

    if !crate::scan::checkpoint() {
        return UsageLedger::empty();
    }
    let built = if buckets.is_empty() {
        UsageLedger::empty()
    } else {
        priced(&buckets, &model_prices::prices(), None)
    };
    let mut guard = memory();
    let entry = guard.get_or_insert_with(|| (Instant::now(), HashMap::new()));
    if crate::scan::checkpoint() {
        entry.1.insert(KEY.to_string(), built.clone());
    }
    built
}

/// A JSON timestamp in either epoch milliseconds or RFC3339 text.
fn json_timestamp(value: Option<&serde_json::Value>, milliseconds: bool) -> Option<i64> {
    let value = value?;
    match value {
        serde_json::Value::Number(number) => {
            let raw = if milliseconds {
                number.as_i64()?
            } else {
                let seconds = number.as_f64()?;
                if !seconds.is_finite() || seconds < 0.0 {
                    return None;
                }
                (seconds * 1000.0).round() as i64
            };
            Some(raw)
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(seconds) = trimmed.parse::<f64>() {
                if !seconds.is_finite() || seconds < 0.0 {
                    return None;
                }
                let millis = if milliseconds {
                    seconds
                } else {
                    seconds * 1000.0
                };
                return Some((millis).round() as i64);
            }
            parse_iso8601(trimmed)
        }
        _ => None,
    }
}

/// The first non-blank JSON string among candidates.
fn json_text<'a>(row: &'a serde_json::Value, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|name| {
        row.get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    })
}

/// A JSON integer. Strings carrying whole numbers are accepted; booleans,
/// fractions and missing fields are not.
fn json_count(value: Option<&serde_json::Value>) -> Option<i64> {
    match value? {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiClient {
    Pi,
    Omp,
    Senpi,
    Kimchi,
}

impl PiClient {
    pub fn id(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Omp => "omp",
            Self::Senpi => "senpi",
            Self::Kimchi => "kimchi",
        }
    }

    pub fn root(self) -> Vec<PathBuf> {
        match self {
            Self::Pi => vec![crate::model::home_path(".pi/agent/sessions")],
            Self::Omp => vec![crate::model::home_path(".omp/agent/sessions")],
            Self::Senpi => {
                let home = crate::home_dir();
                let base = std::env::var("SENPI_CODING_AGENT_DIR")
                    .ok()
                    .and_then(|raw| environment_path(&raw, &home))
                    .unwrap_or_else(|| home.join(".senpi/agent"));
                let mut roots = vec![base.join("sessions")];
                if let Some(state) = std::env::var("SENPI_CODING_AGENT_SESSION_DIR")
                    .ok()
                    .and_then(|raw| environment_path(&raw, &home))
                {
                    roots.push(state);
                }
                roots
            }
            Self::Kimchi => {
                let home = crate::home_dir();
                let root = std::env::var("KIMCHI_CODING_AGENT_DIR")
                    .ok()
                    .and_then(|raw| environment_path(&raw, &home))
                    .unwrap_or_else(|| home.join(".config/kimchi/harness"));
                vec![root.join("sessions")]
            }
        }
    }
}

/// A path from an agent's environment, with `~` and `~/...` against Home.
/// An empty value is absent, not the current directory.
fn environment_path(raw: &str, home: &Path) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed == "~" {
        return Some(home.to_path_buf());
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        return Some(home.join(rest));
    }
    Some(PathBuf::from(trimmed))
}

/// Decodes the Pi transcript's four independent buckets. A total larger than
/// the named kinds stays unclassified here as a record remainder; only a
/// total standing alone is usable at all in the current ledger shape.
fn pi_usage(object: &serde_json::Value) -> Option<(TokenTally, i64)> {
    let object = object.as_object()?;
    let count = |name: &str| json_count(object.get(name));
    let input = count("input");
    let output = count("output");
    let cache_read = count("cacheRead");
    let cache_write = count("cacheWrite");
    let total = count("totalTokens");
    if input.is_none() && output.is_none() && cache_read.is_none() && cache_write.is_none() {
        return None;
    }

    let tally = TokenTally {
        input: input.unwrap_or(0),
        cache_write: cache_write.unwrap_or(0),
        cache_read: cache_read.unwrap_or(0),
        output: output.unwrap_or(0),
    };
    if [
        tally.input,
        tally.cache_write,
        tally.cache_read,
        tally.output,
    ]
    .iter()
    .copied()
    .any(|value| value < 0)
    {
        return None;
    }
    if let Some(total) = total {
        if total < 0 {
            return None;
        }
        let remainder = total.checked_sub(tally.total())?;
        if remainder > 0 {
            // The ledger currently carries classified kinds only. Rather than
            // putting the remainder into input, skip an object whose own total
            // disagrees with the buckets it named.
            return None;
        }
    }
    Some((tally, 0))
}

/// Reads the Pi-shaped clients. Branch copies fold by response id, or by the
/// message's own fields; Kimchi's older scheme keeps each session separate.
pub fn pi_ledger(client: PiClient) -> UsageLedger {
    let key = client.id();
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(key) {
                    return known.clone();
                }
            }
        }
    }

    let mut files = Vec::new();
    for root in client.root() {
        if !crate::scan::checkpoint() {
            break;
        }
        collect_jsonl(&root, &mut files);
    }
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(text) = crate::scan::read_to_string(&file) else {
            continue;
        };
        let mut session: Option<(String, Option<String>)> = None;
        for (index, line) in text.lines().enumerate() {
            if !crate::scan::checkpoint() {
                break;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let row_type = row.get("type").and_then(serde_json::Value::as_str);
            match row_type {
                Some("title") => {}
                Some("session") => {
                    session = json_text(&row, &["id"]).map(|id| {
                        (
                            id.to_string(),
                            json_text(&row, &["cwd"]).map(str::to_string),
                        )
                    });
                }
                Some("message") => {
                    let Some((_session_id, _project)) = session.as_ref() else {
                        continue;
                    };
                    let Some(message) = row.get("message").filter(|value| value.is_object()) else {
                        continue;
                    };
                    if message.get("role").and_then(serde_json::Value::as_str) != Some("assistant")
                    {
                        continue;
                    }
                    let Some(usage) = message.get("usage") else {
                        continue;
                    };
                    let Some((tally, _)) = pi_usage(usage) else {
                        continue;
                    };
                    if tally.total() == 0 {
                        continue;
                    }
                    let Some(model) = json_text(message, &["model"]).map(str::to_string) else {
                        continue;
                    };
                    let Some(at) = json_timestamp(
                        row.get("timestamp").or_else(|| message.get("timestamp")),
                        false,
                    ) else {
                        continue;
                    };
                    let provider = json_text(message, &["provider"]).unwrap_or("");
                    let response_id = json_text(message, &["responseId"]).map(str::to_string);
                    let identity = if client == PiClient::Kimchi {
                        format!("{key}:{}:{}", json_text(&row, &["id"]).unwrap_or(""), at)
                    } else {
                        match response_id {
                            Some(id) => format!("{key}:response:{id}"),
                            None => format!(
                                "{key}:message:{}:{}:{provider}:{model}:{}",
                                json_text(&row, &["id"]).unwrap_or(""),
                                at,
                                tally.total()
                            ),
                        }
                    };
                    if !seen.insert(identity) {
                        continue;
                    }
                    *buckets
                        .entry(slot_key_from_ms(at))
                        .or_default()
                        .entry(model)
                        .or_default() += tally;
                }
                _ => {}
            }
            let _ = index;
        }
    }

    ledger_from_slot_buckets(&buckets, key)
}

fn ledger_from_slot_buckets(
    buckets: &BTreeMap<String, BTreeMap<String, TokenTally>>,
    cache_key: &str,
) -> UsageLedger {
    if !crate::scan::checkpoint() {
        return UsageLedger::empty();
    }
    let built = if buckets.is_empty() {
        UsageLedger::empty()
    } else {
        priced(buckets, &model_prices::prices(), None)
    };
    let mut guard = memory();
    let entry = guard.get_or_insert_with(|| (Instant::now(), HashMap::new()));
    if crate::scan::checkpoint() {
        entry.1.insert(cache_key.to_string(), built.clone());
    }
    built
}

/// One parsed Pi transcript with the header facts Prime Agent's fork
/// accounting needs.
#[derive(Debug, Clone, Default)]
struct PrimeFile {
    path: String,
    id: Option<String>,
    cwd: Option<String>,
    parent_session: Option<String>,
    rlm_depth: Option<i64>,
    /// `row identity, response id, timestamp, provider, model, tally`
    messages: Vec<(
        String,
        Option<String>,
        i64,
        Option<String>,
        String,
        TokenTally,
    )>,
    /// `row identity, target id, child usage, aggregate usage`
    attributions: Vec<(String, Option<String>, TokenTally, TokenTally)>,
}

/// Parses one Pi-shaped file for Prime Agent. A missing session header makes
/// the file unusable rather than attributable to nobody.
fn prime_parse_file(path: &Path) -> PrimeFile {
    let mut file = PrimeFile {
        path: path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let Ok(text) = crate::scan::read_to_string(path) else {
        return file;
    };
    let mut header_seen = false;
    for line in text.lines() {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let row_type = row.get("type").and_then(serde_json::Value::as_str);
        match row_type {
            Some("title") => {}
            Some("session") => {
                file.id = json_text(&row, &["id"]).map(str::to_string);
                file.cwd = json_text(&row, &["cwd"]).map(str::to_string);
                file.parent_session = json_text(&row, &["parentSession"]).map(str::to_string);
                file.rlm_depth = json_count(row.get("rlmDepth"));
                header_seen = file.id.is_some();
            }
            Some("session_info") => {}
            Some("message") => {
                if !header_seen {
                    continue;
                }
                let Some(message) = row.get("message").filter(|value| value.is_object()) else {
                    continue;
                };
                if message.get("role").and_then(serde_json::Value::as_str) != Some("assistant") {
                    continue;
                }
                let Some((tally, _)) = message.get("usage").and_then(pi_usage) else {
                    continue;
                };
                if tally.total() == 0 {
                    continue;
                }
                let Some(model) = json_text(message, &["model"]).map(str::to_string) else {
                    continue;
                };
                let Some(at) = json_timestamp(
                    row.get("timestamp").or_else(|| message.get("timestamp")),
                    false,
                ) else {
                    continue;
                };
                file.messages.push((
                    json_text(&row, &["id"]).unwrap_or("").to_string(),
                    json_text(message, &["responseId"]).map(str::to_string),
                    at,
                    json_text(message, &["provider"]).map(str::to_string),
                    model,
                    tally,
                ));
            }
            Some("child_usage_attributed") => {
                if !header_seen {
                    continue;
                }
                let (Some(child), Some(aggregate)) =
                    (row.get("childUsage"), row.get("aggregateUsage"))
                else {
                    continue;
                };
                let (Some((child_usage, _)), Some((aggregate_usage, _))) =
                    (pi_usage(child), pi_usage(aggregate))
                else {
                    continue;
                };
                if child_usage.total() == 0 || aggregate_usage.total() == 0 {
                    continue;
                }
                file.attributions.push((
                    json_text(&row, &["id"]).unwrap_or("").to_string(),
                    json_text(&row, &["targetId"]).map(str::to_string),
                    child_usage,
                    aggregate_usage,
                ));
            }
            _ => {}
        }
    }
    file
}

/// Prime Agent's Pi RLM format with parent/child accounting. A parent whose
/// own tally equals the stated aggregate is reduced by exactly the stated
/// child usage; a missing child keeps its aggregate.
pub fn prime_agent_ledger() -> UsageLedger {
    const KEY: &str = "prime-agent";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let mut files = Vec::new();
    for root in [
        crate::model::home_path(".prime/agent/sessions"),
        crate::model::home_path(".prime/agent/session-artifacts"),
    ] {
        collect_jsonl(&root, &mut files);
    }
    files.sort();
    let parsed: Vec<PrimeFile> = files
        .iter()
        .take_while(|_| crate::scan::checkpoint())
        .map(|file| prime_parse_file(file))
        .collect();

    let mut parent_of: std::collections::HashMap<String, String> = Default::default();
    for file in &parsed {
        if !crate::scan::checkpoint() {
            break;
        }
        if let Some(parent) = &file.parent_session {
            parent_of.insert(file.path.clone(), parent.clone());
        }
    }
    let lineage_root = |start: &String| -> String {
        let mut chain: Vec<String> = Vec::new();
        let mut current = start.clone();
        loop {
            if let Some(index) = chain.iter().position(|path| path == &current) {
                return chain[index..].iter().min().cloned().unwrap_or(current);
            }
            chain.push(current.clone());
            let Some(parent) = parent_of.get(&current) else {
                return current;
            };
            current = parent.clone();
        }
    };
    let is_descendant = |child: &String, ancestor: &String| -> bool {
        let mut current = Some(child.clone());
        let mut visited = std::collections::HashSet::new();
        while let Some(path) = current {
            if &path == ancestor {
                return true;
            }
            if !visited.insert(path.clone()) {
                return false;
            }
            current = parent_of.get(&path).cloned();
        }
        false
    };

    let child_totals: Vec<(String, TokenTally)> = parsed
        .iter()
        .filter(|file| file.rlm_depth.unwrap_or(0) > 0)
        .map(|file| {
            (
                file.path.clone(),
                file.messages
                    .iter()
                    .fold(TokenTally::default(), |sum, message| sum + message.5),
            )
        })
        .collect();

    let mut consumed_children: std::collections::HashSet<String> = Default::default();
    let mut seen_attributions: std::collections::HashSet<String> = Default::default();
    let mut reductions: std::collections::HashMap<String, TokenTally> = Default::default();
    for file in &parsed {
        if !crate::scan::checkpoint() {
            break;
        }
        for (attribution_id, target, child_usage, aggregate_usage) in &file.attributions {
            if !crate::scan::checkpoint() {
                break;
            }
            let Some(target) = target else {
                continue;
            };
            let lineage = lineage_root(&file.path);
            let attribution_key = format!("{lineage}#{attribution_id}");
            if !seen_attributions.insert(attribution_key) {
                continue;
            }
            let Some(message) = file
                .messages
                .iter()
                .find(|message| &message.0 == target && &message.5 == aggregate_usage)
            else {
                continue;
            };
            let Some(index) = child_totals.iter().position(|(path, tally)| {
                !consumed_children.contains(path)
                    && tally == child_usage
                    && is_descendant(path, &lineage)
            }) else {
                continue;
            };
            consumed_children.insert(child_totals[index].0.clone());
            let identity = message
                .1
                .as_ref()
                .map(|id| format!("{KEY}:response:{id}"))
                .unwrap_or_else(|| {
                    format!(
                        "{KEY}:message:{}:{}:{}:{}",
                        message.0,
                        message.2,
                        message.3.as_deref().unwrap_or(""),
                        message.4
                    )
                });
            *reductions.entry(identity).or_default() += *child_usage;
        }
    }

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in &parsed {
        if !crate::scan::checkpoint() {
            break;
        }
        let Some(_session_id) = file.id.clone() else {
            continue;
        };
        for (row_id, response_id, at, provider, model, tally) in &file.messages {
            if !crate::scan::checkpoint() {
                break;
            }
            let provider = provider.as_deref().unwrap_or("");
            let identity = response_id
                .as_ref()
                .map(|id| format!("{KEY}:response:{id}"))
                .unwrap_or_else(|| {
                    format!(
                        "{KEY}:message:{row_id}:{at}:{provider}:{model}:{}",
                        tally.total()
                    )
                });
            if !seen.insert(identity) {
                continue;
            }
            let mut counted = *tally;
            if let Some(reduction) = response_id
                .as_ref()
                .map(|id| format!("{KEY}:response:{id}"))
                .or_else(|| {
                    Some(format!(
                        "{KEY}:message:{row_id}:{at}:{provider}:{model}:{}",
                        tally.total()
                    ))
                })
                .and_then(|identity| reductions.get(&identity))
            {
                counted.input = counted.input.saturating_sub(reduction.input);
                counted.cache_write = counted.cache_write.saturating_sub(reduction.cache_write);
                counted.cache_read = counted.cache_read.saturating_sub(reduction.cache_read);
                counted.output = counted.output.saturating_sub(reduction.output);
            }
            if counted.total() == 0 {
                continue;
            }
            *buckets
                .entry(slot_key_from_ms(*at))
                .or_default()
                .entry(model.clone())
                .or_default() += counted;
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// OpenClaw's current SQLite store plus its retained JSONL originals. The
/// same call can appear in both, so event id + timestamp + counts fold them
/// together; plumbing mirrors and zstd archives never count.
pub fn openclaw_ledger() -> UsageLedger {
    const KEY: &str = "openclaw";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let roots = [
        crate::model::home_path(".openclaw/agents"),
        crate::model::home_path(".clawdbot"),
        crate::model::home_path(".moltbot"),
        crate::model::home_path(".moldbot"),
    ];
    let mut sqlite_files = Vec::new();
    let mut jsonl_files = Vec::new();
    for root in &roots {
        if !crate::scan::checkpoint() {
            break;
        }
        collect_files(root, "sqlite", &mut sqlite_files);
        collect_files(root, "jsonl", &mut jsonl_files);
    }
    sqlite_files.sort();
    jsonl_files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    let carry = |row: &serde_json::Value| -> (Option<String>, Option<String>) {
        let kind = row.get("type").and_then(serde_json::Value::as_str);
        match kind {
            Some("model_change") => (
                json_text(row, &["modelId"]).map(str::to_string),
                json_text(row, &["provider"]).map(str::to_string),
            ),
            Some("custom")
                if row.get("customType").and_then(serde_json::Value::as_str)
                    == Some("model-snapshot") =>
            {
                let data = row.get("data");
                (
                    data.and_then(|data| json_text(data, &["modelId"]).map(str::to_string)),
                    data.and_then(|data| json_text(data, &["provider"]).map(str::to_string)),
                )
            }
            _ => (None, None),
        }
    };

    for database in sqlite_files.iter().filter(|file| {
        file.file_name()
            .map(|name| name == std::ffi::OsStr::new("openclaw-agent.sqlite"))
            .unwrap_or(false)
    }) {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(connection) = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            continue;
        };
        let mut metadata: std::collections::HashMap<String, Option<String>> = Default::default();
        if let Ok(mut statement) =
            connection.prepare("SELECT session_id, model FROM session_windows")
        {
            if let Ok(rows) = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0).unwrap_or_default(),
                    row.get::<_, String>(1).ok(),
                ))
            }) {
                for row in rows.flatten() {
                    if !crate::scan::checkpoint() {
                        break;
                    }
                    metadata.insert(row.0, row.1);
                }
            }
        }
        if metadata.is_empty() {
            if let Ok(mut statement) = connection.prepare("SELECT session_id, model FROM sessions")
            {
                if let Ok(rows) = statement.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0).unwrap_or_default(),
                        row.get::<_, String>(1).ok(),
                    ))
                }) {
                    for row in rows.flatten() {
                        if !crate::scan::checkpoint() {
                            break;
                        }
                        metadata.insert(row.0, row.1);
                    }
                }
            }
        }

        let Ok(mut statement) = connection.prepare(
            "SELECT session_id, seq, event_json FROM transcript_events \
             WHERE event_json LIKE '%\"usage\"%' OR event_json LIKE '%\"model_change\"%' \
             OR event_json LIKE '%model-snapshot%' ORDER BY session_id, seq",
        ) else {
            continue;
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, i64>(1).unwrap_or(0),
                row.get::<_, String>(2).unwrap_or_default(),
            ))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            if !crate::scan::checkpoint() {
                break;
            }
            let (session, seq, event_json) = row;
            let Ok(event) = serde_json::from_str::<serde_json::Value>(&event_json) else {
                continue;
            };
            let (mut model, provider) = carry(&event);
            let Some(message) = openclaw_message(&event) else {
                continue;
            };
            model = message
                .model
                .clone()
                .or(model)
                .or_else(|| metadata.get(&session).cloned().flatten())
                .or(provider
                    .as_ref()
                    .and_then(|provider| provider_placeholder(provider)));
            let Some(model) = model else {
                continue;
            };
            let Some(at) = message.at else {
                continue;
            };
            let event_id = message.event_id.unwrap_or_else(|| format!("seq-{seq}"));
            let identity = format!(
                "{KEY}:{}:{}:{}:{}",
                event_id, at, message.tally.input, message.tally.output
            );
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += message.tally;
            }
        }
    }

    for file in jsonl_files.iter().filter(|file| openclaw_jsonl(file)) {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(text) = crate::scan::read_to_string(file) else {
            continue;
        };
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut model_carry: Option<String> = None;
        for (index, line) in text.lines().enumerate() {
            if !crate::scan::checkpoint() {
                break;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let (changed_model, _) = carry(&row);
            if changed_model.is_some() {
                model_carry = changed_model;
            }
            let Some(message) = openclaw_message(&row) else {
                continue;
            };
            let Some(model) = message.model.clone().or_else(|| model_carry.clone()) else {
                continue;
            };
            let Some(at) = message.at else {
                continue;
            };
            let event_id = message.event_id.unwrap_or_else(|| format!("line-{index}"));
            let identity = format!(
                "{KEY}:{}:{}:{}:{}",
                event_id, at, message.tally.input, message.tally.output
            );
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += message.tally;
            }
        }
        let _ = stem;
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

struct OpenClawMessage {
    event_id: Option<String>,
    model: Option<String>,
    at: Option<i64>,
    tally: TokenTally,
}

fn openclaw_message(row: &serde_json::Value) -> Option<OpenClawMessage> {
    if row.get("type").and_then(serde_json::Value::as_str) != Some("message") {
        return None;
    }
    let message = row.get("message")?;
    if message.get("role").and_then(serde_json::Value::as_str) != Some("assistant") {
        return None;
    }
    let provider = json_text(message, &["provider"]).or(json_text(row, &["provider"]));
    let model = json_text(message, &["model"]).or(json_text(row, &["model"]));
    if provider == Some("openclaw")
        && matches!(model, Some("delivery-mirror") | Some("gateway-injected"))
    {
        return None;
    }
    if json_text(row, &["api"]) == Some("openclaw-transcript")
        || json_text(message, &["api"]) == Some("openclaw-transcript")
    {
        return None;
    }
    let usage = message.get("usage")?;
    let count = |name: &str| json_count(usage.get(name));
    let input = count("input");
    let output = count("output");
    let cache_read = count("cacheRead");
    let cache_write = count("cacheWrite");
    let reasoning = count("reasoningTokens").unwrap_or(0);
    let total = count("totalTokens");
    if input.is_none() && output.is_none() && cache_read.is_none() && cache_write.is_none() {
        if let Some(total) = total {
            if total <= 0 {
                return None;
            }
        } else {
            return None;
        }
    }
    let tally = TokenTally {
        input: input.unwrap_or(0),
        cache_write: cache_write.unwrap_or(0),
        cache_read: cache_read.unwrap_or(0),
        output: output.unwrap_or(reasoning),
    };
    if tally.total() == 0 {
        return None;
    }
    Some(OpenClawMessage {
        event_id: json_text(row, &["id"]).map(str::to_string),
        model: model.map(str::to_string),
        at: json_timestamp(message.get("timestamp").or(row.get("timestamp")), true),
        tally,
    })
}

fn openclaw_jsonl(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !name.contains(".jsonl") || name.ends_with(".zst") {
        return false;
    }
    let text = path.to_string_lossy();
    let normalized = text.replace('\\', "/");
    if !normalized.contains("/sessions/") && !normalized.contains("/session-sqlite-import-archive/")
    {
        return false;
    }
    !normalized.contains("/codex-home/") && !normalized.contains("/cli-auth/")
}

fn provider_placeholder(provider: &str) -> Option<String> {
    let provider = provider.trim().to_ascii_lowercase();
    if provider.is_empty() {
        return None;
    }
    if provider.contains("anthropic") || provider.contains("claude") {
        return Some("claude-unknown".into());
    }
    if provider.contains("openai") || provider.contains("gpt") {
        return Some("gpt-unknown".into());
    }
    if provider.contains("google") || provider.contains("gemini") {
        return Some("gemini-unknown".into());
    }
    if provider.contains("xai") || provider.contains("grok") {
        return Some("grok-unknown".into());
    }
    Some(format!("{provider}-unknown"))
}

/// Mux's cumulative per-workspace snapshot. One record per model key; the
/// snapshot is a whole reading, not an increment to be summed across copies.
pub fn mux_ledger() -> UsageLedger {
    const KEY: &str = "mux";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let mut files = Vec::new();
    collect_json(
        crate::model::home_path(".mux/sessions").as_path(),
        &mut files,
    );
    files.retain(|file| {
        file.file_name()
            .map(|name| name == std::ffi::OsStr::new("session-usage.json"))
            == Some(true)
    });
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(bytes) = crate::scan::read(&file) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(at) = value
            .get("lastRequest")
            .and_then(|request| json_timestamp(request.get("timestamp"), true))
            .filter(|at| *at > 0)
        else {
            continue;
        };
        let Some(by_model) = value.get("byModel").and_then(serde_json::Value::as_object) else {
            continue;
        };
        let session = file
            .parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fallback_model = value
            .get("lastRequest")
            .and_then(|request| json_text(request, &["model"]).map(str::to_string));
        for (key, entry) in by_model {
            if !crate::scan::checkpoint() {
                break;
            }
            let bucket = |name: &str| {
                entry
                    .get(name)
                    .and_then(|bucket| json_count(bucket.get("tokens")))
                    .unwrap_or(0)
            };
            // Reasoning sits beside output with no total to prove containment,
            // so the reported output is kept and reasoning is not counted.
            let tally = TokenTally {
                input: bucket("input"),
                cache_write: bucket("cacheCreate"),
                cache_read: bucket("cached"),
                output: bucket("output"),
            };
            if tally.total() == 0 {
                continue;
            }
            let Some(model) = key
                .split_once(':')
                .map(|(_, model)| model.trim().to_string())
                .filter(|model| !model.is_empty())
                .or_else(|| fallback_model.clone())
            else {
                continue;
            };
            let identity = format!("{KEY}:{session}:{key}");
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += tally;
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// JetBrains Junie's event stream. Only `LlmResponseMetadataEvent` rows with
/// `modelUsage` count; a positive latency dates the row at the call's start.
pub fn junie_ledger() -> UsageLedger {
    const KEY: &str = "junie";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let mut files = Vec::new();
    collect_jsonl(
        crate::model::home_path(".junie/sessions").as_path(),
        &mut files,
    );
    files.retain(|file| {
        file.file_name()
            .map(|name| name == std::ffi::OsStr::new("events.jsonl"))
            == Some(true)
    });
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(text) = crate::scan::read_to_string(&file) else {
            continue;
        };
        let session = file
            .parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let session_start = junie_session_time(&session);
        for row in text.lines() {
            if !crate::scan::checkpoint() {
                break;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(row) else {
                continue;
            };
            let Some(agent_event) = row.get("event").and_then(|event| event.get("agentEvent"))
            else {
                continue;
            };
            if json_text(agent_event, &["kind"]) != Some("LlmResponseMetadataEvent") {
                continue;
            }
            let Some(usages) = agent_event
                .get("modelUsage")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let end = json_timestamp(row.get("timestampMs"), true).filter(|at| *at > 0);
            for (index, entry) in usages.iter().enumerate() {
                if !crate::scan::checkpoint() {
                    break;
                }
                let Some(model) = json_text(entry, &["model"]).map(str::to_string) else {
                    continue;
                };
                let latency = json_count(entry.get("time")).unwrap_or(0);
                let at = match end {
                    Some(end) if latency > 0 => end.saturating_sub(latency).max(end.min(1)),
                    Some(end) => end,
                    None => session_start.unwrap_or_default(),
                };
                if at <= 0 {
                    continue;
                }
                let merged = |aliases: &[&str]| -> i64 {
                    aliases
                        .iter()
                        .find_map(|alias| json_count(entry.get(*alias)).filter(|value| *value > 0))
                        .unwrap_or(0)
                };
                let tally = TokenTally {
                    input: merged(&["inputTokens", "input"]),
                    cache_write: merged(&[
                        "cacheCreateTokens",
                        "cacheCreationInputTokens",
                        "cacheWrite",
                    ]),
                    cache_read: merged(&["cacheInputTokens", "cacheReadInputTokens", "cacheRead"]),
                    output: merged(&["outputTokens", "output"]),
                };
                if tally.total() == 0 {
                    continue;
                }
                let identity = format!(
                    "{KEY}:{session}:{at}:{model}:{}:{}:{}:{}:{index}",
                    tally.input, tally.cache_write, tally.cache_read, tally.output
                );
                if seen.insert(identity) {
                    *buckets
                        .entry(slot_key_from_ms(at))
                        .or_default()
                        .entry(model)
                        .or_default() += tally;
                }
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

fn junie_session_time(id: &str) -> Option<i64> {
    let marker = id.find("session-")?;
    let stamp = &id[marker + "session-".len()..];
    let stamp = stamp.get(..13)?;
    let at = chrono::Local.with_ymd_and_hms(0, 1, 1, 0, 0, 0).single()?;
    let _ = at;
    let parsed = chrono::NaiveDateTime::parse_from_str(stamp, "%y%m%d-%H%M%S").ok()?;
    use chrono::TimeZone;
    let at = chrono::Local
        .from_local_datetime(&parsed)
        .single()
        .or_else(|| chrono::Local.from_local_datetime(&parsed).earliest())?;
    Some(at.timestamp_millis())
}

/// Augment's completed chat turns. A streamed turn is cumulative across its
/// response nodes, so the last non-empty `token_usage` is the turn total.
pub fn augment_ledger() -> UsageLedger {
    const KEY: &str = "augment";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let mut files = Vec::new();
    collect_json(
        crate::model::home_path(".augment/sessions").as_path(),
        &mut files,
    );
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(bytes) = crate::scan::read(&file) else {
            continue;
        };
        let Ok(session_value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let session = json_text(&session_value, &["sessionId"])
            .map(str::to_string)
            .unwrap_or_else(|| {
                file.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        let agent_model = session_value
            .get("agentState")
            .and_then(|state| json_text(state, &["modelId"]).map(str::to_string));
        let Some(turns) = session_value
            .get("chatHistory")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for (index, turn) in turns.iter().enumerate() {
            if !crate::scan::checkpoint() {
                break;
            }
            if turn.get("completed").and_then(serde_json::Value::as_bool) != Some(true) {
                continue;
            }
            let Some(exchange) = turn.get("exchange") else {
                continue;
            };
            let Some(at) = json_timestamp(turn.get("finishedAt"), false).filter(|at| *at > 0)
            else {
                continue;
            };
            let Some(nodes) = exchange
                .get("response_nodes")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let usage = nodes.iter().rev().find_map(|node| {
                let usage = node.get("token_usage")?;
                let count = |name: &str| json_count(usage.get(name)).unwrap_or(0);
                let total = count("input_tokens")
                    + count("cache_creation_input_tokens")
                    + count("cache_read_input_tokens")
                    + count("output_tokens");
                (total > 0).then(|| usage.clone())
            });
            let Some(usage) = usage else {
                continue;
            };
            let count = |name: &str| json_count(usage.get(name)).unwrap_or(0);
            let tally = TokenTally {
                input: count("input_tokens"),
                cache_write: count("cache_creation_input_tokens"),
                cache_read: count("cache_read_input_tokens"),
                output: count("output_tokens"),
            };
            if tally.total() == 0 {
                continue;
            }
            let Some(model) = json_text(exchange, &["model_id"])
                .map(str::to_string)
                .or_else(|| agent_model.clone())
            else {
                continue;
            };
            let identity = json_text(exchange, &["request_id"])
                .or(json_text(turn, &["sequenceId"]))
                .map(|id| format!("{KEY}:{session}:{id}"))
                .unwrap_or_else(|| format!("{KEY}:{session}:{index}"));
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += tally;
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// Jcode's sessions plus their append-only journals. Cache shape is decided
/// only by an explicit schema marker; ambiguous input stays uncounted rather
/// than priced fresh.
pub fn jcode_ledger() -> UsageLedger {
    const KEY: &str = "jcode";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let mut base = crate::model::home_path(".jcode");
    if let Some(raw) = std::env::var("JCODE_HOME").ok() {
        if let Some(path) = environment_path(&raw, &crate::home_dir()) {
            base = path;
        }
    }
    let mut files = Vec::new();
    collect_files(base.join("sessions").as_path(), "json", &mut files);
    files.retain(|file| {
        file.file_name()
            .map(|name| name.to_string_lossy().starts_with("session_"))
            == Some(true)
    });
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(bytes) = crate::scan::read(&file) else {
            continue;
        };
        let Ok(session) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let session_id = json_text(&session, &["id"])
            .map(str::to_string)
            .unwrap_or_else(|| {
                file.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        let session_model = json_text(&session, &["model"]).map(str::to_string);
        let mut messages: Vec<(serde_json::Value, Option<String>)> = session
            .get("messages")
            .and_then(serde_json::Value::as_array)
            .map(|rows| {
                rows.iter()
                    .cloned()
                    .map(|row| (row, session_model.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let journal = file.with_file_name(format!(
            "{}.journal.jsonl",
            file.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        ));
        if let Ok(text) = crate::scan::read_to_string(&journal) {
            let mut journal_model = session_model.clone();
            for line in text.lines() {
                if !crate::scan::checkpoint() {
                    break;
                }
                let Ok(line) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                if let Some(meta) = line.get("meta") {
                    if let Some(model) = json_text(meta, &["model"]).map(str::to_string) {
                        journal_model = Some(model);
                    }
                }
                if let Some(appended) = line
                    .get("append_messages")
                    .and_then(serde_json::Value::as_array)
                {
                    messages.extend(
                        appended
                            .iter()
                            .cloned()
                            .map(|row| (row, journal_model.clone())),
                    );
                }
            }
        }

        for (message, carried_model) in messages {
            if !crate::scan::checkpoint() {
                break;
            }
            let Some(usage) = message.get("token_usage") else {
                continue;
            };
            let Some(at) = json_timestamp(message.get("timestamp"), false).filter(|at| *at > 0)
            else {
                continue;
            };
            let Some(model) = json_text(&message, &["model"])
                .map(str::to_string)
                .or(carried_model)
                .or_else(|| session_model.clone())
            else {
                continue;
            };
            let count = |name: &str| json_count(usage.get(name)).unwrap_or(0);
            let input = count("input_tokens");
            let cache_write = count("cache_creation_input_tokens");
            let cache_read = count("cache_read_input_tokens");
            let output = count("output_tokens");
            let tally = if usage.get("cache_creation_input_tokens").is_some() {
                TokenTally {
                    input,
                    cache_write,
                    cache_read,
                    output,
                }
            } else if [
                "prompt_tokens_details",
                "promptTokensDetails",
                "input_tokens_details",
                "inputTokensDetails",
            ]
            .iter()
            .any(|name| usage.get(*name).is_some())
            {
                TokenTally {
                    input: input.saturating_sub(cache_read.min(input)),
                    cache_write,
                    cache_read,
                    output,
                }
            } else if cache_read > 0 {
                TokenTally {
                    input: 0,
                    cache_write: 0,
                    cache_read: 0,
                    output,
                }
            } else {
                TokenTally {
                    input,
                    cache_write: 0,
                    cache_read: 0,
                    output,
                }
            };
            if tally.total() == 0 {
                continue;
            }
            let name = json_text(&message, &["id"])
                .map(str::to_string)
                .unwrap_or_else(|| {
                    format!(
                        "{}:{at}:{}:{}:{}:{}",
                        model, input, cache_write, cache_read, output
                    )
                });
            let identity = format!("{KEY}:{session_id}:{name}");
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += tally;
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// Gajae Code's Pi-like JSONL with byte-identical mirror folding.
pub fn gjc_ledger() -> UsageLedger {
    const KEY: &str = "gjc";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let home = crate::home_dir();
    let mut roots = Vec::new();
    let agent = std::env::var("GJC_CODING_AGENT_DIR")
        .ok()
        .and_then(|raw| environment_path(&raw, &home))
        .unwrap_or_else(|| home.join(".gjc/agent"));
    roots.push(agent.join("sessions"));
    for key in ["GJC_CONFIG_DIR", "PI_CONFIG_DIR"] {
        if let Some(raw) = std::env::var(key)
            .ok()
            .and_then(|raw| environment_path(&raw, &home))
        {
            roots.push(raw.join("agent/sessions"));
        }
    }
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .and_then(|raw| environment_path(&raw, &home))
        .unwrap_or_else(|| home.join(".local/share"));
    roots.push(data_home.join("gjc/sessions"));

    let mut files = Vec::new();
    for root in &roots {
        if !crate::scan::checkpoint() {
            break;
        }
        collect_jsonl(root, &mut files);
    }
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    let mut seen_files: std::collections::HashMap<String, std::collections::HashSet<String>> =
        Default::default();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(text) = crate::scan::read_to_string(&file) else {
            continue;
        };
        let digest = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            text.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        };
        let mut header_id: Option<String> = None;
        for line in text.lines() {
            if !crate::scan::checkpoint() {
                break;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if row.get("type").and_then(serde_json::Value::as_str) == Some("session")
                && header_id.is_none()
            {
                header_id = json_text(&row, &["id"]).map(str::to_string);
            }
        }
        if let Some(header_id) = &header_id {
            let mirror = format!(
                "{header_id}|{}",
                file.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            );
            if !seen_files.entry(mirror).or_default().insert(digest) {
                continue;
            }
        }
        let session = header_id.unwrap_or_else(|| {
            file.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        for (index, line) in text.lines().enumerate() {
            if !crate::scan::checkpoint() {
                break;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if row.get("type").and_then(serde_json::Value::as_str) != Some("message") {
                continue;
            }
            let Some(message) = row.get("message") else {
                continue;
            };
            if json_text(message, &["role"])
                .map(str::to_ascii_lowercase)
                .as_deref()
                != Some("assistant")
            {
                continue;
            }
            let Some(usage) = message.get("usage") else {
                continue;
            };
            let Some(model) = json_text(message, &["model"]).map(str::to_string) else {
                continue;
            };
            let at = json_timestamp(message.get("timestamp"), true)
                .filter(|at| *at > 0)
                .or_else(|| json_timestamp(row.get("timestamp"), false));
            let Some(at) = at else {
                continue;
            };
            let count = |name: &str| json_count(usage.get(name)).unwrap_or(0);
            let tally = TokenTally {
                input: count("input"),
                cache_write: count("cacheWrite"),
                cache_read: count("cacheRead"),
                output: count("output"),
            };
            if tally.total() == 0 {
                continue;
            }
            let identity = json_text(&row, &["id"])
                .map(|id| format!("{KEY}:{session}:{id}"))
                .unwrap_or_else(|| format!("{KEY}:{session}:{file:?}:{index}"));
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += tally;
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// Codebuff's chat logs under its manicode trees. The same provider counts
/// are copied into several places; each field is taken from the first
/// non-zero source, so a duplicate never doubles and a zero never masks.
pub fn codebuff_ledger() -> UsageLedger {
    const KEY: &str = "codebuff";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let home = crate::home_dir();
    let roots = match std::env::var("CODEBUFF_DATA_DIR")
        .ok()
        .and_then(|raw| environment_path(&raw, &home))
    {
        Some(overridden) => vec![overridden],
        None => vec![
            home.join(".config/manicode"),
            home.join(".config/manicode-dev"),
            home.join(".config/manicode-staging"),
        ],
    };
    let mut files = Vec::new();
    for root in &roots {
        if !crate::scan::checkpoint() {
            break;
        }
        collect_json(root, &mut files);
    }
    files.retain(|file| {
        file.file_name()
            .map(|name| name == std::ffi::OsStr::new("chat-messages.json"))
            == Some(true)
    });
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(bytes) = crate::scan::read(&file) else {
            continue;
        };
        let Ok(messages) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(messages) = messages.as_array() else {
            continue;
        };
        let (channel, project, chat_id) = codebuff_location(&file);
        let session = format!("{channel}/{project}/{chat_id}");
        for (index, message) in messages.iter().enumerate() {
            if !crate::scan::checkpoint() {
                break;
            }
            let variant = json_text(message, &["variant"]).map(str::to_ascii_lowercase);
            let role = json_text(message, &["role"]).map(str::to_ascii_lowercase);
            if !matches!(
                variant.as_deref(),
                Some("ai") | Some("agent") | Some("assistant")
            ) && !matches!(
                role.as_deref(),
                Some("ai") | Some("agent") | Some("assistant")
            ) {
                continue;
            }
            let Some(sources) = codebuff_usage_sources(message) else {
                continue;
            };
            let merged = |aliases: &[&str]| -> i64 {
                aliases
                    .iter()
                    .find_map(|alias| {
                        sources.iter().find_map(|source| {
                            json_count(source.get(*alias)).filter(|value| *value > 0)
                        })
                    })
                    .unwrap_or(0)
            };
            let mut cache_read = merged(&[
                "cacheReadInputTokens",
                "cache_read_input_tokens",
                "cachedTokensCreated",
                "cached_tokens_created",
            ]);
            if cache_read == 0 {
                cache_read = sources
                    .iter()
                    .find_map(|source| {
                        ["promptTokensDetails", "prompt_tokens_details"]
                            .iter()
                            .find_map(|key| {
                                let details = source.get(*key)?;
                                json_count(details.get("cachedTokens"))
                                    .or_else(|| json_count(details.get("cached_tokens")))
                                    .filter(|value| *value > 0)
                            })
                    })
                    .unwrap_or(0);
            }
            let tally = TokenTally {
                input: merged(&[
                    "inputTokens",
                    "input_tokens",
                    "promptTokens",
                    "prompt_tokens",
                ]),
                cache_write: merged(&[
                    "cacheCreationInputTokens",
                    "cache_creation_input_tokens",
                    "cacheCreationTokens",
                    "cache_creation_tokens",
                ]),
                cache_read,
                output: merged(&[
                    "outputTokens",
                    "output_tokens",
                    "completionTokens",
                    "completion_tokens",
                ]),
            };
            if tally.total() == 0 {
                continue;
            }
            let Some(model) = codebuff_model(message, &sources) else {
                continue;
            };
            let Some(at) = codebuff_timestamp(message, &chat_id) else {
                continue;
            };
            let identity = json_text(message, &["id"])
                .map(|id| format!("{KEY}:{session}:{id}"))
                .unwrap_or_else(|| {
                    format!(
                        "{KEY}:{session}:{index}:{at}:{model}:{}:{}:{}:{}",
                        tally.input, tally.cache_write, tally.cache_read, tally.output
                    )
                });
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += tally;
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

fn codebuff_location(file: &Path) -> (String, String, String) {
    let chat_id = file
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let components: Vec<String> = file
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if let Some(index) = components.iter().rposition(|segment| segment == "projects") {
        if index > 0 && index + 1 < components.len() {
            let channel = components[index - 1].clone();
            let project = components[index + 1].clone();
            return (channel, project, chat_id);
        }
    }
    let chats = file.parent().unwrap_or(Path::new(""));
    let project = chats
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let channel = chats
        .parent()
        .and_then(|parent| parent.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    (channel, project, chat_id)
}

fn codebuff_usage_sources(message: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    let metadata = message.get("metadata")?;
    let mut sources = Vec::new();
    if let Some(usage) = metadata.get("usage") {
        sources.push(usage.clone());
    }
    if let Some(usage) = metadata
        .get("codebuff")
        .and_then(|codebuff| codebuff.get("usage"))
    {
        sources.push(usage.clone());
    }
    if let Some(history) = metadata
        .get("runState")
        .and_then(|state| state.get("sessionState"))
        .and_then(|state| state.get("mainAgentState"))
        .and_then(|agent| agent.get("messageHistory"))
        .and_then(serde_json::Value::as_array)
    {
        for row in history.iter().rev() {
            if !crate::scan::checkpoint() {
                break;
            }
            if let Some(usage) = row
                .get("providerOptions")
                .and_then(|options| options.get("usage"))
            {
                sources.push(usage.clone());
            }
            if let Some(usage) = row
                .get("providerOptions")
                .and_then(|options| options.get("codebuff"))
                .and_then(|codebuff| codebuff.get("usage"))
            {
                sources.push(usage.clone());
            }
        }
    }
    (!sources.is_empty()).then_some(sources)
}

fn codebuff_model(message: &serde_json::Value, sources: &[serde_json::Value]) -> Option<String> {
    if let Some(model) = message
        .get("metadata")
        .and_then(|metadata| json_text(metadata, &["model"]).map(str::to_string))
    {
        return Some(model);
    }
    if let Some(model) = message
        .get("metadata")
        .and_then(|metadata| metadata.get("runState"))
        .and_then(|state| state.get("sessionState"))
        .and_then(|state| state.get("mainAgentState"))
        .and_then(|agent| agent.get("messageHistory"))
        .and_then(serde_json::Value::as_array)
        .and_then(|history| {
            history.iter().rev().find_map(|row| {
                row.get("providerOptions")
                    .and_then(|options| options.get("codebuff"))
                    .and_then(|codebuff| json_text(codebuff, &["model"]).map(str::to_string))
            })
        })
    {
        return Some(model);
    }
    sources
        .iter()
        .find_map(|source| json_text(source, &["model"]).map(str::to_string))
}

fn codebuff_timestamp(message: &serde_json::Value, chat_id: &str) -> Option<i64> {
    json_timestamp(message.get("timestamp"), false)
        .filter(|at| *at > 0)
        .or_else(|| json_timestamp(message.get("createdAt"), false).filter(|at| *at > 0))
        .or_else(|| {
            message
                .get("metadata")
                .and_then(|metadata| json_timestamp(metadata.get("timestamp"), false))
                .filter(|at| *at > 0)
        })
        .or_else(|| iso_from_chat_id(chat_id))
}

/// An ISO 8601 chat id whose time separators were written as dashes.
fn iso_from_chat_id(chat_id: &str) -> Option<i64> {
    let marker = chat_id.find('T')?;
    let head = &chat_id[..marker];
    let tail = chat_id[marker + 1..].replace('-', ":");
    parse_iso8601(&format!("{head}T{tail}"))
}

/// Fx's per-session cumulative snapshot: one record per model, timestamped by
/// the session sidecar. Reasoning beside output without a total stays out.
pub fn fx_ledger() -> UsageLedger {
    const KEY: &str = "fx";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    let sidecars = collect_sidecars(crate::model::home_path(".fx/sessions").as_path());
    for (session_dir, sidecar) in sidecars {
        if !crate::scan::checkpoint() {
            break;
        }
        let usage_file = session_dir.join("usage-v2.json");
        let Ok(bytes) = crate::scan::read(&usage_file) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(snapshot) = value.get("snapshot") else {
            continue;
        };
        let session = json_text(&value, &["session_id"])
            .map(str::to_string)
            .unwrap_or_else(|| {
                session_dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
        let Some(at) = json_timestamp(sidecar.get("updated_at_ms"), true)
            .filter(|at| *at > 0)
            .or_else(|| json_timestamp(sidecar.get("created_at_ms"), true).filter(|at| *at > 0))
        else {
            continue;
        };
        let count =
            |object: &serde_json::Value, name: &str| json_count(object.get(name)).unwrap_or(0);
        let models = snapshot.get("models").and_then(serde_json::Value::as_array);
        if let Some(models) = models.filter(|models| !models.is_empty()) {
            for entry in models {
                if !crate::scan::checkpoint() {
                    break;
                }
                let tally = TokenTally {
                    input: count(entry, "input_tokens"),
                    cache_write: count(entry, "cache_write_tokens"),
                    cache_read: count(entry, "cache_read_tokens"),
                    output: count(entry, "output_tokens"),
                };
                if tally.total() == 0 {
                    continue;
                }
                let model = json_text(entry, &["model"])
                    .unwrap_or("fx-unknown")
                    .to_string();
                let identity = format!("{KEY}:{session}:{model}");
                if seen.insert(identity) {
                    *buckets
                        .entry(slot_key_from_ms(at))
                        .or_default()
                        .entry(model)
                        .or_default() += tally;
                }
            }
        } else {
            let tally = TokenTally {
                input: count(snapshot, "input_tokens"),
                cache_write: count(snapshot, "cache_write_tokens"),
                cache_read: count(snapshot, "cache_read_tokens"),
                output: count(snapshot, "output_tokens"),
            };
            if tally.total() > 0 {
                let identity = format!("{KEY}:{session}:fx-unknown");
                if seen.insert(identity) {
                    *buckets
                        .entry(slot_key_from_ms(at))
                        .or_default()
                        .entry("fx-unknown".to_string())
                        .or_default() += tally;
                }
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// A session directory with its parsed `session.json` sidecar, where one
/// exists and is an object.
fn collect_sidecars(root: &Path) -> Vec<(PathBuf, serde_json::Value)> {
    let mut sidecars = Vec::new();
    if !crate::scan::checkpoint() {
        return sidecars;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return sidecars;
    };
    for entry in entries.flatten() {
        if !crate::scan::checkpoint() {
            break;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(text) = crate::scan::read_to_string(path.join("session.json")) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value.is_object() {
            sidecars.push((path, value));
        }
    }
    sidecars.sort_by(|left, right| left.0.cmp(&right.0));
    sidecars
}

/// Reasonix's daily JSONL stats. `turn` marks a marker row, not a call; a
/// bare total names no kind and stays out rather than becoming a zero.
pub fn reasonix_ledger() -> UsageLedger {
    const KEY: &str = "reasonix";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let home = crate::home_dir();
    let root = if let Some(state) = std::env::var("REASONIX_STATE_HOME")
        .ok()
        .and_then(|raw| environment_path(&raw, &home))
    {
        state
    } else {
        let base = std::env::var("REASONIX_HOME")
            .ok()
            .and_then(|raw| environment_path(&raw, &home))
            .unwrap_or_else(|| home.join(".reasonix"));
        base.join("stats")
    };
    let mut files = Vec::new();
    collect_jsonl(&root, &mut files);
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(text) = crate::scan::read_to_string(&file) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if !crate::scan::checkpoint() {
                break;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if row.get("turn").and_then(serde_json::Value::as_bool) == Some(true) {
                continue;
            }
            let Some(model) = json_text(&row, &["model"]).map(str::to_string) else {
                continue;
            };
            let total = json_count(row.get("total")).unwrap_or(0);
            let requests = json_count(row.get("requests")).unwrap_or(0);
            if total <= 0 && requests <= 0 {
                continue;
            }
            let Some(at) = json_timestamp(row.get("ts"), false).filter(|at| *at > 0) else {
                continue;
            };
            let prompt = json_count(row.get("prompt"));
            let completion = json_count(row.get("completion"));
            let Some(tally) = (|| {
                if prompt.is_none() && completion.is_none() {
                    return None;
                }
                let cache_read = json_count(row.get("cache_hit")).unwrap_or(0);
                let miss = json_count(row.get("cache_miss")).unwrap_or(0);
                Some(TokenTally {
                    input: if miss > 0 {
                        miss
                    } else {
                        prompt.unwrap_or(0).saturating_sub(cache_read)
                    },
                    cache_write: 0,
                    cache_read,
                    output: completion.unwrap_or(0),
                })
            })() else {
                continue;
            };
            if tally.total() == 0 {
                continue;
            }
            let identity = format!(
                "{KEY}:{}:{index}:{requests}:{total}",
                file.to_string_lossy()
            );
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += tally;
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// LM Studio's pretty-printed server logs. Usage appears as a balanced JSON
/// object after a `"usage"` key; the model id and local timestamp must both
/// appear in the preceding text, or the block is skipped.
pub fn lmstudio_ledger() -> UsageLedger {
    const KEY: &str = "lmstudio";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let home = crate::home_dir();
    let root = std::env::var("LM_STUDIO_HOME")
        .ok()
        .and_then(|raw| environment_path(&raw, &home))
        .unwrap_or_else(|| home.join(".lmstudio"))
        .join("server-logs");
    let mut files = Vec::new();
    collect_files(&root, "log", &mut files);
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let Ok(text) = crate::scan::read_to_string(&file) else {
            continue;
        };
        let mut cursor = 0usize;
        while let Some(marker) = text[cursor..].find(r#""usage""#) {
            if !crate::scan::checkpoint() {
                break;
            }
            let key_start = cursor + marker;
            let after_key = key_start + r#""usage""#.len();
            let Some((json_start, json_end)) = balanced_object(&text[after_key..]) else {
                cursor = after_key;
                continue;
            };
            cursor = after_key + json_end;
            let Ok(usage) = serde_json::from_str::<serde_json::Value>(
                &text[after_key + json_start..after_key + json_end],
            ) else {
                continue;
            };
            let context = text
                .get(..key_start)
                .and_then(|head| head.get(head.len().saturating_sub(4096)..))
                .unwrap_or("")
                .to_string();
            let Some(at) = last_log_time(&context) else {
                continue;
            };
            let Some(model) = last_quoted_field(&context, "model") else {
                continue;
            };
            let count = |names: &[&str]| -> Option<i64> {
                names.iter().find_map(|name| json_count(usage.get(*name)))
            };
            let prompt = count(&[
                "prompt_tokens",
                "promptTokens",
                "input_tokens",
                "inputTokens",
            ]);
            let completion = count(&[
                "completion_tokens",
                "completionTokens",
                "output_tokens",
                "outputTokens",
            ]);
            if prompt.is_none() && completion.is_none() {
                continue;
            }
            let prompt_tokens = prompt.unwrap_or(0);
            let completion_tokens = completion.unwrap_or(0);
            let details = |names: &[&str]| -> Option<serde_json::Value> {
                names.iter().find_map(|name| usage.get(*name)).cloned()
            };
            let prompt_details = details(&[
                "prompt_tokens_details",
                "input_tokens_details",
                "inputTokensDetails",
            ]);
            let cached = prompt_details
                .as_ref()
                .and_then(|details| json_count(details.get("cached_tokens")))
                .or_else(|| json_count(usage.get("cached_tokens")))
                .unwrap_or(0);
            let creation = prompt_details
                .as_ref()
                .and_then(|details| json_count(details.get("cache_creation_input_tokens")))
                .or_else(|| json_count(usage.get("cache_creation_input_tokens")))
                .unwrap_or(0);
            let cache_read = cached.min(prompt_tokens);
            let cache_write = creation.min(prompt_tokens.saturating_sub(cache_read));
            let reported_total = count(&["total_tokens", "totalTokens"]).unwrap_or(0);
            let total = reported_total.max(prompt_tokens + completion_tokens);
            let tally = TokenTally {
                input: total
                    .saturating_sub(completion_tokens)
                    .saturating_sub(cache_read)
                    .saturating_sub(cache_write),
                cache_write,
                cache_read,
                output: completion_tokens,
            };
            if tally.total() == 0 {
                continue;
            }
            let identity = last_quoted_field(&context, "id")
                .map(|id| format!("{KEY}:{id}"))
                .unwrap_or_else(|| {
                    format!(
                        "{KEY}:{}:{key_start}:{model}:{}:{}:{}:{}",
                        file.to_string_lossy(),
                        tally.input,
                        tally.cache_write,
                        tally.cache_read,
                        tally.output
                    )
                });
            if seen.insert(identity) {
                *buckets
                    .entry(slot_key_from_ms(at))
                    .or_default()
                    .entry(model)
                    .or_default() += tally;
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

/// A balanced `{ ... }` following a colon; returns the JSON span offsets.
fn balanced_object(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() || bytes[index] != b':' {
        return None;
    }
    index += 1;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() || bytes[index] != b'{' {
        return None;
    }
    let start = index;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some((start, index + 1));
            }
        }
        index += 1;
    }
    None
}

fn last_quoted_field(text: &str, name: &str) -> Option<String> {
    let needle = format!(r#""{name}""#);
    let mut result = None;
    let mut from = 0usize;
    while let Some(offset) = text[from..].find(&needle) {
        let start = from + offset + needle.len();
        let rest = text[start..].trim_start();
        if let Some(value) = rest.strip_prefix(':') {
            let value = value.trim_start();
            if let Some(quoted) = value.strip_prefix('"') {
                if let Some(end) = quoted.find('"') {
                    result = Some(quoted[..end].to_string());
                }
            }
        }
        from = start;
    }
    result.filter(|value| !value.trim().is_empty())
}

fn last_log_time(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let mut result = None;
    for offset in 0..bytes.len().saturating_sub(19) {
        if bytes[offset + 4] != b'-'
            || bytes[offset + 7] != b'-'
            || (bytes[offset + 10] != b' ' && bytes[offset + 10] != b'T')
            || bytes[offset + 13] != b':'
            || bytes[offset + 16] != b':'
        {
            continue;
        }
        if offset > 0 && bytes[offset - 1].is_ascii_digit() {
            continue;
        }
        if !bytes[offset..offset + 4].iter().all(u8::is_ascii_digit)
            || !bytes[offset + 5..offset + 7].iter().all(u8::is_ascii_digit)
            || !bytes[offset + 8..offset + 10]
                .iter()
                .all(u8::is_ascii_digit)
            || !bytes[offset + 11..offset + 13]
                .iter()
                .all(u8::is_ascii_digit)
            || !bytes[offset + 14..offset + 16]
                .iter()
                .all(u8::is_ascii_digit)
            || !bytes[offset + 17..offset + 19]
                .iter()
                .all(u8::is_ascii_digit)
        {
            continue;
        }
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(
            &text[offset..offset + 19].replace('T', " "),
            "%Y-%m-%d %H:%M:%S",
        ) {
            use chrono::TimeZone;
            let at = chrono::Local
                .from_local_datetime(&naive)
                .single()
                .or_else(|| chrono::Local.from_local_datetime(&naive).earliest());
            if let Some(at) = at {
                result = Some(at.timestamp_millis());
            }
        }
    }
    result
}

/// Gemini CLI's chat JSON and headless JSONL shapes. Session JSON's cache
/// overlap is proven only by a total equal to the non-cache sum; headless
/// prompt-style inputs are cache-inclusive. A bare total is not assigned to
/// a kind here.
pub fn gemini_ledger() -> UsageLedger {
    const KEY: &str = "gemini";
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(KEY) {
                    return known.clone();
                }
            }
        }
    }

    let mut files = Vec::new();
    let root = crate::model::home_path(".gemini");
    let root = std::env::var("GEMINI_CLI_HOME")
        .ok()
        .and_then(|raw| environment_path(&raw, &crate::home_dir()))
        .unwrap_or(root);
    collect_json(&root, &mut files);
    collect_jsonl(&root, &mut files);
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let extension = file
            .extension()
            .map(|ext| ext.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_chat_json = extension == "json"
            && file
                .components()
                .any(|component| component.as_os_str() == "chats");
        let is_session_json = extension == "json" && stem.starts_with("session-");
        if extension == "jsonl" {
            let mut current_model: Option<String> = None;
            let Ok(text) = crate::scan::read_to_string(&file) else {
                continue;
            };
            for (line_index, line) in text.lines().enumerate() {
                if !crate::scan::checkpoint() {
                    break;
                }
                let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                if row.get("type").and_then(serde_json::Value::as_str) == Some("init") {
                    if let Some(model) = json_text(&row, &["model"]).map(str::to_string) {
                        current_model = Some(model);
                    }
                    continue;
                }
                if row.get("tokens").is_none() {
                    match (
                        row.get("stats"),
                        row.get("result").and_then(|result| result.get("stats")),
                    ) {
                        (Some(_stats), _) | (_, Some(_stats)) => {
                            // A headless stream may close with aggregate stats. They
                            // are line-owned records and follow upstream's same
                            // identity rules.
                            let stats = row.get("stats").or_else(|| {
                                row.get("result").and_then(|result| result.get("stats"))
                            });
                            let Some(stats) = stats else {
                                continue;
                            };
                            let at = json_timestamp(
                                stats.get("timestamp").or_else(|| row.get("timestamp")),
                                false,
                            );
                            let line_time = json_timestamp(
                                row.get("timestamp").or_else(|| row.get("created_at")),
                                false,
                            );
                            if let Some(models) =
                                stats.get("models").and_then(serde_json::Value::as_object)
                            {
                                for (model, counts) in models {
                                    let Some((tally, _)) = gemini_usage(counts, false) else {
                                        continue;
                                    };
                                    if tally.total() == 0 {
                                        continue;
                                    }
                                    let Some(at) = json_timestamp(counts.get("timestamp"), false)
                                        .or(at)
                                        .or(line_time)
                                    else {
                                        continue;
                                    };
                                    let identity = json_text(&row, &["id"])
                                        .map(|id| format!("gemini:line:{id}:{model}"))
                                        .unwrap_or_else(|| {
                                            format!("gemini:headless:{stem}:{line_index}:{model}")
                                        });
                                    if seen.insert(identity) {
                                        *buckets
                                            .entry(slot_key_from_ms(at))
                                            .or_default()
                                            .entry(model.to_string())
                                            .or_default() += tally;
                                    }
                                }
                            } else {
                                let Some((tally, _)) = gemini_usage(stats, false) else {
                                    continue;
                                };
                                if tally.total() == 0 {
                                    continue;
                                }
                                let Some(model) = json_text(stats, &["model"])
                                    .map(str::to_string)
                                    .or_else(|| current_model.clone())
                                else {
                                    continue;
                                };
                                let Some(at) = at.or(line_time) else {
                                    continue;
                                };
                                let identity = json_text(&row, &["id"])
                                    .map(|id| format!("gemini:line:{id}"))
                                    .unwrap_or_else(|| {
                                        format!("gemini:headless:{stem}:{line_index}")
                                    });
                                if seen.insert(identity) {
                                    *buckets
                                        .entry(slot_key_from_ms(at))
                                        .or_default()
                                        .entry(model)
                                        .or_default() += tally;
                                }
                                continue;
                            }
                        }
                        _ => continue,
                    }
                }
                let tokens = row.get("tokens").unwrap();
                let Some((tally, _)) = gemini_usage(tokens, true) else {
                    continue;
                };
                if tally.total() == 0 {
                    continue;
                }
                let Some(model) = json_text(&row, &["model"])
                    .map(str::to_string)
                    .or_else(|| current_model.clone())
                else {
                    continue;
                };
                let Some(at) = json_timestamp(row.get("timestamp"), false) else {
                    continue;
                };
                let identity = json_text(&row, &["id"])
                    .map(|id| format!("gemini:line:{id}"))
                    .unwrap_or_else(|| format!("gemini:headless:{stem}:{line_index}"));
                if seen.insert(identity) {
                    *buckets
                        .entry(slot_key_from_ms(at))
                        .or_default()
                        .entry(model)
                        .or_default() += tally;
                }
            }
        } else if is_chat_json || is_session_json {
            let Ok(bytes) = crate::scan::read(&file) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let Some(messages) = value.get("messages").and_then(serde_json::Value::as_array) else {
                continue;
            };
            let session = json_text(&value, &["sessionId", "session_id"])
                .map(str::to_string)
                .unwrap_or_else(|| stem.clone());
            for message in messages {
                if !crate::scan::checkpoint() {
                    break;
                }
                if message.get("type").and_then(serde_json::Value::as_str) != Some("gemini") {
                    continue;
                }
                let Some(tokens) = message.get("tokens") else {
                    continue;
                };
                let Some((tally, _)) = gemini_usage(tokens, false) else {
                    continue;
                };
                if tally.total() == 0 {
                    continue;
                }
                let Some(model) = json_text(message, &["model"]).map(str::to_string) else {
                    continue;
                };
                let Some(at) = json_timestamp(
                    message
                        .get("timestamp")
                        .or_else(|| message.get("created_at")),
                    false,
                ) else {
                    continue;
                };
                let identity = json_text(message, &["id"])
                    .map(|id| format!("gemini:session:{session}:{id}"))
                    .unwrap_or_else(|| format!("gemini:session:{session}:{at}"));
                if seen.insert(identity) {
                    *buckets
                        .entry(slot_key_from_ms(at))
                        .or_default()
                        .entry(model)
                        .or_default() += tally;
                }
            }
        }
    }

    ledger_from_slot_buckets(&buckets, KEY)
}

fn gemini_usage(object: &serde_json::Value, headless: bool) -> Option<(TokenTally, i64)> {
    let object = object.as_object()?;
    let count = |names: &[&str]| -> Option<(i64, bool)> {
        names
            .iter()
            .find_map(|name| json_count(object.get(*name)).map(|value| (value, *name != "input")))
    };
    let (input, prompt_style) = count(&[
        "input",
        "prompt",
        "input_tokens",
        "prompt_tokens",
        "promptTokenCount",
    ])
    .unwrap_or((0, false));
    let output = count(&[
        "output",
        "candidates",
        "output_tokens",
        "completion_tokens",
        "candidatesTokenCount",
    ])
    .map(|(value, _)| value)
    .unwrap_or(0);
    let reasoning = count(&["thoughts", "reasoning", "thoughts_tokens"])
        .map(|(value, _)| value)
        .unwrap_or(0);
    let cached = count(&["cached", "cached_tokens", "cachedContentTokenCount"])
        .map(|(value, _)| value)
        .unwrap_or(0);
    let any_kind = [
        "input",
        "prompt",
        "input_tokens",
        "prompt_tokens",
        "promptTokenCount",
    ]
    .iter()
    .any(|name| json_count(object.get(*name)).is_some())
        || [
            "output",
            "candidates",
            "output_tokens",
            "completion_tokens",
            "candidatesTokenCount",
        ]
        .iter()
        .any(|name| json_count(object.get(*name)).is_some())
        || ["cached", "cached_tokens", "cachedContentTokenCount"]
            .iter()
            .any(|name| json_count(object.get(*name)).is_some())
        || ["thoughts", "reasoning", "thoughts_tokens"]
            .iter()
            .any(|name| json_count(object.get(*name)).is_some());
    if !any_kind {
        return None;
    }
    if input < 0 || output < 0 || reasoning < 0 || cached < 0 {
        return None;
    }
    let cache_inclusive = if headless { prompt_style } else { false };
    Some((
        TokenTally {
            input: if cache_inclusive {
                input.saturating_sub(cached)
            } else {
                input
            },
            cache_write: 0,
            cache_read: cached,
            output: output.saturating_add(reasoning),
        },
        0,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericAgent {
    Amp,
    Droid,
}

impl GenericAgent {
    pub fn id(self) -> &'static str {
        match self {
            Self::Amp => "amp",
            Self::Droid => "droid",
        }
    }

    pub fn root(self) -> PathBuf {
        crate::model::home_path(match self {
            Self::Amp => ".local/share/amp/threads",
            Self::Droid => ".factory/sessions",
        })
    }
}

/// Reads Amp's thread JSON and Droid's session settings. Both emit their
/// own store-native increments; Amp reconciles message usage against its
/// ledger, while Droid keeps one aggregate session total.
pub fn generic_agent_ledger(agent: GenericAgent) -> UsageLedger {
    let key = agent.id();
    {
        let guard = memory();
        if let Some((at, map)) = guard.as_ref() {
            if at.elapsed() < LIFETIME {
                if let Some(known) = map.get(key) {
                    return known.clone();
                }
            }
        }
    }

    let root = agent.root();
    let mut files = Vec::new();
    collect_json(&root, &mut files);
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        if !crate::scan::checkpoint() {
            break;
        }
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok(bytes) = crate::scan::read(&file) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        match agent {
            GenericAgent::Amp => {
                if !name.starts_with("T-") || !name.ends_with(".json") {
                    continue;
                }
                let thread = json_text(&value, &["id"])
                    .map(str::to_string)
                    .unwrap_or(stem);
                let created = json_timestamp(value.get("created"), true);
                let mut message_calls: Vec<(String, TokenTally, Option<i64>)> = Vec::new();
                let mut events: Vec<(i64, String, TokenTally, Option<i64>, Option<i64>)> =
                    Vec::new();
                if let Some(messages) = value.get("messages").and_then(serde_json::Value::as_array)
                {
                    for message in messages {
                        if !crate::scan::checkpoint() {
                            break;
                        }
                        if message.get("role").and_then(serde_json::Value::as_str)
                            != Some("assistant")
                        {
                            continue;
                        }
                        let Some(usage) = message.get("usage") else {
                            continue;
                        };
                        let Some(tally) = anthropic_style_usage(
                            usage,
                            &[
                                "inputTokens",
                                "cacheCreationInputTokens",
                                "cacheReadInputTokens",
                                "outputTokens",
                            ],
                        ) else {
                            continue;
                        };
                        let Some(model) = json_text(usage, &["model"]).map(str::to_string) else {
                            continue;
                        };
                        message_calls.push((model, tally, json_count(message.get("messageId"))));
                    }
                }
                if let Some(events_value) = value
                    .get("usageLedger")
                    .and_then(|ledger| ledger.get("events"))
                    .and_then(serde_json::Value::as_array)
                {
                    for (index, row) in events_value.iter().enumerate() {
                        if !crate::scan::checkpoint() {
                            break;
                        }
                        let Some(tokens) = row.get("tokens") else {
                            continue;
                        };
                        let Some(tally) = anthropic_style_usage(
                            tokens,
                            &[
                                "input",
                                "cacheCreationInputTokens",
                                "cacheReadInputTokens",
                                "output",
                            ],
                        ) else {
                            continue;
                        };
                        let Some(model) = json_text(row, &["model"]).map(str::to_string) else {
                            continue;
                        };
                        events.push((
                            index as i64,
                            model,
                            tally,
                            json_count(row.get("toMessageId")),
                            json_count(row.get("fromMessageId")),
                        ));
                    }
                }
                let mut consumed = vec![false; events.len()];
                let mut unmatched = Vec::new();
                for (model, tally, message_id) in message_calls {
                    if !crate::scan::checkpoint() {
                        break;
                    }
                    let match_index = message_id.and_then(|id| {
                        events
                            .iter()
                            .position(|event| !consumed[event.0 as usize] && event.3 == Some(id))
                    });
                    let match_index = match_index.or_else(|| {
                        events.iter().position(|event| {
                            !consumed[event.0 as usize] && event.1 == model && event.2 == tally
                        })
                    });
                    if let Some(index) = match_index {
                        consumed[events[index].0 as usize] = true;
                    } else {
                        unmatched.push((model, tally, message_id));
                    }
                }
                if let Some(rows) = value
                    .get("usageLedger")
                    .and_then(|ledger| ledger.get("events"))
                    .and_then(serde_json::Value::as_array)
                {
                    for (index, model, tally, to_id, from_id) in events {
                        if !crate::scan::checkpoint() {
                            break;
                        }
                        let row = &rows[index as usize];
                        let Some(at) = json_timestamp(row.get("timestamp"), false).or(created)
                        else {
                            continue;
                        };
                        let suffix = to_id
                            .map(|id| id.to_string())
                            .or_else(|| from_id.map(|id| id.to_string()))
                            .unwrap_or_else(|| index.to_string());
                        let identity = format!("amp:{thread}:event:{suffix}");
                        if seen.insert(identity) {
                            *buckets
                                .entry(slot_key_from_ms(at))
                                .or_default()
                                .entry(model)
                                .or_default() += tally;
                        }
                    }
                }
                for (model, tally, message_id) in unmatched {
                    if !crate::scan::checkpoint() {
                        break;
                    }
                    let Some(at) = created else {
                        continue;
                    };
                    let suffix = message_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "0".into());
                    let identity = format!("amp:{thread}:message:{suffix}");
                    if seen.insert(identity) {
                        *buckets
                            .entry(slot_key_from_ms(at))
                            .or_default()
                            .entry(model)
                            .or_default() += tally;
                    }
                }
            }
            GenericAgent::Droid => {
                if !name.ends_with(".settings.json") {
                    continue;
                }
                let Some(usage) = value.get("tokenUsage") else {
                    continue;
                };
                let Some((tally, _partial)) = droid_usage(usage) else {
                    continue;
                };
                if tally.total() == 0 {
                    continue;
                }
                let Some(at) = json_timestamp(value.get("providerLockTimestamp"), false) else {
                    continue;
                };
                let Some(model) = json_text(&value, &["model"])
                    .map(str::to_string)
                    .or_else(|| droid_provider_model(json_text(&value, &["providerLock"])))
                else {
                    continue;
                };
                let session = name.strip_suffix(".settings.json").unwrap_or(&stem);
                let identity = format!("droid:{session}");
                if seen.insert(identity) {
                    *buckets
                        .entry(slot_key_from_ms(at))
                        .or_default()
                        .entry(model)
                        .or_default() += tally;
                }
            }
        }
    }

    ledger_from_slot_buckets(&buckets, key)
}

fn anthropic_style_usage(object: &serde_json::Value, names: &[&str; 4]) -> Option<TokenTally> {
    let object = object.as_object()?;
    let count = |name: &str| json_count(object.get(name));
    let tally = TokenTally {
        input: count(names[0])?,
        cache_write: count(names[1]).unwrap_or(0),
        cache_read: count(names[2]).unwrap_or(0),
        output: count(names[3]).unwrap_or(0),
    };
    (tally.total() > 0
        && [
            tally.input,
            tally.cache_write,
            tally.cache_read,
            tally.output,
        ]
        .iter()
        .copied()
        .all(|value| value >= 0))
    .then_some(tally)
}

fn droid_usage(object: &serde_json::Value) -> Option<(TokenTally, bool)> {
    let object = object.as_object()?;
    let count = |name: &str| json_count(object.get(name));
    let input = count("inputTokens");
    let output = count("outputTokens");
    let thinking = count("thinkingTokens");
    let cache_write = count("cacheCreationTokens");
    let cache_read = count("cacheReadTokens");
    let total = count("totalTokens")
        .or_else(|| count("total"))
        .or_else(|| count("total_tokens"));
    if input.is_none()
        && output.is_none()
        && thinking.is_none()
        && cache_write.is_none()
        && cache_read.is_none()
        && total.is_none()
    {
        return None;
    }
    let input = input.unwrap_or(0);
    let output = output.unwrap_or(0);
    let thinking = thinking.unwrap_or(0);
    let cache_write = cache_write.unwrap_or(0);
    let cache_read = cache_read.unwrap_or(0);
    if [input, output, thinking, cache_write, cache_read]
        .iter()
        .copied()
        .any(|value| value < 0)
    {
        return None;
    }
    if let Some(total) = total {
        if total < 0 {
            return None;
        }
        let variants = [
            (
                input,
                cache_write,
                cache_read,
                output.checked_add(thinking)?,
            ),
            (
                input.saturating_sub(cache_read).saturating_sub(cache_write),
                cache_write,
                cache_read,
                output.checked_add(thinking)?,
            ),
            (input, cache_write, cache_read, output),
            (
                input.saturating_sub(cache_read).saturating_sub(cache_write),
                cache_write,
                cache_read,
                output,
            ),
        ];
        for variant in variants {
            let sum = TokenTally {
                input: variant.0,
                cache_write: variant.1,
                cache_read: variant.2,
                output: variant.3,
            };
            if sum.total() == total {
                return Some((sum, false));
            }
        }
        return Some((TokenTally::default(), false));
    }
    if cache_read == 0 && cache_write == 0 {
        return Some((
            TokenTally {
                input,
                cache_write: 0,
                cache_read: 0,
                output,
            },
            thinking > 0,
        ));
    }
    Some((
        TokenTally {
            input: 0,
            cache_write: 0,
            cache_read: 0,
            output,
        },
        true,
    ))
}

fn droid_provider_model(provider: Option<&str>) -> Option<String> {
    let provider = provider?.trim().to_ascii_lowercase();
    if provider.is_empty() {
        return None;
    }
    if provider.contains("anthropic") || provider.contains("claude") {
        return Some("claude-unknown".into());
    }
    if provider.contains("openai") || provider.contains("gpt") {
        return Some("gpt-unknown".into());
    }
    if provider.contains("google") || provider.contains("gemini") {
        return Some("gemini-unknown".into());
    }
    if provider.contains("xai") || provider.contains("grok") {
        return Some("grok-unknown".into());
    }
    Some(format!("{provider}-unknown"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_prices::ModelPrice;

    struct TranscriptFixture(PathBuf);
    impl TranscriptFixture {
        fn new(text: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "quotascope-stream-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::create_dir(&root).unwrap();
            let path = root.join("session.jsonl");
            std::fs::write(&path, text).unwrap();
            Self(path)
        }
        fn append(&self, text: &str) {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .append(true)
                .open(&self.0)
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        fn read(&self, known: Option<CachedEntry>) -> CachedEntry {
            read_entry(&self.0, Provider::Codex, Stamp::of(&self.0).unwrap(), known).unwrap()
        }
    }
    impl Drop for TranscriptFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_dir(self.0.parent().unwrap());
        }
    }
    fn codex_header(model: &str) -> String {
        format!("{{\"payload\":{{\"model\":\"{model}\",\"cwd\":\"C:/fixture\"}}}}\n{{\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":\"Fixture session\"}}}}\n")
    }
    fn stream_codex_count(n: i64) -> String {
        serde_json::json!({
            "timestamp": "2026-10-03T01:00:00Z",
            "payload": {"type":"token_count", "info":{"total_token_usage":{
                "input_tokens":n*100, "cached_input_tokens":n*20,
                "cache_write_input_tokens":n*5, "output_tokens":n*50
            }}}
        })
        .to_string()
            + "\n"
    }
    fn entry_total(entry: &CachedEntry) -> i64 {
        entry
            .days
            .values()
            .flat_map(|models| models.values())
            .map(TokenTally::total)
            .sum()
    }

    #[test]
    fn nanosecond_stamps_survive_json_and_legacy_float_stamps_require_refresh() {
        let stamp = Stamp {
            size: 50,
            modified_ns: Some(1_791_051_801_000_002_100),
        };
        let restored: Stamp = serde_json::from_slice(&serde_json::to_vec(&stamp).unwrap()).unwrap();
        assert_eq!(restored, stamp);
        let legacy: Stamp =
            serde_json::from_str(r#"{"size":50,"modified":1791051801.0000021}"#).unwrap();
        assert_ne!(legacy, stamp);
        assert_eq!(legacy.size, stamp.size);
        assert_eq!(legacy.modified_ns, None);
    }

    #[test]
    fn unchanged_and_repriced_transcripts_do_not_request_a_cache_write() {
        let file = TranscriptFixture::new(&(codex_header("alpha") + &stream_codex_count(1)));
        let root = file.0.parent().unwrap();
        let first = scan_cached(Provider::Codex, root, FileCache::default());
        assert!(first.changed);
        let old_bytes = serde_json::to_vec(&first.cache).unwrap();
        let cache = serde_json::from_slice(&old_bytes).unwrap();
        let second = scan_cached(Provider::Codex, root, cache);
        assert!(!second.changed);
        assert_eq!(serde_json::to_vec(&second.cache).unwrap(), old_bytes);
        assert_eq!(second.buckets, first.buckets);

        let old_prices = BTreeMap::from([("alpha".into(), price(1.0, 2.0, None))]);
        let new_prices = BTreeMap::from([("alpha".into(), price(3.0, 4.0, None))]);
        let before = priced(&second.buckets, &old_prices, None);
        let after = priced(&second.buckets, &new_prices, None);
        assert_eq!(before.days[0].tokens, after.days[0].tokens);
        assert_ne!(before.days[0].cost, after.days[0].cost);
        assert_eq!(serde_json::to_vec(&second.cache).unwrap(), old_bytes);
    }

    #[test]
    fn transcript_edits_and_deletions_request_persistence_after_cache_reload() {
        let file = TranscriptFixture::new(&(codex_header("alpha") + &stream_codex_count(1)));
        let root = file.0.parent().unwrap();
        let first = scan_cached(Provider::Codex, root, FileCache::default());
        let old = serde_json::from_slice(&serde_json::to_vec(&first.cache).unwrap()).unwrap();
        file.append(&stream_codex_count(2));
        let appended = scan_cached(Provider::Codex, root, old);
        assert!(appended.changed);
        assert_eq!(
            entry_total(appended.cache.files.values().next().unwrap()),
            310
        );
        std::fs::write(&file.0, codex_header("beta") + &stream_codex_count(1)).unwrap();
        let rewritten = scan_cached(Provider::Codex, root, appended.cache);
        assert!(rewritten.changed);
        assert_eq!(
            entry_total(rewritten.cache.files.values().next().unwrap()),
            155
        );
        assert!(rewritten
            .buckets
            .values()
            .all(|models| !models.contains_key("alpha")));
        std::fs::remove_file(&file.0).unwrap();
        let removed = scan_cached(Provider::Codex, root, rewritten.cache);
        assert!(removed.changed);
        assert!(removed.cache.files.is_empty() && removed.buckets.is_empty());
        assert!(!scan_cached(Provider::Codex, root, removed.cache).changed);
    }

    #[test]
    fn codex_appends_resume_after_serialization_without_recounting_or_losing_the_model() {
        let file = TranscriptFixture::new(
            &(codex_header("alpha") + &stream_codex_count(1) + &stream_codex_count(2)),
        );
        let old = file.read(None);
        assert_eq!(entry_total(&old), 310);
        let old = serde_json::from_slice(&serde_json::to_vec(&old).unwrap()).unwrap();
        file.append(
            &(stream_codex_count(2)
                + "{\"payload\":{\"model\":\"beta\"}}\n"
                + &stream_codex_count(3)),
        );
        let new = file.read(Some(old));
        assert_eq!(entry_total(&new), 465);
        let alpha: i64 = new
            .days
            .values()
            .filter_map(|m| m.get("alpha"))
            .map(TokenTally::total)
            .sum();
        let beta: i64 = new
            .days
            .values()
            .filter_map(|m| m.get("beta"))
            .map(TokenTally::total)
            .sum();
        assert_eq!((alpha, beta), (310, 155));
        assert_eq!(new.title.as_deref(), Some("Fixture session"));
        assert_eq!(new.cwd.as_deref(), Some("C:/fixture"));
        assert!(new.codex.is_some());
    }

    #[test]
    fn changed_prefix_and_truncation_rebuild_instead_of_reusing_old_totals() {
        let file = TranscriptFixture::new(&(codex_header("alpha") + &stream_codex_count(1)));
        let old = file.read(None);
        std::fs::write(
            &file.0,
            codex_header("beta") + &stream_codex_count(3) + &stream_codex_count(4),
        )
        .unwrap();
        let changed = file.read(Some(old));
        assert_eq!(entry_total(&changed), 620);
        assert!(changed.days.values().all(|m| !m.contains_key("alpha")));
        std::fs::write(&file.0, codex_header("gamma") + &stream_codex_count(1)).unwrap();
        assert_eq!(entry_total(&file.read(Some(changed))), 155);
    }

    #[test]
    fn an_unfinished_last_line_is_rebuilt_when_the_next_append_completes_it() {
        let second = stream_codex_count(2);
        let split = second.len() / 2;
        let file = TranscriptFixture::new(
            &(codex_header("alpha") + &stream_codex_count(1) + &second[..split]),
        );
        let old = file.read(None);
        assert_eq!(entry_total(&old), 155);
        assert!(old.codex.is_none());
        file.append(&second[split..]);
        let complete = file.read(Some(old));
        assert_eq!(entry_total(&complete), 310);
        assert!(complete.codex.is_some());
    }

    #[test]
    fn a_valid_last_line_without_newline_is_counted_once_after_it_is_terminated() {
        let file =
            TranscriptFixture::new(&(codex_header("alpha") + stream_codex_count(1).trim_end()));
        let old = file.read(None);
        assert_eq!(entry_total(&old), 155);
        assert!(old.codex.is_none());
        file.append(&("\n".to_owned() + &stream_codex_count(2)));
        assert_eq!(entry_total(&file.read(Some(old))), 310);
    }

    #[test]
    fn caches_without_a_resume_checkpoint_remain_compatible() {
        let file = TranscriptFixture::new(&(codex_header("alpha") + &stream_codex_count(1)));
        let old = file.read(None);
        let mut value = serde_json::to_value(old).unwrap();
        value.as_object_mut().unwrap().remove("codex");
        let old = serde_json::from_value(value).unwrap();
        file.append(&stream_codex_count(2));
        assert_eq!(entry_total(&file.read(Some(old))), 310);
    }

    #[test]
    fn a_live_append_keeps_the_bounded_old_snapshot_and_is_counted_on_the_next_scan() {
        let file = TranscriptFixture::new(&(codex_header("alpha") + &stream_codex_count(1)));
        let stamp = Stamp::of(&file.0).unwrap();
        file.append(&stream_codex_count(2));
        let old = read_entry(&file.0, Provider::Codex, stamp, None).unwrap();
        assert_eq!(entry_total(&old), 155);
        assert_eq!(entry_total(&file.read(Some(old))), 310);
    }

    #[test]
    fn streaming_claude_preserves_duplicates_titles_and_rejects_partial_read_errors() {
        let text = concat!(
            "{\"type\":\"assistant\",\"timestamp\":\"2026-10-03T01:00:00Z\",\"message\":{\"id\":\"one\",\"model\":\"alpha\",\"usage\":{\"input_tokens\":100,\"output_tokens\":50}}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-10-03T01:00:00Z\",\"message\":{\"id\":\"one\",\"model\":\"alpha\",\"usage\":{\"input_tokens\":100,\"output_tokens\":50}}}\n",
            "{\"customTitle\":\"Renamed session\"}\n"
        );
        let file = TranscriptFixture::new(text);
        let entry = read_entry(
            &file.0,
            Provider::ClaudeCode,
            Stamp::of(&file.0).unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(entry_total(&entry), 150);
        assert_eq!(entry.title.as_deref(), Some("Renamed session"));
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&file.0)
            .unwrap()
            .write_all(b"\xff\n")
            .unwrap();
        assert!(read_entry(
            &file.0,
            Provider::ClaudeCode,
            Stamp::of(&file.0).unwrap(),
            None
        )
        .is_err());
    }

    #[test]
    fn codex_parser_stops_between_records_when_cancelled() {
        let control = std::sync::Arc::new(crate::scan::Control::default());
        let stop = control.clone();
        let read = std::cell::Cell::new(0);
        let result = crate::scan::run(
            control,
            1,
            |_| {},
            || {
                parse_codex(std::iter::from_fn(|| {
                    let count = read.get() + 1;
                    read.set(count);
                    if count == 2 {
                        stop.cancel();
                    }
                    assert!(count <= 2, "continued pulling records after cancellation");
                    Some("{}".to_owned())
                }))
            },
        );
        assert!(matches!(result, Err(crate::scan::Cancelled)));
        assert_eq!(read.get(), 2);
    }

    #[test]
    fn expired_ledgers_are_evicted_so_new_scans_get_a_fresh_lifetime() {
        let now = Instant::now();
        let mut cache = Some((
            now - LIFETIME,
            HashMap::from([("codex".into(), UsageLedger::empty())]),
        ));
        expire_memory(&mut cache, now, true);
        assert!(cache.is_none());
        let entry = cache.get_or_insert_with(|| (now, HashMap::new()));
        entry.1.insert("codex".into(), UsageLedger::empty());
        expire_memory(&mut cache, now + Duration::from_secs(1), true);
        assert!(cache.as_ref().unwrap().1.contains_key("codex"));
        expire_memory(&mut cache, now + Duration::from_secs(2), false);
        assert!(cache.is_none());
    }

    fn price(input: f64, output: f64, cache_read: Option<f64>) -> ModelPrice {
        ModelPrice {
            input,
            output,
            cache_read,
            cache_write: None,
            name: None,
        }
    }

    #[test]
    fn antigravity_keeps_the_name_it_records_and_says_unknown_otherwise() {
        let generation = |model: Option<&str>, label: Option<&str>| AntigravityGeneration {
            index: 0,
            model: model.map(str::to_string),
            label: label.map(str::to_string),
            tally: None,
            response_id: None,
            timestamp_ms: None,
        };
        let labels = HashMap::new();
        // The label is the provider's own display name, which the price table
        // reaches; folding it into "unknown" would drop a priced model.
        assert_eq!(
            antigravity_model(
                &generation(Some("gemini-default"), Some("Gemini 3.7 Flash (High)")),
                &labels,
                None
            ),
            "Gemini 3.7 Flash (High)"
        );
        // A routing placeholder answers with the conversation's one model.
        assert_eq!(
            antigravity_model(
                &generation(Some("gemini-default"), None),
                &labels,
                Some("gemini-3.8-flash")
            ),
            "gemini-3.8-flash"
        );
        // Nothing named at all stays the honest bucket.
        assert_eq!(
            antigravity_model(&generation(None, None), &labels, None),
            "unknown"
        );
    }

    fn ag_varint(mut value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            out.push(if value == 0 { byte } else { byte | 0x80 });
            if value == 0 {
                return out;
            }
        }
    }

    fn ag_field(field: u32, wire: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = ag_varint(u64::from(field << 3) | u64::from(wire));
        out.extend_from_slice(payload);
        out
    }

    fn ag_count(field: u32, value: u64) -> Vec<u8> {
        ag_field(field, 0, &ag_varint(value))
    }

    fn ag_text(field: u32, text: &str) -> Vec<u8> {
        let bytes = text.as_bytes();
        let mut out = ag_varint(bytes.len() as u64);
        out.extend_from_slice(bytes);
        ag_field(field, 2, &out)
    }

    fn ag_nested(field: u32, inner: &[u8]) -> Vec<u8> {
        let mut out = ag_varint(inner.len() as u64);
        out.extend_from_slice(inner);
        ag_field(field, 2, &out)
    }

    /// The shape read from this machine's own Antigravity databases on
    /// 2026-10-01: usage counts, the model and label strings, and the response
    /// timestamp. Field 1 of the usage message is a count whose meaning is not
    /// established, so it must not reach the tally.
    #[test]
    fn antigravity_usage_counts_come_from_the_fields_observed_on_this_machine() {
        let mut usage = ag_count(1, 9_999);
        usage.extend(ag_count(2, 1_000));
        usage.extend(ag_count(5, 300));
        usage.extend(ag_count(9, 200));
        usage.extend(ag_count(10, 50));
        usage.extend(ag_text(11, "response-7"));
        let mut chat = ag_nested(4, &usage);
        chat.extend(ag_text(19, "gemini-3.8-flash-control"));
        chat.extend(ag_text(21, "Gemini 3.8 Flash"));
        chat.extend(ag_nested(
            9,
            &ag_nested(
                4,
                &[
                    ag_count(1, 1_777_000_000).as_slice(),
                    &ag_count(2, 500_000_000),
                ]
                .concat(),
            ),
        ));
        let root = ag_nested(1, &chat);

        let found = antigravity_generation(3, &root).expect("generation");
        assert_eq!(found.index, 3);
        assert_eq!(found.model.as_deref(), Some("gemini-3.8-flash-control"));
        assert_eq!(found.label.as_deref(), Some("Gemini 3.8 Flash"));
        assert_eq!(found.response_id.as_deref(), Some("response-7"));
        assert_eq!(found.timestamp_ms, Some(1_777_000_000_500));
        assert_eq!(
            found.tally.expect("tally"),
            TokenTally {
                input: 1_000,
                cache_write: 0,
                cache_read: 300,
                output: 250,
            }
        );
    }

    #[test]
    fn antigravity_rejects_a_message_it_cannot_read() {
        // Wire type 3 (start group) is not a shape this decodes.
        let chat = ag_nested(1, &ag_field(4, 3, &[]));
        assert!(antigravity_generation(0, &chat).is_none());
        // A zero field number, and a varint that runs off the end of the
        // buffer, are refused rather than read as something plausible.
        assert!(decode_antigravity_message(&[0x00]).is_none());
        assert!(decode_antigravity_message(&[0x08, 0x80]).is_none());
    }

    fn claude_line(
        model: &str,
        input: i64,
        cache_write: i64,
        cache_read: i64,
        output: i64,
        at: &str,
        id: &str,
    ) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{at}","message":{{"id":"{id}","model":"{model}","usage":{{"input_tokens":{input},"cache_creation_input_tokens":{cache_write},"cache_read_input_tokens":{cache_read},"output_tokens":{output}}}}}}}"#
        )
    }

    #[test]
    fn claude_parser_counts_each_reply_once_into_its_quarter_hour() {
        let at = |iso: &str| slot_key_from_ms(parse_iso8601(iso).expect("parses"));
        let key1 = at("2026-10-01T10:02:11.000Z");
        let key2 = at("2026-10-01T10:19:59.000Z");
        assert_ne!(key1, key2, "the second reply lands in the next quarter");
        let transcript = format!(
            "{}\n{}\n{}\n",
            claude_line(
                "claude-opus-4.6",
                100,
                200,
                300,
                10,
                "2026-10-01T10:02:11.000Z",
                "m1"
            ),
            // A retry of the same reply: same message id, counted once.
            claude_line(
                "claude-opus-4.6",
                100,
                200,
                300,
                10,
                "2026-10-01T10:03:00.000Z",
                "m1"
            ),
            claude_line(
                "claude-opus-4.6",
                5,
                0,
                0,
                1,
                "2026-10-01T10:19:59.000Z",
                "m2"
            ),
        );
        let scanned = parse_claude_code(transcript.lines().map(str::to_string));
        assert_eq!(scanned.days.len(), 2, "two quarter-hours");
        let first = &scanned.days[&key1]["claude-opus-4.6"];
        assert_eq!(
            (first.input, first.output),
            (100, 10),
            "the retry is folded away"
        );
        let second = &scanned.days[&key2]["claude-opus-4.6"];
        assert_eq!(second.input, 5);
    }

    #[test]
    fn streamed_reply_grows_without_adding_input_or_moving_its_bucket() {
        let lines = [
            claude_line("alpha", 100, 20, 30, 1, "2026-10-01T10:14:59Z", "one"),
            claude_line("alpha", 90, 20, 30, 40, "2026-10-01T10:15:01Z", "one"),
            claude_line("alpha", 100, 20, 30, 10, "2026-10-01T10:15:02Z", "one"),
        ];
        let scanned = parse_claude_code(lines.into_iter());
        assert_eq!(scanned.days.len(), 1);
        let tally = scanned.days.values().next().unwrap()["alpha"];
        assert_eq!(
            tally,
            TokenTally {
                input: 100,
                cache_write: 20,
                cache_read: 30,
                output: 40
            }
        );
    }

    #[test]
    fn separate_files_keep_independent_reply_identity() {
        let line = claude_line("alpha", 100, 0, 0, 40, "2026-10-01T10:14:59Z", "one");
        let total = |s: Scanned| {
            s.days
                .values()
                .flat_map(|m| m.values())
                .map(TokenTally::total)
                .sum::<i64>()
        };
        assert_eq!(
            total(parse_claude_code([line.clone()].into_iter()))
                + total(parse_claude_code([line].into_iter())),
            280
        );
    }

    #[test]
    fn claude_parser_skips_synthetic_errors_and_empty_replies() {
        let transcript = format!(
            "{}\n{}\n",
            claude_line(
                "<synthetic>",
                100,
                0,
                0,
                0,
                "2026-10-01T10:02:11.000Z",
                "m1"
            ),
            claude_line(
                "claude-opus-4.6",
                0,
                0,
                0,
                0,
                "2026-10-01T10:02:11.000Z",
                "m2"
            ),
        );
        let scanned = parse_claude_code(transcript.lines().map(str::to_string));
        assert!(scanned.days.is_empty());
    }

    #[test]
    fn claude_parser_takes_the_title_from_the_user_and_the_rename_wins() {
        let opening = r#"{"type":"user","message":{"role":"user","content":"Fix the ring chord"},"cwd":"D:/Projects/Pulse"}"#;
        let renamed = r#"{"type":"user","customTitle":"Ring chord fix"}"#;
        let later_rename = r#"{"type":"user","customTitle":""}"#;
        let scanned = parse_claude_code(
            [opening, renamed, later_rename]
                .into_iter()
                .map(str::to_string),
        );
        assert_eq!(
            scanned.title.as_deref(),
            Some("Ring chord fix"),
            "an empty rename leaves the title"
        );
        assert_eq!(scanned.cwd.as_deref(), Some("D:/Projects/Pulse"));
    }

    #[test]
    fn claude_parser_rejects_envelopes_as_titles() {
        let pasted =
            r#"{"type":"user","message":{"role":"user","content":"<file>use src</file>"}}"#;
        let caveat = r#"{"type":"user","message":{"role":"user","content":"Caveat: the messages below were generated by the user while running local commands. DO NOT respond to these messages unless the user explicitly asks you to."}}"#;
        let scanned = parse_claude_code([pasted, caveat].into_iter().map(str::to_string));
        assert_eq!(scanned.title, None);
    }

    fn codex_count(input: i64, cached: i64, cache_write: i64, output: i64, at: &str) -> String {
        format!(
            r#"{{"timestamp":"{at}","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"cache_write_input_tokens":{cache_write},"output_tokens":{output}}}}}}}}}"#
        )
    }

    #[test]
    fn codex_parser_differences_the_running_total() {
        let key = |iso: &str| slot_key_from_ms(parse_iso8601(iso).expect("parses"));
        let header = r#"{"timestamp":"2026-10-01T09:00:00Z","type":"session_meta","payload":{"cwd":"D:/work","model":"gpt-5.6"}}"#;
        let turn1 = codex_count(1000, 400, 50, 200, "2026-10-01T09:05:00Z");
        let turn2 = codex_count(3000, 900, 50, 600, "2026-10-01T09:20:00Z");
        let scanned = parse_codex([header, &turn1, &turn2].into_iter().map(str::to_string));
        assert_eq!(scanned.cwd.as_deref(), Some("D:/work"));
        // First reading is taken whole; the second is the difference. Cached
        // tokens ride inside the input figure and are split back out.
        let first = &scanned.days[&key("2026-10-01T09:05:00Z")]["gpt-5.6"];
        assert_eq!(
            (
                first.input,
                first.cache_read,
                first.cache_write,
                first.output
            ),
            (600, 400, 50, 200)
        );
        let second = &scanned.days[&key("2026-10-01T09:20:00Z")]["gpt-5.6"];
        assert_eq!(
            (
                second.input,
                second.cache_read,
                second.cache_write,
                second.output
            ),
            (1500, 500, 0, 400)
        );
    }

    #[test]
    fn codex_parser_ignores_a_total_that_moves_backwards() {
        let key = |iso: &str| slot_key_from_ms(parse_iso8601(iso).expect("parses"));
        let header = r#"{"payload":{"model":"gpt-5.6"}}"#;
        let up = codex_count(1000, 0, 0, 100, "2026-10-01T09:05:00Z");
        let down = codex_count(900, 0, 0, 50, "2026-10-01T09:06:00Z");
        let scanned = parse_codex([header, &up, &down].into_iter().map(str::to_string));
        let day = &scanned.days[&key("2026-10-01T09:05:00Z")]["gpt-5.6"];
        // A backwards total differences to zero everywhere and is dropped.
        assert_eq!(day.total(), 1100, "only the first reading counts");
    }

    #[test]
    fn codex_parser_tracks_a_mid_session_model_change() {
        let key = |iso: &str| slot_key_from_ms(parse_iso8601(iso).expect("parses"));
        let header = r#"{"payload":{"model":"gpt-5.6"}}"#;
        let turn = codex_count(100, 0, 0, 10, "2026-10-01T09:05:00Z");
        let switch = r#"{"timestamp":"2026-10-01T09:21:00Z","payload":{"type":"token_count","model":"gpt-5.7","info":{"total_token_usage":{"input_tokens":300,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":30}}}}"#;
        let scanned = parse_codex([header, &turn, &switch].into_iter().map(str::to_string));
        // Differenced against the previous reading, not the raw totals.
        assert_eq!(
            scanned.days[&key("2026-10-01T09:21:00Z")]["gpt-5.7"].total(),
            220
        );
    }

    #[test]
    fn titles_are_cut_and_envelopes_refused() {
        assert_eq!(
            title_from("Fix the ring chord"),
            Some("Fix the ring chord".to_string())
        );
        assert_eq!(title_from("<file>x</file>"), None);
        assert_eq!(title_from("Caveat: no"), None);
        let long = "w".repeat(80);
        let cut = title_from(&long).expect("cut");
        assert_eq!(cut.chars().count(), 70);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn slot_keys_floor_to_quarter_hours_in_local_time() {
        // 10:02:11 falls inside a quarter and is floored to its start; two
        // timestamps in the same quarter share a key.
        let at = parse_iso8601("2026-10-01T10:02:11.000Z").expect("parses");
        let key = slot_key_from_ms(at);
        let minute: i64 = key
            .split(' ')
            .next_back()
            .unwrap()
            .split(':')
            .nth(1)
            .unwrap()
            .parse()
            .expect("minute");
        assert_eq!(
            minute % 15,
            0,
            "the minute is a quarter boundary, got {key}"
        );
        assert_eq!(
            slot_key_from_ms(parse_iso8601("2026-10-01T10:03:49.000Z").expect("parses")),
            key
        );
        let round = slot_key_to_ms(&key).expect("round trips");
        assert_eq!(slot_key_from_ms(round), key);
    }

    #[test]
    fn pricing_splits_the_kinds_and_fills_the_quiet_days() {
        let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
        let today_key =
            slot_key_from_ms(parse_iso8601("2026-10-01T10:02:11.000Z").expect("parses"));
        buckets.entry(today_key.clone()).or_default().insert(
            "claude-opus-4.6".to_string(),
            TokenTally {
                input: 1_000_000,
                cache_write: 0,
                cache_read: 2_000_000,
                output: 1_000_000,
            },
        );
        // A model the table has no price for: counted, never priced.
        buckets.entry(today_key).or_default().insert(
            "mystery-model".to_string(),
            TokenTally {
                input: 500,
                cache_write: 0,
                cache_read: 0,
                output: 0,
            },
        );

        let mut prices: BTreeMap<String, ModelPrice> = BTreeMap::new();
        prices.insert("claude-opus-4.6".to_string(), price(3.0, 15.0, Some(0.3)));

        let ledger = priced(&buckets, &prices, None);
        assert_eq!(ledger.unpriced_models, vec!["mystery-model".to_string()]);
        // $3/M input + 2M × $0.3/M cache read + 1M × $15/M output = 18.60.
        assert!((ledger.slots[0].cost - 18.6).abs() < 1e-9);
        assert_eq!(ledger.slots[0].unpriced_tokens, 500);
        // The money split by model, so a window scoped to one model group can
        // be priced without the other groups' spending inside it. The unpriced
        // model is counted in `models` and absent from `costs`, which is why
        // the estimator places models by `models`, not by `costs`.
        assert_eq!(ledger.slots[0].models.len(), 2);
        assert!((ledger.slots[0].costs["claude-opus-4.6"] - 18.6).abs() < 1e-9);
        assert!(!ledger.slots[0].costs.contains_key("mystery-model"));
        // One calendar day: the day buckets fold to a single row.
        assert_eq!(ledger.days.len(), 1);
        assert_eq!(ledger.days[0].tokens, 4_000_500);
        let (tokens, cost) = ledger.all_time();
        assert_eq!(tokens, 4_000_500);
        assert!((cost - 18.6).abs() < 1e-9);
    }

    #[test]
    fn spend_since_takes_whole_slots_from_the_boundary() {
        let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
        for (hour, minute, tokens) in [("10", "00", 100i64), ("10", "15", 200), ("10", "30", 400)] {
            let key = format!("2026-10-01 {hour}:{minute}");
            buckets.entry(key).or_default().insert(
                "m".to_string(),
                TokenTally {
                    input: tokens,
                    cache_write: 0,
                    cache_read: 0,
                    output: 0,
                },
            );
        }
        let ledger = priced(&buckets, &BTreeMap::new(), None);
        let boundary = slot_key_to_ms("2026-10-01 10:15").expect("key");
        let (tokens, _) = ledger.spend_since(boundary);
        assert_eq!(tokens, 600, "the straddling slot counts in full");
    }

    #[test]
    fn top_model_and_cache_rate_answer_over_the_recent_span() {
        let mut ledger = UsageLedger::empty();
        let day = |input: i64, read: i64, model: &str, tokens: i64| LedgerDay {
            date: NaiveDate::from_ymd_opt(2026, 10, 1).expect("date"),
            tokens,
            cost: 0.0,
            unpriced_tokens: 0,
            models: [(model.to_string(), tokens)].into_iter().collect(),
            tally: TokenTally {
                input,
                cache_write: 0,
                cache_read: read,
                output: 0,
            },
            model_tallies: BTreeMap::new(),
            model_costs: BTreeMap::new(),
        };
        ledger.days = vec![
            day(1_000, 9_000, "claude-opus-4.6", 10_000),
            day(2_000, 3_000, "gpt-5.6", 5_000),
        ];
        ledger
            .model_names
            .insert("claude-opus-4.6".to_string(), "Claude Opus 4.6".to_string());

        let (top, share) = ledger.top_model(7).expect("a leader");
        assert_eq!(top, "Claude Opus 4.6");
        assert!((share - 10_000.0 / 15_000.0).abs() < 1e-9);
        // 12M cache reads over 15M input tokens.
        assert!((ledger.cache_hit_rate(7).expect("rate") - 0.8).abs() < 1e-9);
        assert!(
            (ledger
                .cache_hit_rate_calendar(30, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap())
                .unwrap()
                - 0.8)
                .abs()
                < 1e-9
        );
        assert_eq!(
            ledger.cache_hit_rate_calendar(30, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap()),
            None
        );
        ledger.days[0].tokens += 1;
        assert_eq!(
            ledger.cache_hit_rate_calendar(30, NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()),
            None
        );
        ledger.days[0].tokens -= 1;
    }

    #[test]
    fn empty_histories_answer_nothing() {
        let ledger = UsageLedger::empty();
        assert_eq!(ledger.top_model(7), None);
        assert_eq!(ledger.cache_hit_rate(7), None);
        assert_eq!(ledger.all_time(), (0, 0.0));
    }

    /// Reads this machine's own transcripts, if it has any. Ignored by
    /// default — a test may touch the real stores when the comment says so,
    /// and this is that comment.
    #[test]
    #[ignore = "reads this machine's own ~/.claude and ~/.codex"]
    fn live_ledger_smoke() {
        for provider in [Provider::ClaudeCode, Provider::Codex] {
            let built = build(provider);
            let (tokens, cost) = built.all_time();
            println!(
                "{}: days={} slots={} tokens={} cost=${:.2} unpriced={:?}",
                provider.raw(),
                built.days.len(),
                built.slots.len(),
                tokens,
                cost,
                built.unpriced_models
            );
            if let Some((model, share)) = built.top_model(31) {
                println!("  top model (31d): {model} at {:.0}%", share * 100.0);
            }
        }
    }

    #[test]
    fn qwen_cache_read_is_subtracted_from_prompt_when_included() {
        let usage = serde_json::json!({
            "promptTokenCount": 120,
            "candidatesTokenCount": 30,
            "thoughtsTokenCount": 5,
            "cachedContentTokenCount": 80,
            "totalTokenCount": 155
        });
        assert_eq!(
            qwen_usage(&usage),
            Some(TokenTally {
                input: 40,
                cache_write: 0,
                cache_read: 80,
                output: 35
            })
        );
    }

    #[test]
    fn qwen_unreconciled_total_is_not_assigned_to_a_kind() {
        let usage = serde_json::json!({
            "promptTokenCount": 100,
            "cachedContentTokenCount": 20,
            "outputTokens": 30,
            "totalTokenCount": 999
        });
        assert_eq!(qwen_usage(&usage), None);
    }

    #[test]
    fn pi_usage_keeps_named_buckets_and_refuses_a_disagreeing_total() {
        let usage = serde_json::json!({
            "input": 10,
            "output": 20,
            "cacheRead": 30,
            "cacheWrite": 5,
            "totalTokens": 65
        });
        assert_eq!(
            pi_usage(&usage),
            Some((
                TokenTally {
                    input: 10,
                    cache_write: 5,
                    cache_read: 30,
                    output: 20
                },
                0
            ))
        );
        let disagreeing = serde_json::json!({
            "input": 10,
            "output": 20,
            "totalTokens": 99
        });
        assert_eq!(pi_usage(&disagreeing), None);
    }

    #[test]
    fn pi_timestamps_accept_milliseconds_rfc3339_and_reject_booleans() {
        let expected = parse_iso8601("2026-10-01T10:02:03.000Z");
        assert_eq!(
            json_timestamp(Some(&serde_json::json!(expected.unwrap())), true),
            expected
        );
        assert_eq!(
            json_timestamp(Some(&serde_json::json!("2026-10-01T10:02:03.000Z")), false),
            expected
        );
        assert_eq!(json_timestamp(Some(&serde_json::json!(true)), true), None);
    }

    #[test]
    fn gemini_prompt_style_headless_input_subtracts_cached_content() {
        let usage = serde_json::json!({
            "prompt": 120,
            "candidates": 30,
            "thoughts": 5,
            "cached": 80
        });
        assert_eq!(
            gemini_usage(&usage, true),
            Some((
                TokenTally {
                    input: 40,
                    cache_write: 0,
                    cache_read: 80,
                    output: 35
                },
                0
            ))
        );
        let bare_input = serde_json::json!({"input": 20, "output": 5, "cached": 4});
        assert_eq!(
            gemini_usage(&bare_input, true),
            Some((
                TokenTally {
                    input: 20,
                    cache_write: 0,
                    cache_read: 4,
                    output: 5
                },
                0
            ))
        );
    }

    #[test]
    fn droid_total_selects_a_supported_split_and_bare_total_stays_counted() {
        let usage = serde_json::json!({
            "inputTokens": 100,
            "cacheReadTokens": 20,
            "cacheCreationTokens": 10,
            "outputTokens": 30,
            "thinkingTokens": 5,
            "totalTokens": 165
        });
        assert_eq!(
            droid_usage(&usage),
            Some((
                TokenTally {
                    input: 100,
                    cache_write: 10,
                    cache_read: 20,
                    output: 35
                },
                false
            ))
        );
        let bare = serde_json::json!({"totalTokens": 123});
        assert_eq!(droid_usage(&bare), Some((TokenTally::default(), false)));
    }

    #[test]
    fn anthropic_style_usage_requires_a_reported_input() {
        let names = [
            "inputTokens",
            "cacheCreationInputTokens",
            "cacheReadInputTokens",
            "outputTokens",
        ];
        let usage = serde_json::json!({
            "inputTokens": 10,
            "cacheCreationInputTokens": 2,
            "cacheReadInputTokens": 3,
            "outputTokens": 4
        });
        assert_eq!(
            anthropic_style_usage(&usage, &names),
            Some(TokenTally {
                input: 10,
                cache_write: 2,
                cache_read: 3,
                output: 4
            })
        );
        assert_eq!(
            anthropic_style_usage(&serde_json::json!({"outputTokens": 4}), &names),
            None
        );
    }

    #[test]
    fn prime_fork_copies_fold_and_parent_aggregates_are_reduced_by_children() {
        let root = std::env::temp_dir().join(format!("quotascope-prime-{}", std::process::id()));
        let sessions = root.join("sessions");
        let child = sessions.join("child.jsonl");
        let parent = sessions.join("parent.jsonl");
        let fork = sessions.join("fork.jsonl");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            &child,
            concat!(
                r#"{"type":"session","id":"child","rlmDepth":1}"#,
                "\n",
                r#"{"type":"message","id":"c1","timestamp":"2026-10-01T10:00:00Z","message":{"role":"assistant","model":"m1","usage":{"input":10,"output":5}}}"#,
                "\n"
            ),
        )
        .unwrap();
        std::fs::write(
            &parent,
            concat!(
                r#"{"type":"session","id":"parent"}"#,
                "\n",
                r#"{"type":"message","id":"p1","timestamp":"2026-10-01T10:01:00Z","message":{"role":"assistant","model":"m1","usage":{"input":20,"output":10}}}"#,
                "\n",
                r#"{"type":"child_usage_attributed","id":"a1","targetId":"p1","childUsage":{"input":10,"output":5},"aggregateUsage":{"input":20,"output":10}}"#,
                "\n"
            ),
        )
        .unwrap();
        std::fs::write(
            &fork,
            concat!(
                r#"{"type":"session","id":"fork","parentSession":"PARENT_PATH"}"#,
                "\n",
                r#"{"type":"message","id":"p1","timestamp":"2026-10-01T10:01:00Z","message":{"role":"assistant","model":"m1","usage":{"input":20,"output":10}}}"#,
                "\n"
            ),
        )
        .unwrap();
        let fork_text = std::fs::read_to_string(&fork)
            .unwrap()
            .replace("PARENT_PATH", &parent.to_string_lossy().replace('\\', "/"));
        std::fs::write(&fork, fork_text).unwrap();

        // The parser is path-driven, so exercise it directly against these
        // fixtures without touching the process HOME.
        let parent_file = prime_parse_file(&parent);
        let fork_file = prime_parse_file(&fork);
        assert_eq!(parent_file.messages.len(), 1);
        assert_eq!(fork_file.messages.len(), 1);
        assert_eq!(parent_file.attributions.len(), 1);

        let attribution = &parent_file.attributions[0];
        let message = &parent_file.messages[0];
        assert_eq!(&message.5, &attribution.3);
        let child_file = prime_parse_file(&child);
        let child_total: TokenTally = child_file
            .messages
            .iter()
            .fold(TokenTally::default(), |sum, message| sum + message.5);
        assert_eq!(child_total, attribution.2);

        let reduced = TokenTally {
            input: message.5.input.saturating_sub(attribution.2.input),
            cache_write: 0,
            cache_read: 0,
            output: message.5.output.saturating_sub(attribution.2.output),
        };
        assert_eq!(reduced.total(), 15);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn openclaw_events_skip_plumbing_and_keep_bare_totals_counted() {
        let call = serde_json::json!({
            "type": "message",
            "id": "e1",
            "timestamp": 1791943323000_i64,
            "message": {
                "role": "assistant",
                "provider": "anthropic",
                "model": "claude-opus-4.6",
                "usage": {"input": 10, "output": 5, "reasoningTokens": 2}
            }
        });
        let message = openclaw_message(&call).expect("assistant call");
        assert_eq!(message.tally.total(), 15);
        assert_eq!(message.model.as_deref(), Some("claude-opus-4.6"));

        let mirror = serde_json::json!({
            "type": "message",
            "api": "openclaw-transcript",
            "message": {
                "role": "assistant",
                "provider": "openclaw",
                "model": "delivery-mirror",
                "usage": {"input": 999}
            }
        });
        assert!(openclaw_message(&mirror).is_none());

        let bare = serde_json::json!({
            "type": "message",
            "message": {
                "role": "assistant",
                "model": "m",
                "usage": {"totalTokens": 42}
            }
        });
        // Our ledger carries classified kinds only, matching the Qwen and
        // Prime rules: a total that names no kind is skipped rather than
        // filed as a fabricated zero.
        assert!(openclaw_message(&bare).is_none());
    }

    #[test]
    fn junie_session_time_parses_local_stamps() {
        let at = junie_session_time("session-261001-120000").expect("parses");
        let day = day_of_ms(at);
        assert_eq!(day.to_string(), "2026-10-01");
        assert!(junie_session_time("other").is_none());
    }

    #[test]
    fn mux_model_key_strips_the_provider_prefix() {
        let key = "anthropic:claude-opus-4.6";
        assert_eq!(
            key.split_once(':').map(|(_, model)| model.to_string()),
            Some("claude-opus-4.6".to_string())
        );
    }

    #[test]
    fn lmstudio_scanner_finds_balanced_usage_and_log_times() {
        let text = r#"2026-10-01 10:02:03 "id":"abc","model":"m1","usage": {"prompt_tokens": 10, "nested": {"output_tokens": 5}}"#;
        let marker = text.find(r#""usage""#).unwrap();
        let (start, end) = balanced_object(&text[marker + r#""usage""#.len()..]).expect("balanced");
        let usage: serde_json::Value = serde_json::from_str(
            &text[marker + r#""usage""#.len() + start..marker + r#""usage""#.len() + end],
        )
        .unwrap();
        assert_eq!(usage["prompt_tokens"], 10);
        assert_eq!(usage["nested"]["output_tokens"], 5);
        let at = last_log_time(text).expect("log time");
        assert_eq!(day_of_ms(at).to_string(), "2026-10-01");
        assert_eq!(last_quoted_field(text, "model").as_deref(), Some("m1"));
    }

    #[test]
    fn reasonix_rows_skip_turn_markers_and_bare_totals() {
        let call = serde_json::json!({
            "ts": "2026-10-01T10:00:00Z",
            "model": "provider/m1",
            "prompt": 100,
            "completion": 20,
            "cache_hit": 30,
            "total": 120,
            "requests": 1
        });
        assert!(json_count(call.get("total")).unwrap() > 0);
        let marker = serde_json::json!({"turn": true});
        assert_eq!(
            marker.get("turn").and_then(serde_json::Value::as_bool),
            Some(true)
        );
    }
}
