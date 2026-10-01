//! Sakana AI's subscription: a five-hour and a weekly limit, each stated as
//! a percentage on the console's billing page, and the pay-as-you-go credit
//! balance on the same page's other tab.
//!
//! **A page, not an API.** Sakana publishes no usage route; the console
//! renders the figures on the server, so the page is read as HTML with the
//! browser session the user imports: `GET https://console.sakana.ai/billing`,
//! and `?tab=payAsYouGo` for the balance, which that tab alone renders. The
//! markup is second-hand — taken from CodexBar's Sakana provider and its
//! tests, not from a captured page — and the fixtures in the tests say so.
//! The session cookie's name is not from CodexBar, which takes a whole pasted
//! header: the console signs in with Auth.js, whose cookies its sign-in page
//! sets under the `authjs` prefix. Unverified against a signed-in browser.
//!
//! **Redirects are not followed** (the shared client refuses them): a
//! signed-out console redirects to its sign-in page, and a redirect is an
//! expired session.
//!
//! The balance is best effort: a tab that fails, or does not state its figure
//! in dollars, leaves the balance off and the limits standing.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use chrono::TimeZone;
use std::sync::Arc;

/// Where the session lives, for the Settings import button.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["console.sakana.ai"],
    cookies: &["__Secure-authjs.session-token"],
};

const BILLING: &str = "https://console.sakana.ai/billing";
const PAY_AS_YOU_GO: &str = "https://console.sakana.ai/billing?tab=payAsYouGo";

/// The two limits the page names, with the length each one is.
const KNOWN: [(&str, &str, Kind, i64); 2] = [
    ("5-hour", "sakana.five_hour", Kind::FiveHour, 5 * 3_600),
    ("Weekly", "sakana.weekly", Kind::Weekly, 7 * 86_400),
];

pub struct SakanaService {
    http: Arc<HttpClient>,
}

impl SakanaService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// One GET with the session, its status read by hand: the shared fetch
    /// folds 401, 403 and the unfollowed redirect into the refused key,
    /// which is the wrong story for a browser session — those all mean the
    /// session no longer works.
    fn get(&self, url: &str, cookie: &str) -> Result<(u16, String), Unavailability> {
        let response = self
            .http
            .client_for_login()
            .get(url)
            .header("Cookie", cookie)
            // The page is read as a page, and the labels below are the
            // English ones.
            .header("Accept", "text/html,application/xhtml+xml")
            .header("Accept-Language", "en-US,en;q=0.9")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .map_err(|_| Unavailability::UnreadableReply)?;
        Ok((status, text))
    }

    /// One page read as HTML, failures in session terms.
    fn html(&self, url: &str, cookie: &str) -> Result<String, Unavailability> {
        let (status, text) = self.get(url, cookie)?;
        match status {
            200..=299 => Ok(text),
            300..=399 | 401 | 403 => Err(Unavailability::SessionExpired),
            429 => Err(Unavailability::RateLimited),
            _ => Err(Unavailability::ServerError),
        }
    }
}

impl ProviderService for SakanaService {
    fn provider(&self) -> Provider {
        Provider::Sakana
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Sakana);
        let Some(header) = pasted_or_none(keys.api_key(Provider::Sakana)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let Some(cookie) = keep(&header, SESSION.cookies) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        let page = match self.html(BILLING, &cookie) {
            Ok(page) => page,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        // The balance tab is best effort: whatever goes wrong with it, the
        // limits on the main page still stand.
        let tab = self
            .get(PAY_AS_YOU_GO, &cookie)
            .ok()
            .filter(|(status, _)| (200..300).contains(status))
            .map(|(_, body)| body);

        let usage = reading(&page, tab.as_deref(), account);
        if matches!(usage.state, crate::model::State::Live) {
            return ProviderUsage {
                origin: Some(self.origin_token().into()),
                ..usage
            };
        }
        usage
    }
}

/// The reading: the two limits the page reports, the plan its card names,
/// and the balance from the pay-as-you-go tab when that tab answered. A page
/// with neither limit nor balance is not a billing page — unreadable, rather
/// than an account with no limits.
pub fn reading(page: &str, pay_as_you_go: Option<&str>, account: AccountKey) -> ProviderUsage {
    if page.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    let windows: Vec<UsageWindow> = KNOWN
        .iter()
        .filter_map(|(label, id, kind, seconds)| {
            let body = section(label, page)?;
            let percent = percent_used(body)?;
            let mut window = UsageWindow::new(
                id,
                *kind,
                None,
                percent / 100.0,
                *seconds,
                resets_on(body).and_then(|text| reset_date(&text)),
            );
            window.is_exhausted = percent >= 100.0;
            Some(window)
        })
        .collect();
    let balance = pay_as_you_go.and_then(credit_balance);

    if windows.is_empty() && balance.is_none() {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.plan = plan_name(page);
    usage.credit_balance = balance.map(money);
    usage.credit_remaining = balance.map(|amount| CreditAmount {
        amount,
        currency: "USD".into(),
    });
    usage
}

/// The card's "Credit balance", when it is stated in dollars. A figure
/// without its `$` is left off rather than given a currency.
pub fn credit_balance(html: &str) -> Option<f64> {
    let bytes = html.as_bytes();
    // <h2 …> Credit balance </h2>
    let mut at = 0;
    let after_heading = loop {
        let tag = next_tag(bytes, at, "h2")?;
        let cursor = skip_ws(bytes, tag.end);
        if at_ci(bytes, cursor, "credit balance") {
            let tail = skip_ws(bytes, cursor + "credit balance".len());
            if at_ci(bytes, tail, "</h2>") {
                break tail + "</h2>".len();
            }
        }
        at = tag.start + 1;
    };
    // Then, within nine hundred characters of it, the nearest paragraph set
    // in tabular figures carrying a dollar amount. The gap is walked byte by
    // byte: the page is text, and the two counts part only in a page that is
    // not.
    let mut gap = 0;
    while after_heading + gap <= bytes.len() && gap <= 900 {
        if let Some(tag) = tag_at(bytes, after_heading + gap, "p") {
            if tabular_attrs(bytes, tag.attrs.clone()) {
                let cursor = skip_ws(bytes, tag.end);
                if at_ci(bytes, cursor, "$") {
                    if let Some((figure, end)) = dollar_number(bytes, cursor + 1) {
                        let close = skip_ws(bytes, end);
                        if at_ci(bytes, close, "</p>") {
                            return figure
                                .replace(',', "")
                                .parse::<f64>()
                                .ok()
                                .filter(|amount| amount.is_finite() && *amount >= 0.0);
                        }
                    }
                }
            }
        }
        gap += 1;
    }
    None
}

/// The `<p …>32% used</p>` figure in a section. The percentage is the card's
/// own; nothing here invents the share it stands for.
pub fn percent_used(body: &str) -> Option<f64> {
    let bytes = body.as_bytes();
    let mut at = 0;
    while let Some(tag) = next_tag(bytes, at, "p") {
        let cursor = skip_ws(bytes, tag.end);
        if let Some((figure, end)) = plain_number(bytes, cursor) {
            // The figure runs straight into "% used", no room between.
            if at_ci(bytes, end, "% used") {
                let close = skip_ws(bytes, end + "% used".len());
                if at_ci(bytes, close, "</p>") {
                    return figure.parse::<f64>().ok();
                }
            }
        }
        at = tag.start + 1;
    }
    None
}

/// The `<p …>Resets on …</p>` line in a section, verbatim.
pub fn resets_on(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut at = 0;
    while let Some(tag) = next_tag(bytes, at, "p") {
        let cursor = skip_ws(bytes, tag.end);
        if at_ci(bytes, cursor, "resets on") {
            // One space of the pattern belongs to the sentence.
            if bytes.get(cursor + "resets on".len()) == Some(&b' ') {
                let capture = cursor + "resets on".len() + 1;
                // The sentence cannot run past the next tag opening.
                if let Some(end) = bytes[capture..]
                    .iter()
                    .position(|byte| *byte == b'<')
                    .map(|found| capture + found)
                {
                    if at_ci(bytes, end, "</p>") {
                        let value = body[capture..end].trim();
                        if !value.is_empty() {
                            return Some(value.to_string());
                        }
                        return None;
                    }
                }
            }
        }
        at = tag.start + 1;
    }
    None
}

/// "June 23, 2026 at 2:53 PM", as the console renders it on the server, in
/// UTC — only the browser's script moves it to local time afterwards.
pub fn reset_date(text: &str) -> Option<i64> {
    let parts: Vec<&str> = text.trim().split_whitespace().collect();
    let [month, day, year, at, clock, meridiem] = parts.as_slice() else {
        return None;
    };
    let month = month_number(month)?;
    let day: u32 = day.trim_end_matches(',').parse().ok()?;
    let year: i32 = year.parse().ok()?;
    if *at != "at" {
        return None;
    }
    let (hour_text, minute_text) = clock.split_once(':')?;
    let hour: u32 = hour_text.parse().ok()?;
    let minute: u32 = minute_text.parse().ok()?;
    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    let hour = match *meridiem {
        "AM" if hour == 12 => 0,
        "AM" => hour,
        "PM" if hour == 12 => 12,
        "PM" => hour + 12,
        _ => return None,
    };
    chrono::Utc
        .with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .map(|stamp| stamp.timestamp_millis())
}

/// The full English month names the page writes, in order.
fn month_number(name: &str) -> Option<u32> {
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    months
        .iter()
        .position(|named| *named == name)
        .map(|index| index as u32 + 1)
}

/// What follows a limit's label, up to the next limit or the next card.
pub fn section<'a>(label: &str, html: &'a str) -> Option<&'a str> {
    let bytes = html.as_bytes();
    let mut at = 0;
    let start = loop {
        let tag = next_tag(bytes, at, "p")?;
        let cursor = skip_ws(bytes, tag.end);
        // The label is read case-sensitively, the way the page writes it.
        if bytes[cursor..].starts_with(label.as_bytes()) {
            let tail = skip_ws(bytes, cursor + label.len());
            if bytes[tail..].starts_with(b"</p>") {
                break tail + 4;
            }
        }
        at = tag.start + 1;
    };
    let rest = &html[start..];
    let end = section_boundary(rest).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// Where the section ends: the next limit's label paragraph, or the next
/// card's opening — whichever comes first. Both scans answer with their
/// earliest match, so the smaller position wins.
fn section_boundary(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    match (label_paragraph(bytes, 0), card_opening(bytes, 0)) {
        (Some(limit), Some(card)) => Some(limit.min(card)),
        (Some(limit), None) => Some(limit),
        (None, Some(card)) => Some(card),
        (None, None) => None,
    }
}

/// `<p …> 5-hour | Weekly </p>`, letter case aside.
fn label_paragraph(bytes: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(tag) = next_tag(bytes, at, "p") {
        let cursor = skip_ws(bytes, tag.end);
        for name in ["5-hour", "weekly"] {
            if at_ci(bytes, cursor, name) {
                let tail = skip_ws(bytes, cursor + name.len());
                if at_ci(bytes, tail, "</p>") {
                    return Some(tag.start);
                }
            }
        }
        at = tag.start + 1;
    }
    None
}

/// `<div … data-slot="card" | data-slot="card-title" …>`, letter case aside.
fn card_opening(bytes: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(tag) = next_tag(bytes, at, "div") {
        if bytes.get(tag.attrs.clone()).is_some_and(card_slot_attr) {
            return Some(tag.start);
        }
        at = tag.start + 1;
    }
    None
}

/// Whether one tag's attributes carry `data-slot="card"` or the card-title
/// spelling of it.
fn card_slot_attr(attrs: &[u8]) -> bool {
    let mut at = 0;
    while let Some(found) = find_ci(attrs, at, "data-slot=") {
        let value = found + "data-slot=".len();
        if matches!(attrs.get(value), Some(b'"') | Some(b'\'')) {
            let name = value + 1;
            if at_ci(attrs, name, "card") {
                let after = if at_ci(attrs, name + "card".len(), "-title") {
                    name + "card".len() + "-title".len()
                } else {
                    name + "card".len()
                };
                if matches!(attrs.get(after), Some(b'"') | Some(b'\'')) {
                    return true;
                }
            }
        }
        at = found + 1;
    }
    false
}

/// The card title's span: the plan as the console names it.
pub fn plan_name(page: &str) -> Option<String> {
    let bytes = page.as_bytes();
    let mut at = 0;
    while let Some(tag) = next_tag(bytes, at, "div") {
        let carries_title = bytes
            .get(tag.attrs.clone())
            .is_some_and(|attrs| find_ci(attrs, 0, "data-slot=\"card-title\"").is_some());
        if carries_title {
            let cursor = skip_ws(bytes, tag.end);
            if at_ci(bytes, cursor, "<span>") {
                let capture = skip_ws(bytes, cursor + "<span>".len());
                if let Some(end) = bytes[capture..]
                    .iter()
                    .position(|byte| *byte == b'<')
                    .map(|found| capture + found)
                {
                    if at_ci(bytes, end, "</span>") {
                        let value = page[capture..end].trim();
                        if !value.is_empty() {
                            return Some(value.to_string());
                        }
                        return None;
                    }
                }
            }
        }
        at = tag.start + 1;
    }
    None
}

/// Whether a paragraph's attributes set the `tabular-nums` class the balance
/// figure is written in: the class, then non-quote attributes, then a quote.
fn tabular_attrs(bytes: &[u8], attrs: std::ops::Range<usize>) -> bool {
    let Some(attrs) = bytes.get(attrs) else {
        return false;
    };
    let mut at = 0;
    while let Some(found) = find_ci(attrs, at, "tabular-nums") {
        let after = found + "tabular-nums".len();
        let mut cursor = after;
        while cursor < attrs.len() && attrs[cursor] != b'"' {
            cursor += 1;
        }
        if cursor < attrs.len() {
            return true;
        }
        at = found + 1;
    }
    false
}

/// A plain number — digits, then optional decimals — as the percentages are
/// written. The end of the figure comes back with it.
fn plain_number(bytes: &[u8], from: usize) -> Option<(String, usize)> {
    if !bytes.get(from)?.is_ascii_digit() {
        return None;
    }
    let mut at = from + 1;
    while bytes.get(at).is_some_and(|byte| byte.is_ascii_digit()) {
        at += 1;
    }
    let mut end = at;
    if bytes.get(at) == Some(&b'.') {
        let mut decimals = at + 1;
        while bytes
            .get(decimals)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            decimals += 1;
        }
        if decimals > at + 1 {
            end = decimals;
        }
    }
    Some((
        std::str::from_utf8(&bytes[from..end]).ok()?.to_string(),
        end,
    ))
}

/// A dollar figure — a digit, then digits and thousands commas, then
/// optional decimals — commas and all, for the caller to strip.
fn dollar_number(bytes: &[u8], from: usize) -> Option<(String, usize)> {
    if !bytes.get(from)?.is_ascii_digit() {
        return None;
    }
    let mut at = from + 1;
    while bytes
        .get(at)
        .is_some_and(|byte| byte.is_ascii_digit() || *byte == b',')
    {
        at += 1;
    }
    let mut end = at;
    if bytes.get(at) == Some(&b'.') {
        let mut decimals = at + 1;
        while bytes
            .get(decimals)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            decimals += 1;
        }
        if decimals > at + 1 {
            end = decimals;
        }
    }
    Some((
        std::str::from_utf8(&bytes[from..end]).ok()?.to_string(),
        end,
    ))
}

/// An opening tag located in a page: where it starts, where its attributes
/// sit, and where its `>` ends.
#[derive(Debug, Clone)]
struct Tag {
    start: usize,
    attrs: std::ops::Range<usize>,
    end: usize,
}

/// The opening `name` tag at exactly `at`, letter case aside.
fn tag_at(bytes: &[u8], at: usize, name: &str) -> Option<Tag> {
    if bytes.get(at) != Some(&b'<') || !at_ci(bytes, at + 1, name) {
        return None;
    }
    let attrs_start = at + 1 + name.len();
    let mut cursor = attrs_start;
    while cursor < bytes.len() && bytes[cursor] != b'>' {
        cursor += 1;
    }
    if cursor < bytes.len() {
        Some(Tag {
            start: at,
            attrs: attrs_start..cursor,
            end: cursor + 1,
        })
    } else {
        None
    }
}

/// The next opening `name` tag at or after `from`, letter case aside. The
/// attributes end at the first `>`, as the page patterns they stand in read.
fn next_tag(bytes: &[u8], from: usize, name: &str) -> Option<Tag> {
    let mut at = from;
    while let Some(start) = find_byte(bytes, at, b'<') {
        if let Some(tag) = tag_at(bytes, start, name) {
            return Some(tag);
        }
        at = start + 1;
    }
    None
}

fn skip_ws(bytes: &[u8], from: usize) -> usize {
    let mut at = from.min(bytes.len());
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    at
}

/// Whether the bytes at `at` read `text`, letter case aside.
fn at_ci(bytes: &[u8], at: usize, text: &str) -> bool {
    let text = text.as_bytes();
    bytes.len() >= at + text.len() && bytes[at..at + text.len()].eq_ignore_ascii_case(text)
}

fn find_byte(bytes: &[u8], from: usize, byte: u8) -> Option<usize> {
    bytes[from.min(bytes.len())..]
        .iter()
        .position(|seen| *seen == byte)
        .map(|at| at + from.min(bytes.len()))
}

fn find_ci(bytes: &[u8], from: usize, text: &str) -> Option<usize> {
    let text = text.as_bytes();
    if text.is_empty() || bytes.len() < text.len() {
        return None;
    }
    (from..=bytes.len() - text.len())
        .find(|at| bytes[*at..*at + text.len()].eq_ignore_ascii_case(text))
}

/// Keeps only the named cookies out of a pasted `Cookie:` header, and
/// nothing at all if the first name is missing. What is not kept never
/// leaves the process. A name ending in `*` is a prefix; names joined with
/// `|` are alternatives, of which any one in the first entry is enough.
fn keep(header: &str, cookies: &[&str]) -> Option<String> {
    let first = cookies.first()?;
    let required: Vec<&str> = first.split('|').collect();
    let patterns: Vec<&str> = cookies
        .iter()
        .flat_map(|cookie| cookie.split('|'))
        .collect();
    let matches = |name: &str, pattern: &str| -> bool {
        match pattern.strip_suffix('*') {
            Some(prefix) => {
                !prefix.is_empty() && name.starts_with(prefix) && name.len() > prefix.len()
            }
            None => name == pattern,
        }
    };
    let pairs: Vec<(&str, &str)> = header
        .split(';')
        .filter_map(|part| {
            let trimmed = part.trim();
            let equals = trimmed.find('=')?;
            let (name, value) = (&trimmed[..equals], &trimmed[equals + 1..]);
            (!value.is_empty() && patterns.iter().any(|pattern| matches(name, pattern)))
                .then_some((name, value))
        })
        .collect();
    if !pairs
        .iter()
        .any(|(name, _)| required.iter().any(|want| matches(name, want)))
    {
        return None;
    }
    let mut seen: Vec<&str> = Vec::new();
    let kept: Vec<String> = pairs
        .into_iter()
        .filter(|(name, _)| {
            if seen.contains(name) {
                return false;
            }
            seen.push(name);
            true
        })
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    Some(kept.join("; "))
}

/// Dollars, as the console prices them: two decimals.
pub fn money(amount: f64) -> String {
    format!("${amount:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn account() -> AccountKey {
        AccountKey::primary(Provider::Sakana)
    }

    /// The billing page's shape, as CodexBar's Sakana fixtures draw it —
    /// second-hand, not captured from a live console.
    const PAGE: &str = concat!(
        "<main>",
        "<div data-slot=\"card\"><div data-slot=\"card-title\"><span>Pro</span></div>",
        "<p>5-hour</p><p>32% used</p><p>Resets on June 23, 2026 at 2:53 PM</p></div>",
        "<div data-slot=\"card\"><div data-slot=\"card-title\"><span>Pro</span></div>",
        "<p>Weekly</p><p>8.5% used</p><p>Resets on June 29, 2026 at 12:00 PM</p></div>",
        "</main>"
    );

    /// The pay-as-you-go tab's balance card.
    const TAB: &str = concat!(
        "<div data-slot=\"card\"><h2>Credit balance</h2>",
        "<p class=\"text-2xl tabular-nums\">$12.50</p></div>"
    );

    #[test]
    fn both_limits_and_the_plan_are_read_from_the_page() {
        let usage = reading(PAGE, None, account());
        assert!(matches!(usage.state, State::Live));
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            vec!["sakana.five_hour", "sakana.weekly"]
        );
        let five_hour = &usage.windows[0];
        assert_eq!(five_hour.kind, Kind::FiveHour);
        assert_eq!(five_hour.used_fraction, 0.32);
        assert_eq!(five_hour.window_seconds, 5 * 3_600);
        assert!(five_hour.reports_length);
        assert_eq!(five_hour.resets_at, reset_date("June 23, 2026 at 2:53 PM"));
        let weekly = &usage.windows[1];
        assert_eq!(weekly.kind, Kind::Weekly);
        assert_eq!(weekly.used_fraction, 0.085);
        assert_eq!(usage.plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn the_balance_comes_from_the_pay_as_you_go_tab_alone() {
        let usage = reading(PAGE, Some(TAB), account());
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 12.5,
                currency: "USD".into()
            })
        );
        assert_eq!(usage.credit_balance.as_deref(), Some("$12.50"));

        // A tab that fails, or that hides its figure, leaves the limits
        // standing.
        let usage = reading(PAGE, None, account());
        assert!(usage.credit_remaining.is_none());
        assert_eq!(usage.windows.len(), 2);
    }

    #[test]
    fn a_spent_limit_is_exhausted_and_a_page_without_figures_is_unreadable() {
        let page = "<div data-slot=\"card\"><p>5-hour</p><p>100% used</p><p>Resets on June 23, 2026 at 2:53 PM</p></div>";
        let usage = reading(page, None, account());
        assert!(usage.windows[0].is_exhausted);
        assert_eq!(usage.windows[0].used_fraction, 1.0);

        // No percent paragraph and no balance: an unreadable page rather
        // than an account without limits.
        let page = "<div data-slot=\"card\"><p>5-hour</p><p>soon</p></div>";
        assert_eq!(
            reading(page, None, account()).state,
            State::Unavailable(Unavailability::UnreadableReply)
        );
        assert_eq!(
            reading("", Some(TAB), account()).state,
            State::Unavailable(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn a_reset_line_without_a_clock_makes_no_date() {
        let body = "<p>5-hour</p><p>32% used</p><p>Resets on soon</p>";
        assert_eq!(percent_used(body), Some(32.0));
        assert_eq!(resets_on(body).as_deref(), Some("soon"));
        assert_eq!(reset_date("soon"), None);
        // A reset line that names no date leaves the reset off, the limit on.
        let usage = reading(
            "<div data-slot=\"card\"><p>Weekly</p><p>8.5% used</p><p>Resets on soon</p></div>",
            None,
            account(),
        );
        assert_eq!(usage.windows[0].resets_at, None);
    }

    #[test]
    fn the_balance_figure_may_carry_thousands_and_sit_far_below_its_heading() {
        let tab = "<h2>Credit balance</h2><div></div><p class=\"other tabular-nums\">$1,234.50</p>";
        assert_eq!(credit_balance(tab), Some(1234.5));
        // No dollar sign, no balance; no heading, no balance.
        assert_eq!(
            credit_balance("<h2>Credit balance</h2><p class=\"tabular-nums\">12.50</p>"),
            None
        );
        assert_eq!(
            credit_balance("<h2>Other</h2><p class=\"tabular-nums\">$5</p>"),
            None
        );
    }

    #[test]
    fn a_section_stops_at_the_next_limit_or_the_next_card() {
        let body = section("5-hour", PAGE).unwrap();
        assert!(body.contains("32% used"));
        assert!(!body.contains("8.5% used"));
        let body = section("Weekly", PAGE).unwrap();
        assert!(body.contains("8.5% used"));
        // A label the page does not carry has no section.
        assert_eq!(section("Daily", PAGE), None);
    }

    #[test]
    fn resets_render_in_utc_and_read_back_as_milliseconds() {
        assert_eq!(
            reset_date("June 23, 2026 at 2:53 PM"),
            Some(1_782_226_380_000)
        );
        // Midnight is midnight, not noon.
        assert_eq!(
            reset_date("June 29, 2026 at 12:00 AM"),
            Some(1_782_691_200_000)
        );
        assert_eq!(
            reset_date("June 29, 2026 at 12:00 PM"),
            Some(1_782_734_400_000)
        );
        for text in [
            "June 23, 2026",            // no clock
            "June 23, 2026 at 2:53",    // no meridiem
            "Fune 23, 2026 at 2:53 PM", // no such month
            "June 0, 2026 at 2:53 PM",  // no such day
            "June 23, 2026 at 0:53 PM", // no such hour
            "",                         // nothing at all
        ] {
            assert_eq!(reset_date(text), None, "{text}");
        }
    }

    #[test]
    fn only_the_authjs_session_cookie_is_kept_and_not_without_it() {
        assert_eq!(
            keep(
                "__Host-next-auth.csrf-token=a; __Secure-authjs.session-token=good; theme=dark",
                SESSION.cookies
            ),
            Some("__Secure-authjs.session-token=good".to_string())
        );
        // An empty value is no session, and a header without the session
        // cookie keeps nothing at all.
        assert_eq!(
            keep("__Secure-authjs.session-token=", SESSION.cookies),
            None
        );
        assert_eq!(keep("theme=dark", SESSION.cookies), None);
        assert_eq!(keep("", SESSION.cookies), None);
    }
}
