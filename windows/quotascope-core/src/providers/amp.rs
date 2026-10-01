//! Amp, the coding agent: its free daily allowance, a paid tier's monthly
//! agent and Orb allowances, and the individual credit balance.
//!
//! Read with an access token the user pastes, from the RPC Amp's own CLI
//! calls: `POST https://ampcode.com/api/internal?userDisplayBalanceInfo`. The
//! reply carries no fields — only `result.displayText`, the same lines
//! `amp usage` prints — so those lines are what is read. The shape is
//! second-hand, from CodexBar's Amp provider and its tests, not from a
//! captured reply.
//!
//! Every figure drawn is one the text states in both halves: "$18.57 of $20
//! remaining", "61% remaining today". Amp's old "time to full" is not read —
//! it was an estimate from the replenishment rate, never a stated reset — and
//! neither are workspace balances, which there is nowhere to show beside the
//! account's own.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{HttpClient, Method};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use std::sync::Arc;

const ENDPOINT: &str = "https://ampcode.com/api/internal?userDisplayBalanceInfo";
/// A paid allowance renews with the billing period, which is a month that is
/// not a fixed length: thirty days is a sort key only.
const MONTH: i64 = 30 * 86_400;

pub struct AmpService {
    http: Arc<HttpClient>,
}

impl AmpService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for AmpService {
    fn provider(&self) -> Provider {
        Provider::Amp
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Amp);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Amp)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        // A read-only RPC: it names the method and passes nothing.
        let body = serde_json::json!({"method": "userDisplayBalanceInfo", "params": {}});
        let reply = match self
            .http
            .fetch_json(Method::Post, ENDPOINT, &refs, Some(&body))
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let text = match display_text(&reply) {
            Ok(text) => text.to_string(),
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let (windows, plan, credits) = reading(&text);
        if windows.is_empty() && credits.is_none() {
            // Nothing understood in the text is one thing; a sign-in screen is
            // another, and it deserves its own sentence.
            let reason = if looks_signed_out(&text) {
                Unavailability::ApiKeyRefused
            } else {
                Unavailability::UnreadableReply
            };
            return ProviderUsage::unavailable(account, reason);
        }
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.origin = Some(self.origin_token().into());
        usage.plan = plan;
        if let Some(credits) = credits {
            // The port's balance display convention (see deepseek.rs): two
            // decimals, the mark after the figure.
            usage.credit_balance = Some(format!("{credits:.2} $"));
            usage.credit_remaining = Some(CreditAmount {
                amount: credits,
                currency: "USD".into(),
            });
        }
        usage
    }
}

/// The envelope decides whether the text is worth reading: Amp answers a dead
/// token with a 200 carrying `ok: false`, so the status alone is not enough.
pub fn display_text(reply: &serde_json::Value) -> Result<&str, Unavailability> {
    if reply.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        let auth_required = reply
            .pointer("/error/code")
            .and_then(|v| v.as_str())
            .is_some_and(|code| code == "auth-required");
        return Err(if auth_required {
            Unavailability::ApiKeyRefused
        } else {
            Unavailability::ServerError
        });
    }
    if reply.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(Unavailability::UnreadableReply);
    }
    let text = reply
        .pointer("/result/displayText")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if text.is_empty() {
        return Err(Unavailability::UnreadableReply);
    }
    Ok(text)
}

/// The lines Amp prints, one at a time. A line this build doesn't know is
/// skipped, never guessed at. Returns the windows — deduplicated by id, the
/// first reading of an allowance standing, and sorted shortest first — the
/// plan's name, and the individual credit balance.
pub fn reading(text: &str) -> (Vec<UsageWindow>, Option<String>, Option<f64>) {
    let cleaned = clean(text);
    let lines: Vec<&str> = cleaned
        .split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .filter(|line| !line.is_empty())
        .collect();
    let mut windows: Vec<UsageWindow> = Vec::new();
    let mut plan: Option<String> = None;
    let mut credits: Option<f64> = None;

    // The dollar form is exact; the percentage one is rounded, so it only
    // counts when the other is not there.
    if let Some(free) = lines
        .iter()
        .find_map(|line| free_dollars(line))
        .or_else(|| lines.iter().find_map(|line| free_percent(line)))
    {
        windows.push(free);
    }
    for line in &lines {
        if let Some((tier_plan, tier_windows)) = tier(line) {
            if plan.is_none() {
                plan = Some(tier_plan);
            }
            windows.extend(tier_windows);
        } else if let Some((legacy_plan, legacy_windows)) = subscription(line) {
            if plan.is_none() {
                plan = Some(legacy_plan);
            }
            windows.extend(legacy_windows);
        } else if credits.is_none() {
            credits = individual_credits(line);
        }
    }
    let mut seen = std::collections::HashSet::new();
    windows.retain(|window| seen.insert(window.id.clone()));
    windows.sort_by_key(|window| window.window_seconds);
    (windows, plan, credits)
}

// MARK: - The lines

/// "Amp Free: $6/$10 remaining (replenishes +$0.5/hour)". It refills by the
/// hour and never turns over, so it has no reset and no length.
fn free_dollars(line: &str) -> Option<UsageWindow> {
    let rest = skip_ws(line);
    let rest = cut(rest, "amp free:")?;
    let rest = skip_ws(rest);
    let rest = rest.strip_prefix('$').unwrap_or(rest);
    let (remaining, rest) = take_number(rest)?;
    let rest = skip_ws(rest);
    let rest = rest.strip_prefix('/')?;
    let rest = skip_ws(rest);
    let rest = rest.strip_prefix('$').unwrap_or(rest);
    let (limit, rest) = take_number(rest)?;
    let rest = take_ws(rest)?;
    cut(rest, "remaining")?;
    let used = used_fraction(amount(remaining), amount(limit))?;
    let mut window = UsageWindow::new(
        "amp.free",
        Kind::Credits,
        Some("Amp Free".into()),
        used,
        86_400,
        None,
    );
    window.reports_length = false;
    window.is_exhausted = used >= 1.0;
    Some(window)
}

/// "Amp Free: 61% remaining today (resets daily)". Daily is stated; the hour
/// it turns over is not, so no reset is given.
fn free_percent(line: &str) -> Option<UsageWindow> {
    let rest = skip_ws(line);
    let rest = cut(rest, "amp free:")?;
    let rest = skip_ws(rest);
    let (remaining, rest) = take_number(rest)?;
    let rest = skip_ws(rest);
    let rest = rest.strip_prefix('%')?;
    let rest = take_ws(rest)?;
    let rest = cut(rest, "remaining")?;
    // (\s+today)?(\s*\(resets daily\))? — either note is Amp stating a length;
    // without them the row claims none.
    let mut rest = rest;
    let mut stated = false;
    if let Some(after) = take_ws(rest).and_then(|rest| cut(rest, "today")) {
        rest = after;
        stated = true;
    }
    if let Some(after) = cut(skip_ws(rest), "(resets daily)") {
        rest = after;
        stated = true;
    }
    let _ = rest;
    let remaining = amount(remaining)?;
    if remaining < 0.0 {
        return None;
    }
    let used = (100.0 - remaining.min(100.0)).max(0.0) / 100.0;
    let mut window = UsageWindow::new(
        "amp.free",
        Kind::Daily,
        Some("Amp Free".into()),
        used,
        86_400,
        None,
    );
    window.reports_length = stated;
    window.is_exhausted = used >= 1.0;
    Some(window)
}

/// "Amp Megawatt Tier: agent usage $18.57 of $20 remaining (93%), orb usage
/// 732.8h of 750h a1.small orb hours remaining (98%) - period 2026-09-13 to
/// 2026-10-13, resets upon renewal in 27 days". Dollars and hours, not the
/// rounded percentages beside them.
fn tier(line: &str) -> Option<(String, Vec<UsageWindow>)> {
    let rest = skip_ws(line);
    let rest = cut(rest, "amp")?;
    let rest = take_ws(rest)?;
    // The plan's name is a lazy run — the shortest one that reaches a
    // "tier:" the rest of the line can follow — so each "tier:" is tried
    // in turn, leftmost first.
    for index in occurrences(rest, "tier:") {
        let head = &rest[..index];
        let name = head.trim();
        if name.is_empty() || !head.ends_with(char::is_whitespace) {
            continue;
        }
        if let Some((agent, orb, tail)) = tier_amounts(&rest[index + "tier:".len()..]) {
            let resets_at = period_end(tail);
            let mut windows = Vec::new();
            if let Some(used) = used_fraction(amount(&agent.0), amount(&agent.1)) {
                windows.push(monthly("amp.agent", None, used, resets_at));
            }
            if let Some((remaining, limit)) = orb {
                if let Some(used) = used_fraction(Some(remaining), Some(limit)) {
                    windows.push(monthly("amp.orb", Some("Orb"), used, resets_at));
                }
            }
            return Some((name.to_string(), windows));
        }
    }
    None
}

/// After "tier:": the agent allowance in dollars, and — where the tier names
/// a1.small machines — the orb allowance in hours.
fn tier_amounts(text: &str) -> Option<((String, String), Option<(f64, f64)>, &str)> {
    let rest = skip_ws(text);
    let rest = cut(rest, "agent usage")?;
    let rest = take_ws(rest)?;
    let rest = rest.strip_prefix('$')?;
    let (remaining, rest) = take_number(rest)?;
    let rest = take_ws(rest)?;
    let rest = cut(rest, "of")?;
    let rest = take_ws(rest)?;
    let rest = rest.strip_prefix('$')?;
    let (limit, rest) = take_number(rest)?;
    let rest = take_ws(rest)?;
    let rest = cut(rest, "remaining")?;
    if !at_word_end(rest) {
        return None;
    }
    let orb = orb_usage(rest);
    Some(((remaining.to_string(), limit.to_string()), orb, rest))
}

/// `\borb usage N h of N h a1\.small orb hours remaining\b` — only the unit
/// Amp names its allowance in; another size of machine would be another
/// allowance.
fn orb_usage(text: &str) -> Option<(f64, f64)> {
    for index in occurrences(text, "orb usage") {
        if !at_word_start(text, index) {
            continue;
        }
        let rest = take_ws(&text[index + "orb usage".len()..])?;
        let (used, rest) = take_number(rest)?;
        let rest = cut(rest, "h")?;
        let rest = take_ws(rest)?;
        let rest = cut(rest, "of")?;
        let rest = take_ws(rest)?;
        let (limit, rest) = take_number(rest)?;
        let rest = cut(rest, "h")?;
        let rest = take_ws(rest)?;
        let rest = cut(rest, "a1.small orb hours remaining")?;
        if at_word_end(rest) {
            return Some((amount(used)?, amount(limit)?));
        }
    }
    None
}

/// The older wording, in percentages remaining: "Amp Megawatt
/// Subscription: 68% other usage and 97% orb usage remaining - resets upon
/// renewal in 5 days", or "Subscription Megawatt: …".
fn subscription(line: &str) -> Option<(String, Vec<UsageWindow>)> {
    let rest = skip_ws(line);
    if let Some(rest) = cut(rest, "amp") {
        let rest = take_ws(rest)?;
        for index in occurrences(rest, "subscription:") {
            let head = &rest[..index];
            let name = head.trim();
            if name.is_empty() || !head.ends_with(char::is_whitespace) {
                continue;
            }
            if let Some((other, orb)) = subscription_tail(&rest[index + "subscription:".len()..]) {
                return Some((name.to_string(), percent_windows(&other, &orb)));
            }
        }
        None
    } else if let Some(rest) = cut(rest, "subscription") {
        let rest = take_ws(rest)?;
        // The name here runs to the first colon the tail can follow.
        for (index, _) in rest.match_indices(':') {
            let name = rest[..index].trim();
            if name.is_empty() {
                continue;
            }
            if let Some((other, orb)) = subscription_tail(&rest[index + 1..]) {
                return Some((name.to_string(), percent_windows(&other, &orb)));
            }
        }
        None
    } else {
        None
    }
}

/// After the colon: `\s*N%\s+other usage and\s+N%\s+orb usage remaining`.
fn subscription_tail(text: &str) -> Option<(String, String)> {
    let rest = skip_ws(text);
    let (other, rest) = take_number(rest)?;
    let rest = skip_ws(rest);
    let rest = rest.strip_prefix('%')?;
    let rest = take_ws(rest)?;
    let rest = cut(rest, "other usage and")?;
    let rest = take_ws(rest)?;
    let (orb, rest) = take_number(rest)?;
    let rest = skip_ws(rest);
    let rest = rest.strip_prefix('%')?;
    let rest = take_ws(rest)?;
    let rest = cut(rest, "orb usage")?;
    let rest = take_ws(rest)?;
    cut(rest, "remaining")?;
    Some((other.to_string(), orb.to_string()))
}

/// The two percentage allowances, used turned from what is left.
fn percent_windows(other: &str, orb: &str) -> Vec<UsageWindow> {
    let parts = [("amp.agent", None, other), ("amp.orb", Some("Orb"), orb)];
    let mut windows = Vec::new();
    for (id, scope, text) in parts {
        let Some(remaining) = amount(text).filter(|remaining| *remaining >= 0.0) else {
            continue;
        };
        let used = (100.0 - remaining.min(100.0)).max(0.0) / 100.0;
        windows.push(monthly(id, scope, used, None));
    }
    windows
}

/// "Individual credits: $18.57 remaining" — the credit balance, read once.
fn individual_credits(line: &str) -> Option<f64> {
    let rest = skip_ws(line);
    let rest = cut(rest, "individual credits:")?;
    let rest = skip_ws(rest);
    let rest = rest.strip_prefix('$').unwrap_or(rest);
    let (number, rest) = take_number(rest)?;
    let rest = take_ws(rest)?;
    cut(rest, "remaining")?;
    amount(number).filter(|value| *value >= 0.0)
}

/// A paid allowance renews with the billing period, which is a month that is
/// not a fixed length: thirty days is a sort key only.
fn monthly(id: &str, scope: Option<&str>, used: f64, resets_at: Option<i64>) -> UsageWindow {
    let mut window = UsageWindow::new(
        id,
        Kind::Monthly,
        scope.map(str::to_string),
        used,
        MONTH,
        resets_at,
    );
    window.reports_length = false;
    window.is_exhausted = used >= 1.0;
    window
}

/// "period 2026-09-13 to 2026-10-13": dates only, so the renewal is taken as
/// the start of that day in UTC. The countdown beside it ("in 27 days") is
/// rounded and moves every refresh, so it is not used.
fn period_end(text: &str) -> Option<i64> {
    for index in occurrences(text, "period") {
        if !at_word_start(text, index) {
            continue;
        }
        let Some(rest) = take_ws(&text[index + "period".len()..]) else {
            continue;
        };
        let Some((start, rest)) = take_date(rest) else {
            continue;
        };
        let Some(rest) = take_ws(rest) else {
            continue;
        };
        let Some(rest) = cut(rest, "to") else {
            continue;
        };
        let Some(rest) = take_ws(rest) else {
            continue;
        };
        let Some((end, rest)) = take_date(rest) else {
            continue;
        };
        if !at_word_end(rest) {
            continue;
        }
        let (Some(start), Some(end)) = (date(start), date(end)) else {
            continue;
        };
        if end > start {
            return Some(end);
        }
    }
    None
}

/// `\d{4}-\d{2}-\d{2}` at the front — nothing longer is a date here.
fn take_date(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    if bytes.len() < 10 {
        return None;
    }
    for (index, byte) in bytes[..10].iter().enumerate() {
        let ok = if index == 4 || index == 7 {
            *byte == b'-'
        } else {
            byte.is_ascii_digit()
        };
        if !ok {
            return None;
        }
    }
    Some((&text[..10], &text[10..]))
}

/// The start of that day in UTC, as milliseconds.
fn date(text: &str) -> Option<i64> {
    let day = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    Some(day.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis())
}

// MARK: - Helpers

/// Used over limit, from what is left of a stated limit. More left than the
/// limit is nothing used, not a negative.
fn used_fraction(remaining: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let remaining = remaining?;
    let limit = limit?;
    if !remaining.is_finite() || !limit.is_finite() || remaining < 0.0 || limit <= 0.0 {
        return None;
    }
    Some(((limit - remaining) / limit).max(0.0))
}

/// The figure without its comma separators — "1,234.5" as Amp prints it.
fn amount(text: &str) -> Option<f64> {
    text.replace(',', "").parse::<f64>().ok()
}

/// A number as Amp prints it — `[0-9][0-9,]*(\.[0-9]+)?` — and what follows.
fn take_number(text: &str) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b',') {
        end += 1;
    }
    // The decimal part needs a digit after the dot; a bare dot is not part
    // of the number.
    if end + 1 < bytes.len() && bytes[end] == b'.' && bytes[end + 1].is_ascii_digit() {
        end += 2;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
    }
    Some((&text[..end], &text[end..]))
}

/// Leading whitespace, all of it — a regex `\s*`.
fn skip_ws(text: &str) -> &str {
    text.trim_start()
}

/// At least one whitespace, as a regex `\s+`: none here means no match.
fn take_ws(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    (trimmed.len() < text.len()).then_some(trimmed)
}

/// A literal matched the way a case-insensitive regex would — the reply's own
/// capitalisation ("Megawatt") is what the plan's name keeps.
fn cut<'a>(text: &'a str, literal: &str) -> Option<&'a str> {
    let bytes = text.as_bytes();
    if bytes.len() >= literal.len()
        && bytes[..literal.len()].eq_ignore_ascii_case(literal.as_bytes())
    {
        Some(&text[literal.len()..])
    } else {
        None
    }
}

/// Where `literal` starts in `text`, ASCII-case-insensitively, leftmost
/// first. The literal is ASCII, so every hit is on a character boundary.
fn occurrences(text: &str, literal: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let needle = literal.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return Vec::new();
    }
    (0..=bytes.len() - needle.len())
        .filter(|start| bytes[*start..*start + needle.len()].eq_ignore_ascii_case(needle))
        .collect()
}

/// A word boundary where a literal just ended: the next character must not
/// continue the word.
fn at_word_end(text: &str) -> bool {
    !text.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
}

/// A word boundary where a literal is about to start at `index`: what comes
/// before must not already be inside a word.
fn at_word_start(text: &str, index: usize) -> bool {
    index == 0 || !text[..index].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
}

/// Terminal colour codes and Markdown bold, which the text may carry.
fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1B}' {
            // ESC '[' [0-9;]* letter — consumed whole when the shape holds.
            let mut probe = chars.clone();
            if probe.next() == Some('[') {
                while probe
                    .peek()
                    .is_some_and(|c| c.is_ascii_digit() || *c == ';')
                {
                    probe.next();
                }
                if probe.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                    probe.next();
                    chars = probe;
                    continue;
                }
            }
        }
        out.push(c);
    }
    out.replace("**", "")
}

/// A text that talks about signing in without saying the account is signed in
/// is Amp's way of refusing a dead token.
fn looks_signed_out(text: &str) -> bool {
    let lower = text.to_lowercase();
    !lower.contains("signed in as")
        && (lower.contains("sign in") || lower.contains("log in") || lower.contains("login"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_dollars_have_no_length_and_no_reset() {
        let window = free_dollars("Amp Free: $6/$10 remaining (replenishes +$0.5/hour)").unwrap();
        assert_eq!(window.id, "amp.free");
        assert_eq!(window.used_fraction, 0.4);
        assert!(!window.reports_length);
        assert!(window.resets_at.is_none());
        assert!(!window.is_exhausted);
        assert_eq!(window.scope.as_deref(), Some("Amp Free"));
    }

    #[test]
    fn free_dollars_exhausted_when_nothing_remains() {
        let window = free_dollars("Amp Free: $0/$10 remaining").unwrap();
        assert!(window.is_exhausted);
    }

    #[test]
    fn free_percent_claims_length_only_when_amp_states_it() {
        let stated = free_percent("Amp Free: 61% remaining today (resets daily)").unwrap();
        assert_eq!(stated.used_fraction, 0.39);
        assert!(stated.reports_length);
        assert!(stated.resets_at.is_none());
        let bare = free_percent("Amp Free: 61% remaining").unwrap();
        assert!(!bare.reports_length);
        let daily = free_percent("Amp Free: 61% remaining (resets daily)").unwrap();
        assert!(daily.reports_length);
    }

    #[test]
    fn tier_windows_carry_the_period_end_and_both_allowances() {
        let line = "Amp Megawatt Tier: agent usage $18.57 of $20 remaining (93%), orb usage \
                    732.8h of 750h a1.small orb hours remaining (98%) - period 2026-09-13 to \
                    2026-10-13, resets upon renewal in 27 days";
        let (plan, windows) = tier(line).unwrap();
        assert_eq!(plan, "Megawatt");
        assert_eq!(windows.len(), 2);
        let agent = &windows[0];
        assert_eq!(agent.id, "amp.agent");
        assert_eq!(agent.kind, Kind::Monthly);
        assert!((agent.used_fraction - 1.43 / 20.0).abs() < 1e-12);
        assert!(!agent.reports_length);
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 10, 13)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        assert_eq!(agent.resets_at, Some(expected));
        let orb = &windows[1];
        assert_eq!(orb.id, "amp.orb");
        assert_eq!(orb.scope.as_deref(), Some("Orb"));
        assert!((orb.used_fraction - (750.0 - 732.8) / 750.0).abs() < 1e-12);
    }

    #[test]
    fn subscription_windows_have_no_reset() {
        let line = "Amp Megawatt Subscription: 68% other usage and 97% orb usage remaining \
                    - resets upon renewal in 5 days";
        let (plan, windows) = subscription(line).unwrap();
        assert_eq!(plan, "Megawatt");
        assert_eq!(windows[0].id, "amp.agent");
        assert_eq!(windows[0].used_fraction, 0.32);
        assert_eq!(windows[1].id, "amp.orb");
        assert_eq!(windows[1].used_fraction, 0.03);
        assert!(windows
            .iter()
            .all(|w| w.resets_at.is_none() && !w.reports_length));
    }

    #[test]
    fn subscription_without_the_amp_prefix() {
        let (plan, windows) =
            subscription("Subscription Megawatt: 68% other usage and 97% orb usage remaining")
                .unwrap();
        assert_eq!(plan, "Megawatt");
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn reading_sorts_dedups_and_carries_plan_and_credits() {
        let text = "Amp Free: $6/$10 remaining (replenishes +$0.5/hour)\n\
                    Individual credits: $18.57 remaining\n\
                    Amp Megawatt Subscription: 68% other usage and 97% orb usage remaining\n\
                    Amp Megawatt Tier: agent usage $18.57 of $20 remaining (93%)";
        let (windows, plan, credits) = reading(text);
        // The tier and subscription lines describe the same allowance; the
        // first one printed stands, and the duplicate ids are dropped. The
        // free day sorts before the monthly pair.
        let ids: Vec<&str> = windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["amp.free", "amp.agent", "amp.orb"]);
        let agent = windows.iter().find(|w| w.id == "amp.agent").unwrap();
        assert_eq!(agent.used_fraction, 0.32);
        assert_eq!(plan.as_deref(), Some("Megawatt"));
        assert_eq!(credits, Some(18.57));
    }

    #[test]
    fn reading_without_a_free_line_takes_the_percent_form() {
        let (windows, _, _) = reading("Amp Free: 61% remaining today (resets daily)");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "amp.free");
        assert!(windows[0].reports_length);
    }

    #[test]
    fn unknown_lines_are_skipped_not_guessed() {
        let (windows, plan, credits) = reading("Something new Amp printed some day");
        assert!(windows.is_empty());
        assert!(plan.is_none());
        assert!(credits.is_none());
    }

    #[test]
    fn terminal_colours_and_bold_are_stripped() {
        let (windows, _, credits) = reading(
            "\u{1B}[1m**Amp Free: $6/$10 remaining**\u{1B}[0m\n\u{1B}[32mIndividual credits: $12 remaining\u{1B}[0m",
        );
        assert_eq!(windows.len(), 1);
        assert_eq!(credits, Some(12.0));
    }

    #[test]
    fn envelope_maps_auth_required_to_a_refused_key() {
        let reply = serde_json::json!({"ok": false, "error": {"code": "auth-required"}});
        assert_eq!(display_text(&reply), Err(Unavailability::ApiKeyRefused));
        let reply = serde_json::json!({"ok": false, "error": {"code": "other"}});
        assert_eq!(display_text(&reply), Err(Unavailability::ServerError));
    }

    #[test]
    fn envelope_without_text_is_unreadable() {
        let reply = serde_json::json!({"ok": true, "result": {"displayText": ""}});
        assert_eq!(display_text(&reply), Err(Unavailability::UnreadableReply));
        let reply = serde_json::json!({"ok": true});
        assert_eq!(display_text(&reply), Err(Unavailability::UnreadableReply));
        let reply = serde_json::json!({"ok": true, "result": {"displayText": "Amp Free: $6/$10 remaining"}});
        assert!(display_text(&reply).is_ok());
    }

    #[test]
    fn a_sign_in_screen_reads_as_a_refused_key() {
        assert!(looks_signed_out("Please sign in at ampcode.com/settings"));
        assert!(!looks_signed_out(
            "signed in as ada@example.com\nAmp Free: $6/$10 remaining"
        ));
        assert!(!looks_signed_out("Amp Free: $6/$10 remaining"));
    }

    #[test]
    fn period_end_refuses_a_backwards_or_broken_period() {
        assert!(period_end("period 2026-10-13 to 2026-09-13, resets").is_none());
        assert!(period_end("period 2026-99-13 to 2026-10-13, resets").is_none());
        assert!(period_end("no period here").is_none());
        assert!(period_end("superperiod 2026-09-13 to 2026-10-13").is_none());
    }
}
