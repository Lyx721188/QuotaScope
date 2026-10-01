//! The model price list, ported from `ModelPrices.swift`.
//!
//! Straight from models.dev, which publishes the providers' own list prices.
//! Missing rates stay `None` rather than falling back to a plausible number:
//! a model with no price is left out of the total and counted separately, so
//! a figure on screen is never part guesswork.
//!
//! What an agent writes for a model is often not the id models.dev publishes,
//! and an unmatched spelling reads to the user as a model with no price at all.
//! [`aliases`] therefore covers a documented list of those spellings — a
//! provider's display name, a vendor written in front of the id, an effort or a
//! serving arm written on the end — and stops there. Only the *spelling* is
//! normalised; the rate always comes from the published entry.
//!
//! The table is refreshed on the next read after 24 hours, and the cached copy
//! keeps the estimate working offline. Failed downloads may retry after five
//! minutes. Prices are the base rates: some models charge more above a
//! long-context threshold, and that tier is not applied — the transcripts
//! record how many tokens a request used, not how full its context was, so
//! honouring the tier would mean guessing which side of the line each request
//! fell on.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// What one model charges, per million tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    /// How the provider writes the model's name — "GPT-5.6 Sol" rather than
    /// `gpt-5.6-sol`. Optional so an older cached file still decodes.
    #[serde(default)]
    pub name: Option<String>,
}

/// The providers whose models the ledgers can see usage for, **in priority
/// order** — the first one in this list to publish an id wins it, because
/// model ids are unique within a provider and not across all of them.
const PROVIDERS: &[&str] = &[
    "anthropic",
    "openai",
    "xai",
    "moonshotai",
    "zhipuai",
    "minimax",
    "deepseek",
    "google",
    "xiaomi",
    "alibaba",
    "mistral",
    "meta",
];

/// The plan vendors an agent can be priced against when **no first-party
/// provider publishes the model at all**, keyed by the models.dev id.
///
/// This is not a second guess at the same number, it is a different question.
/// Resellers stay out of the first-party list; they belong here, where they
/// are only ever consulted for the agent whose plan they are. Stored
/// namespaced (`vendor|id`) so a vendor's price can never be found by a
/// lookup that did not ask for that vendor.
const VENDORS: &[&str] = &["opencode-go", "kilo", "cline-pass"];

/// Separates a vendor from a model id in the table. Not a character any
/// models.dev id uses.
pub const VENDOR_SEPARATOR: char = '|';

pub fn vendor_key(vendor: &str, model: &str) -> String {
    format!("{vendor}{VENDOR_SEPARATOR}{model}")
}

const SOURCE: &str = "https://models.dev/api.json";
const REFRESH_AFTER_MS: i64 = 24 * 3600 * 1000;
const RETRY_AFTER_MS: i64 = 5 * 60 * 1000;
/// Version 4 includes namespaced plan-vendor rates. The number is part of the
/// contract: a cache written by an older shape must not parse as nothing at
/// all, silently.
const CACHE_FILE: &str = "model-prices-4.json";

#[derive(Serialize, Deserialize)]
struct Cache {
    fetched_at_ms: i64,
    prices: BTreeMap<String, ModelPrice>,
}

struct State {
    cache: Option<Cache>,
    /// Epoch ms after which a download may run again. `None` until the first
    /// read decides it.
    next_fetch_at_ms: Option<i64>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The price table for the id → rate lookup, downloading or reading the cache
/// as their ages demand. An empty map is a valid answer: nothing gets priced,
/// and the ledger counts the tokens as unpriced instead of inventing money.
pub fn prices() -> BTreeMap<String, ModelPrice> {
    let mut guard = STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = now_ms();

    if let Some(state) = guard.as_mut() {
        if let Some(next) = state.next_fetch_at_ms {
            if now < next {
                return state
                    .cache
                    .as_ref()
                    .map(|c| c.prices.clone())
                    .unwrap_or_default();
            }
        }
    } else {
        *guard = Some(State {
            cache: read_cache(),
            next_fetch_at_ms: None,
        });
        // A cache that is still fresh answers without a download.
        if let Some(state) = guard.as_mut() {
            if let Some(cache) = state.cache.as_ref() {
                let expires = cache.fetched_at_ms + REFRESH_AFTER_MS;
                if now < expires {
                    state.next_fetch_at_ms = Some(expires);
                    return cache.prices.clone();
                }
            }
        }
    }

    let state = guard.as_mut().expect("state initialised above");
    match download() {
        Some(table) if !table.is_empty() => {
            let cache = Cache {
                fetched_at_ms: now,
                prices: table,
            };
            write_cache(&cache);
            state.next_fetch_at_ms = Some(now + REFRESH_AFTER_MS);
            state.cache = Some(cache);
        }
        // Keep the last table without renewing its age. Repeated reads
        // offline are bounded by the retry delay.
        _ => {
            state.next_fetch_at_ms = Some(now + RETRY_AFTER_MS);
        }
    }

    state
        .cache
        .as_ref()
        .map(|c| c.prices.clone())
        .unwrap_or_default()
}

fn read_cache() -> Option<Cache> {
    let data = std::fs::read(crate::data_dir().join(CACHE_FILE)).ok()?;
    serde_json::from_slice(&data).ok()
}

fn write_cache(cache: &Cache) {
    let path = crate::data_dir().join(CACHE_FILE);
    if let Ok(data) = serde_json::to_vec(cache) {
        let _ = std::fs::create_dir_all(crate::data_dir());
        let _ = std::fs::write(path, data);
    }
}

/// The one place that decides what a model costs. `None` when the table has
/// no entry the transcript's spelling can honestly reach.
pub fn price_for(
    model: &str,
    table: &BTreeMap<String, ModelPrice>,
    vendor: Option<&str>,
) -> Option<ModelPrice> {
    if let Some(price) = first_party(model, table) {
        return Some(price);
    }

    // Only now, and only for the vendor asked about: the plan the tokens were
    // bought on is the last word, never the first.
    let vendor = vendor?;
    if let Some(exact) = table.get(&vendor_key(vendor, model)) {
        return Some(exact.clone());
    }
    let lowered = vendor_key(vendor, model).to_lowercase();
    if let Some((_, match_)) = table.iter().find(|(k, _)| k.to_lowercase() == lowered) {
        return Some(match_.clone());
    }
    for candidate in aliases(model) {
        if let Some(match_) = table.get(&vendor_key(vendor, &candidate)) {
            return Some(match_.clone());
        }
    }
    None
}

fn first_party(model: &str, table: &BTreeMap<String, ModelPrice>) -> Option<ModelPrice> {
    if let Some(exact) = table.get(model) {
        return Some(exact.clone());
    }

    // MiniMax writes `MiniMax-M3` and the agents that call it write
    // `minimax-m3`. Case is the only difference.
    let lowered = model.to_lowercase();
    if let Some((_, match_)) = table.iter().find(|(k, _)| k.to_lowercase() == lowered) {
        return Some(match_.clone());
    }

    for candidate in aliases(model) {
        if let Some(match_) = table.get(&candidate) {
            return Some(match_.clone());
        }
        let folded = candidate.to_lowercase();
        if let Some((_, match_)) = table.iter().find(|(k, _)| k.to_lowercase() == folded) {
            return Some(match_.clone());
        }
    }

    None
}

/// Spellings to try for one id, most specific first.
///
/// **Aliases, not fuzzy matching.** Every rule here is one product's known
/// habit, written out, because the failure mode of a loose match is a model
/// priced at another model's rate — a wrong number that looks right.
pub fn aliases(model: &str) -> Vec<String> {
    let mut candidates: Vec<String> = Vec::new();

    // Grok Build tags its own build of a model: `grok-4.6-build` is xAI's
    // `grok-4.6`, at xAI's rates.
    if let Some(base) = model.strip_suffix("-build") {
        if model != "grok-build-0.1" {
            candidates.push(base.to_string());
        }
    }

    // Devin's CLI writes the version with dashes and an effort on the end:
    // `gpt-5-6-sol-medium` is OpenAI's `gpt-5.6-sol`.
    for effort in ["-extra-low", "-medium", "-high", "-low", "-minimal"] {
        push_bases(&mut candidates, model, effort);
    }

    // Which arm served the turn, written onto the model rather than beside it.
    // Antigravity's own conversation databases record
    // `gemini-3.8-flash-control` under the same `model_enum` as
    // `gemini-3.8-flash`, so the arm is that model spelling one of its
    // channels. Thinking is billed at the model's own token rates — the rates
    // models.dev publishes for `gemini-3.5-flash-thinking` are
    // `gemini-3.5-flash`'s — and Antigravity writes
    // `claude-opus-4-6-thinking` for what its label calls
    // "Claude Opus 4.6 (Thinking)". An experimental build is not a spelling:
    // `gemini-3.7-flash-exp-b` stays unpriced, because no published rate covers
    // a build nobody has published.
    for arm in ["-thinking", "-control"] {
        push_bases(&mut candidates, model, arm);
    }

    candidates.push(dotted(model));

    // A context window on the end is the same model with more room:
    // `k3-256k` is `kimi-k3`, and it is billed at `k3`'s rates.
    if let Some(base) = strip_context_tag(model) {
        candidates.push(base.clone());
        candidates.extend(aliases(&base));
    }

    // Local servers and HuggingFace-style clients name the vendor inside the
    // id: LM Studio logs `deepseek/deepseek-v4-flash` for the model DeepSeek
    // publishes. The vendor half is decoration; the rate is the model's.
    if let Some(tail) = model.rsplit_once('/').map(|(_, tail)| tail) {
        if !tail.is_empty() {
            candidates.push(tail.to_string());
            candidates.extend(aliases(tail));
        }
    }

    // The provider's own display name, where a client writes that instead of
    // the id. Names keep a version's dot ("Claude Opus 4.6") while ids may spell
    // the same version with a dash ("claude-opus-4-6"), so both separators are
    // tried before the name is called a model nobody prices.
    if let Some(slug) = display_name_slug(model) {
        for form in [slug.clone(), dashed(&slug)] {
            candidates.push(form.clone());
            candidates.extend(aliases(&form));
        }
    }

    // Kimi's CLI abbreviates: `k2p6` is `kimi-k2.6`, `k3` is `kimi-k3`.
    let mut chars = model.chars();
    if chars.next() == Some('k')
        && chars.all(|c| c.is_ascii_digit() || c == 'p')
        && !model.is_empty()
    {
        candidates.push(format!("kimi-{}", model.replace('p', ".")));
    }

    candidates.retain(|c| c != model);
    candidates.dedup();
    candidates
}

/// The id left when one known suffix is dropped, its dotted spelling, and the
/// aliases of that id.
fn push_bases(candidates: &mut Vec<String>, model: &str, suffix: &str) {
    let Some(base) = model.strip_suffix(suffix) else {
        return;
    };
    if base.is_empty() {
        return;
    }
    candidates.push(base.to_string());
    candidates.push(dotted(base));
    candidates.extend(aliases(base));
}

/// `Gemini 3.7 Flash (High)` → `gemini-3.7-flash`: a provider's display name is
/// its id with capitals, spaces, and on the end a qualifier the published rate
/// does not answer to. Anything that does not read as a name returns `None`, so
/// a published id never reaches this rule by accident.
fn display_name_slug(model: &str) -> Option<String> {
    if !model.contains(' ') && !model.contains('(') {
        return None;
    }
    let mut slug = String::with_capacity(model.len());
    let mut parenthesised = 0usize;
    for character in model.chars() {
        match character {
            '(' => parenthesised += 1,
            ')' => parenthesised = parenthesised.saturating_sub(1),
            c if parenthesised == 0 && c.is_alphanumeric() => slug.extend(c.to_lowercase()),
            // A version number keeps its dot: the id is `gemini-3.8-flash`, not
            // `gemini-38-flash`.
            '.' if parenthesised == 0 => slug.push('.'),
            c if parenthesised == 0
                && matches!(c, ' ' | '-' | '_' | ':' | '/')
                && !slug.is_empty()
                && !slug.ends_with('-') =>
            {
                slug.push('-');
            }
            _ => {}
        }
    }
    let slug = slug.trim_matches(['-', '.']).to_string();
    // A single word is not a model id, and a name with no separator between its
    // version and its tier would be a coincidence rather than a match.
    (slug.contains('-') && slug.len() > 3).then_some(slug)
}

/// `k3-256k` → `k3`: a dash, digits, and one of k/K/m/M on the very end.
fn strip_context_tag(model: &str) -> Option<String> {
    let tag_start = model.rfind('-')?;
    let tag = &model[tag_start + 1..];
    if tag.is_empty() {
        return None;
    }
    let (digits, unit) = tag.split_at(tag.len() - 1);
    let is_unit = matches!(unit, "k" | "K" | "m" | "M");
    if is_unit && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        return Some(model[..tag_start].to_string());
    }
    None
}

/// `gpt-5-6-sol` → `gpt-5.6-sol`: a digit, a dash, a digit is a version
/// number somebody spelled with the wrong separator. Two words joined by a
/// dash are left alone.
fn dotted(model: &str) -> String {
    flip_version_separator(model, '-', '.')
}

/// `claude-opus-4.6` → `claude-opus-4-6`: the other direction of the same
/// mistake. Providers publish both shapes — Anthropic dashes its versions,
/// Google dots them — so a name has to be asked in both.
fn dashed(model: &str) -> String {
    flip_version_separator(model, '.', '-')
}

fn flip_version_separator(model: &str, from: char, to: char) -> String {
    let chars: Vec<char> = model.chars().collect();
    let mut out = String::with_capacity(model.len());
    for (index, &character) in chars.iter().enumerate() {
        let version_mark = character == from
            && index > 0
            && index + 1 < chars.len()
            && chars[index - 1].is_ascii_digit()
            && chars[index + 1].is_ascii_digit();
        out.push(if version_mark { to } else { character });
    }
    out
}

fn download() -> Option<BTreeMap<String, ModelPrice>> {
    const NO_HEADERS: &[(&str, &str)] = &[];
    let client = crate::http::HttpClient::new();
    let root = client
        .fetch_json(crate::http::Method::Get, SOURCE, NO_HEADERS, None)
        .ok()?;

    let mut prices: BTreeMap<String, ModelPrice> = BTreeMap::new();
    for provider in PROVIDERS {
        let models = root
            .get(*provider)
            .and_then(|p| p.get("models"))
            .and_then(Value::as_object);
        let Some(models) = models else { continue };
        for (id, model) in models {
            // First provider in the list wins a shared id.
            if prices.contains_key(id) {
                continue;
            }
            let Some(price) = read_price(model) else {
                continue;
            };
            prices.insert(id.clone(), price);
        }
    }

    // The plan vendors, namespaced, and only for ids the first-party
    // providers did not already price: a vendor re-listing somebody else's
    // model must not shadow that model's own rate.
    for vendor in VENDORS {
        let models = root
            .get(*vendor)
            .and_then(|p| p.get("models"))
            .and_then(Value::as_object);
        let Some(models) = models else { continue };
        for (id, model) in models {
            if prices.contains_key(id) {
                continue;
            }
            let Some(price) = read_price(model) else {
                continue;
            };
            prices.insert(vendor_key(vendor, id), price);
        }
    }

    Some(prices)
}

fn read_price(model: &Value) -> Option<ModelPrice> {
    let cost = model.get("cost")?;
    let input = number(cost.get("input"))?;
    let output = number(cost.get("output"))?;
    Some(ModelPrice {
        input,
        output,
        cache_read: number(cost.get("cache_read")),
        cache_write: number(cost.get("cache_write")),
        name: model
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn number(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value.as_f64().or_else(|| value.as_i64().map(|i| i as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(entries: &[(&str, f64, f64)]) -> BTreeMap<String, ModelPrice> {
        entries
            .iter()
            .map(|(id, input, output)| {
                (
                    id.to_string(),
                    ModelPrice {
                        input: *input,
                        output: *output,
                        cache_read: None,
                        cache_write: None,
                        name: None,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn dotted_turns_version_dashes_into_dots_only_between_digits() {
        assert_eq!(dotted("gpt-5-6-sol"), "gpt-5.6-sol");
        assert_eq!(dotted("kimi-k3"), "kimi-k3");
        assert_eq!(dotted("claude-opus-4-6"), "claude-opus-4.6");
    }

    #[test]
    fn aliases_cover_the_documented_habits() {
        assert_eq!(aliases("grok-4.6-build"), vec!["grok-4.6"]);
        // The dotted form of the whole id is always tried too — it just does
        // nothing here, because "medium" is not a version number.
        assert_eq!(
            aliases("gpt-5-6-sol-medium"),
            vec!["gpt-5-6-sol", "gpt-5.6-sol", "gpt-5.6-sol-medium"]
        );
        assert!(aliases("k3-256k").contains(&"k3".to_string()));
        assert_eq!(aliases("k2p6"), vec!["kimi-k2.6"]);
        // A model that is already a table id gets no aliases.
        assert!(aliases("claude-opus-4.6").is_empty());
    }

    #[test]
    fn first_party_matches_case_insensitively_and_through_aliases() {
        let prices = table(&[("MiniMax-M3", 1.0, 2.0), ("kimi-k3", 0.5, 1.5)]);
        assert_eq!(
            price_for("minimax-m3", &prices, None),
            Some(prices["MiniMax-M3"].clone())
        );
        assert_eq!(
            price_for("k3-256k", &prices, None),
            Some(prices["kimi-k3"].clone())
        );
        assert_eq!(price_for("unknown-model", &prices, None), None);
    }

    /// Spellings taken from this machine's own records on 2026-10-01: the ids
    /// agents write, and the names Antigravity's databases store beside them.
    #[test]
    fn documented_spellings_reach_the_published_rate() {
        let prices = table(&[
            ("gemini-3.8-flash", 0.75, 3.75),
            ("gemini-3.7-flash", 0.75, 3.75),
            ("gemini-3.5-flash", 1.5, 9.0),
            ("claude-opus-4-6", 5.0, 25.0),
            ("deepseek-v4-flash", 0.2, 0.4),
            ("MiniMax-M2.7", 0.3, 1.2),
        ]);
        for (spelling, id) in [
            // Antigravity writes the arm that served the turn onto the id.
            ("gemini-3.8-flash-control", "gemini-3.8-flash"),
            ("claude-opus-4-6-thinking", "claude-opus-4-6"),
            // … or keeps only the provider's display name in its label field.
            ("Gemini 3.7 Flash (High)", "gemini-3.7-flash"),
            ("Claude Opus 4.6 (Thinking)", "claude-opus-4-6"),
            ("Gemini 3.8 Flash", "gemini-3.8-flash"),
            ("MiniMax M2.7", "MiniMax-M2.7"),
            // Antigravity's own spelling of the low effort tier.
            ("gemini-3.5-flash-extra-low", "gemini-3.5-flash"),
            // A local server names the vendor inside the id.
            ("deepseek/deepseek-v4-flash", "deepseek-v4-flash"),
        ] {
            assert_eq!(
                price_for(spelling, &prices, None),
                Some(prices[id].clone()),
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_model_nobody_has_priced_is_not_reached_by_a_spelling_rule() {
        let prices = table(&[
            ("gemini-3.7-flash", 0.75, 3.75),
            ("deepseek-v4-flash", 0.2, 0.4),
        ]);
        // An experimental build is a different model, not an arm of one.
        assert_eq!(price_for("gemini-3.7-flash-exp-b", &prices, None), None);
        // A speed tier is billed on its own: Veo 3.1 fast is not Veo 3.1.
        assert_eq!(
            price_for("deepseek/deepseek-v4-flash-fast", &prices, None),
            None
        );
        assert_eq!(price_for("codex-auto-review", &prices, None), None);
    }

    #[test]
    fn display_names_slug_to_ids_and_plain_ids_are_left_alone() {
        assert_eq!(
            display_name_slug("Gemini 3.7 Flash (High)"),
            Some("gemini-3.7-flash".to_string())
        );
        // A name keeps the version's dot; `dashed` is what turns it into
        // Anthropic's published spelling.
        assert_eq!(
            display_name_slug("Claude Opus 4.6"),
            Some("claude-opus-4.6".to_string())
        );
        assert_eq!(dashed("claude-opus-4.6"), "claude-opus-4-6");
        assert_eq!(dotted("claude-opus-4-6"), "claude-opus-4.6");
        assert_eq!(
            display_name_slug("MiniMax M2.7"),
            Some("minimax-m2.7".to_string())
        );
        assert_eq!(display_name_slug("gemini-3.7-flash"), None);
        assert_eq!(display_name_slug("Grok"), None);
    }

    #[test]
    fn vendor_rates_are_namespaced_and_never_shadow_first_party() {
        let mut prices = table(&[("glm-5.2", 1.0, 2.0)]);
        let vendor_rate = ModelPrice {
            input: 9.0,
            output: 9.0,
            cache_read: None,
            cache_write: None,
            name: None,
        };
        prices.insert(vendor_key("kilo", "glm-5.2"), vendor_rate.clone());

        // First party always wins — the vendor is consulted only when no
        // first-party provider publishes the model at all.
        assert_eq!(
            price_for("glm-5.2", &prices, Some("kilo")),
            Some(prices["glm-5.2"].clone())
        );
        // A vendor-only id is reachable only through that vendor.
        assert_eq!(price_for("kilomodel", &prices, None), None);
        let mut with_vendor = prices.clone();
        with_vendor.insert(vendor_key("opencode-go", "flash-x"), vendor_rate.clone());
        assert_eq!(price_for("flash-x", &with_vendor, None), None);
        assert_eq!(
            price_for("flash-x", &with_vendor, Some("opencode-go")),
            Some(vendor_rate)
        );
    }

    #[test]
    fn context_tag_strips_only_a_trailing_size() {
        assert_eq!(strip_context_tag("k3-256k"), Some("k3".to_string()));
        assert_eq!(strip_context_tag("k3-256K"), Some("k3".to_string()));
        assert_eq!(strip_context_tag("gpt-5.6"), None);
        assert_eq!(strip_context_tag("model-"), None);
    }
}
