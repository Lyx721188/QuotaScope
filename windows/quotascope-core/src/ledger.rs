//! A provider's history, worked out from the logs its own CLI leaves on this
//! machine — ported from `UsageLedger.swift`.
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
fn slot_key_from_ms(ms: i64) -> String {
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
    // Retries and resumed sessions can write the same reply twice; the
    // message id identifies it. This only catches repeats within a file,
    // which is where they actually happen.
    let mut seen: HashSet<String> = HashSet::new();

    for line in lines {
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

        if let Some(id) = message.get("id").and_then(|i| i.as_str()) {
            if !seen.insert(id.to_string()) {
                continue;
            }
        }

        let tally = TokenTally {
            input: int(usage.get("input_tokens")),
            cache_write: int(usage.get("cache_creation_input_tokens")),
            cache_read: int(usage.get("cache_read_input_tokens")),
            output: int(usage.get("output_tokens")),
        };
        if tally.total() <= 0 {
            continue;
        }

        let key = slot_key_from_ms(at_ms);
        *days
            .entry(key)
            .or_default()
            .entry(model.to_string())
            .or_default() += tally;
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
    let mut scanned = Scanned::default();
    let mut days: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut model: Option<String> = None;
    let mut previous: Option<[i64; 4]> = None;

    for line in lines {
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
            model = Some(named.to_string());
        }

        if !is_count
            || payload.get("type").and_then(|t| t.as_str()) != Some("token_count")
            || model.is_none()
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
        let previous_totals = previous.unwrap_or([0; 4]);
        let delta: [i64; 4] = std::array::from_fn(|i| (current[i] - previous_totals[i]).max(0));
        previous = Some(current);

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
            .entry(model.clone().expect("guarded above"))
            .or_default() += tally;
    }

    scanned.days = days;
    scanned
}

/// What has already been counted, so opening a card a second time doesn't
/// re-read a few hundred megabytes of transcripts. A log file is rewritten
/// only by being appended to, so size and modification date together are
/// enough to know nothing changed.
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
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Stamp {
    size: i64,
    modified: f64,
}

impl Stamp {
    fn of(path: &Path) -> Option<Stamp> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs_f64();
        Some(Stamp {
            size: meta.len() as i64,
            modified,
        })
    }
}

impl FileCache {
    fn load(provider: Provider) -> FileCache {
        let path = data_dir().join(format!("ledger-4-{}.json", provider.raw()));
        std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    fn save(&self, provider: Provider) {
        if let Ok(data) = serde_json::to_vec(self) {
            let _ = std::fs::create_dir_all(data_dir());
            let _ = std::fs::write(
                data_dir().join(format!("ledger-4-{}.json", provider.raw())),
                data,
            );
        }
    }
}

fn data_dir() -> PathBuf {
    crate::data_dir()
}

/// The transcript root a provider's CLI writes, when it writes one.
pub fn transcript_root(provider: Provider) -> Option<PathBuf> {
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
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
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

fn parse_file(path: &Path, provider: Provider) -> Scanned {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Scanned::default();
    };
    let lines = text.lines().map(str::to_string);
    match provider {
        Provider::ClaudeCode => parse_claude_code(lines),
        Provider::Codex => parse_codex(lines),
        _ => Scanned::default(),
    }
}

/// Reads every transcript under the provider's root, reusing the on-disk cache
/// for any file whose stamp has not moved.
fn scan(provider: Provider) -> (BTreeMap<String, BTreeMap<String, TokenTally>>, FileCache) {
    let Some(root) = transcript_root(provider) else {
        return (BTreeMap::new(), FileCache::default());
    };
    let mut cache = FileCache::load(provider);
    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut fresh: BTreeMap<String, CachedEntry> = BTreeMap::new();

    let mut files: Vec<PathBuf> = Vec::new();
    collect_jsonl(&root, &mut files);
    files.sort();

    for file in files {
        let Some(stamp) = Stamp::of(&file) else {
            continue;
        };
        let key = file.to_string_lossy().to_string();

        let entry = match cache.files.remove(&key) {
            Some(known) if known.stamp == stamp => known,
            _ => {
                let scanned = parse_file(&file, provider);
                CachedEntry {
                    stamp,
                    days: scanned.days,
                    title: scanned.title,
                    cwd: scanned.cwd,
                }
            }
        };

        for (day, models) in &entry.days {
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

    cache.files = fresh;
    cache.save(provider);
    (buckets, cache)
}

/// Turns buckets into days, slots and money. **Static and free of instance
/// state**, so every caller prices the same way — two ways of turning tokens
/// into dollars in one app is two figures that eventually disagree.
pub fn priced(
    buckets: &BTreeMap<String, BTreeMap<String, TokenTally>>,
    prices: &BTreeMap<String, ModelPrice>,
    vendor: Option<&str>,
) -> UsageLedger {
    if buckets.is_empty() {
        return UsageLedger::empty();
    }

    let mut unpriced: BTreeSet<String> = BTreeSet::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut slots: Vec<Slot> = Vec::new();

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
        let Some(start_ms) = slot_key_to_ms(key) else {
            continue;
        };
        let day = day_of_ms(start_ms);

        let mut tokens = 0i64;
        let mut cost = 0.0f64;
        let mut unpriced_tokens = 0i64;

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

            if let Some(price) = model_prices::price_for(model, prices, vendor) {
                let money = tally.cost_breakdown(&price);
                cost += money.total();
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
    }
}

/// Long enough that moving between rings does not rescan, short enough that
/// "Today" is today's. The upstream card keeps the same lifetime.
const LIFETIME: Duration = Duration::from_secs(5 * 60);

static MEMORY: Mutex<Option<(Instant, HashMap<String, UsageLedger>)>> = Mutex::new(None);

/// The provider's ledger, read through the in-memory cache. Providers without
/// a transcript root come back empty — including every one whose history is
/// not this machine's to know.
pub fn ledger(provider: Provider) -> UsageLedger {
    let Some(_) = transcript_root(provider) else {
        return UsageLedger::empty();
    };
    let raw = provider.raw().to_string();

    let mut guard = MEMORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    entry.1.insert(raw, built.clone());
    built
}

fn build(provider: Provider) -> UsageLedger {
    let (buckets, _) = scan(provider);
    if buckets.is_empty() {
        return UsageLedger::empty();
    }
    let prices = model_prices::prices();
    priced(&buckets, &prices, None)
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
        let guard = MEMORY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let Ok(text) = std::fs::read_to_string(&file) else {
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

    let built = if buckets.is_empty() {
        UsageLedger::empty()
    } else {
        priced(&buckets, &model_prices::prices(), None)
    };
    let mut guard = MEMORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.get_or_insert_with(|| (Instant::now(), HashMap::new()));
    entry.1.insert(KEY.to_string(), built.clone());
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
        let guard = MEMORY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        collect_jsonl(&root, &mut files);
    }
    files.sort();

    let mut buckets: BTreeMap<String, BTreeMap<String, TokenTally>> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let mut session: Option<(String, Option<String>)> = None;
        for (index, line) in text.lines().enumerate() {
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
    let built = if buckets.is_empty() {
        UsageLedger::empty()
    } else {
        priced(buckets, &model_prices::prices(), None)
    };
    let mut guard = MEMORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = guard.get_or_insert_with(|| (Instant::now(), HashMap::new()));
    entry.1.insert(cache_key.to_string(), built.clone());
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
    let Ok(text) = std::fs::read_to_string(path) else {
        return file;
    };
    let mut header_seen = false;
    for line in text.lines() {
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
        let guard = MEMORY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let parsed: Vec<PrimeFile> = files.iter().map(|file| prime_parse_file(file)).collect();

    let mut parent_of: std::collections::HashMap<String, String> = Default::default();
    for file in &parsed {
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
        for (attribution_id, target, child_usage, aggregate_usage) in &file.attributions {
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
        let Some(_session_id) = file.id.clone() else {
            continue;
        };
        for (row_id, response_id, at, provider, model, tally) in &file.messages {
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
        let guard = MEMORY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut model_carry: Option<String> = None;
        for (index, line) in text.lines().enumerate() {
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

/// Gemini CLI's chat JSON and headless JSONL shapes. Session JSON's cache
/// overlap is proven only by a total equal to the non-cache sum; headless
/// prompt-style inputs are cache-inclusive. A bare total is not assigned to
/// a kind here.
pub fn gemini_ledger() -> UsageLedger {
    const KEY: &str = "gemini";
    {
        let guard = MEMORY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (line_index, line) in text.lines().enumerate() {
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
            let Ok(bytes) = std::fs::read(&file) else {
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
        let guard = MEMORY
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Ok(bytes) = std::fs::read(&file) else {
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

    fn price(input: f64, output: f64, cache_read: Option<f64>) -> ModelPrice {
        ModelPrice {
            input,
            output,
            cache_read,
            cache_write: None,
            name: None,
        }
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
}
