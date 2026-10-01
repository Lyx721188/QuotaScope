//! TypeSafe: the credit balance on the console's billing page, read with the
//! signed-in console session.
//!
//! **A pasted Cookie header, for now.** The console's session cookie has no
//! name anyone has written down — CodexBar deliberately keeps every cookie
//! the console sets rather than assume one — so the profile pins none and
//! the field takes the `Cookie` header copied from a request to
//! `console.typesafe.ai/settings/billing`, sent on whole.
//!
//! The billing figures come from a Next.js server action, not an API:
//!
//! 1. The action's id is found in the billing page's own scripts — the page
//!    lists its chunks, and one of them names `getBillingOverviewResult`
//!    beside its id. Found once and kept for twelve hours; the page and
//!    chunks are fetched again only when the id goes stale (the action
//!    answers 404).
//! 2. `POST /settings/billing` with `Next-Action: <id>` and a body of `[]` —
//!    the read the page itself makes. It changes nothing.
//! 3. The reply is React's line format: `n:{json}` per line, and the line
//!    with an `ok` key is the result.
//!
//! The shapes are second-hand — taken from CodexBar's TypeSafe plugin and
//! its tests, not from captured replies — and the fixtures in the tests say
//! so.
//!
//! **What is left out, and why.** The cycle's spend has no limit to measure
//! it against and nowhere to go yet. Each credit grant's size and remainder
//! are reported, but a grant expires rather than resets, and the balance
//! already sums what is left of them.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, Unavailability};
use std::sync::{Arc, Mutex};

/// Where the session lives, for the Settings import button. No cookie name
/// is pinned: the console's session has no name anyone has written down, so
/// the whole pasted header is what travels.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["console.typesafe.ai"],
    cookies: &[],
};

const ORIGIN: &str = "https://console.typesafe.ai";
/// The origin with the path separator on — what "same origin" is read as.
const ORIGIN_ROOT: &str = "https://console.typesafe.ai/";
const BILLING_PAGE: &str = "https://console.typesafe.ai/settings/billing";

/// How long a found action id is trusted before the page is read again.
const ACTION_LIFETIME_SECS: u64 = 12 * 3_600;
/// At most this many of the page's scripts are opened looking for it.
const MAXIMUM_CHUNKS: usize = 60;

pub struct TypeSafeService {
    http: Arc<HttpClient>,
    /// The action id, kept between fetches. One console, one id.
    action: Mutex<Option<(String, std::time::Instant)>>,
}

impl TypeSafeService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self {
            http,
            action: Mutex::new(None),
        }
    }

    fn cached_action(&self) -> Option<String> {
        let cached = self.action.lock().ok()?;
        let (id, found_at) = cached.as_ref()?;
        (found_at.elapsed() < std::time::Duration::from_secs(ACTION_LIFETIME_SECS))
            .then(|| id.clone())
    }

    fn remember(&self, id: Option<&str>) {
        if let Ok(mut cached) = self.action.lock() {
            *cached = id.map(|id| (id.to_string(), std::time::Instant::now()));
        }
    }
}

impl ProviderService for TypeSafeService {
    fn provider(&self) -> Provider {
        Provider::TypeSafe
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::TypeSafe);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::TypeSafe))
            .map(|header| cookie_header(&header))
            .filter(|cookie| !cookie.is_empty())
        else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        let mut discovered = false;
        let action = match self.cached_action() {
            Some(action) => action,
            None => {
                discovered = true;
                match self.discover_action(&cookie) {
                    Ok(action) => {
                        self.remember(Some(&action));
                        action
                    }
                    Err(reason) => return ProviderUsage::unavailable(account, reason),
                }
            }
        };

        let mut reply = self.post_action(&action, &cookie);
        // A 404 from the action is Next.js saying the id is stale: the
        // console was redeployed. Find it again, once — but only when the id
        // came out of the cache, not when it is brand new.
        if matches!(&reply, Ok((404, _))) && !discovered {
            self.remember(None);
            let action = match self.discover_action(&cookie) {
                Ok(action) => action,
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            };
            self.remember(Some(&action));
            reply = self.post_action(&action, &cookie);
        }

        let (status, body) = match reply {
            Ok(pair) => pair,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let body = match status {
            200..=299 => body,
            300..=399 | 401 | 403 => {
                return ProviderUsage::unavailable(account, Unavailability::SessionExpired)
            }
            429 => return ProviderUsage::unavailable(account, Unavailability::RateLimited),
            _ => return ProviderUsage::unavailable(account, Unavailability::ServerError),
        };
        let usage = reading(&body, account);
        if matches!(usage.state, crate::model::State::Live) {
            return ProviderUsage {
                origin: Some(self.origin_token().into()),
                ..usage
            };
        }
        usage
    }
}

impl TypeSafeService {
    /// One billing page GET, its status read by hand: the shared fetch folds
    /// 401, 403 and the unfollowed redirect into the refused key, which is
    /// the wrong story for a browser session.
    fn page(&self, cookie: &str) -> Result<String, Unavailability> {
        let response = self
            .http
            .client_for_login()
            .get(BILLING_PAGE)
            .header("Cookie", cookie)
            .header("Accept", "text/html")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        let page = response
            .text()
            .map_err(|_| Unavailability::UnreadableReply)?;
        match status {
            200..=299 => Ok(page),
            300..=399 | 401 | 403 => Err(Unavailability::SessionExpired),
            429 => Err(Unavailability::RateLimited),
            _ => Err(Unavailability::ServerError),
        }
    }

    /// The action id, taken from the page's own scripts. A lapsed session
    /// lands on the sign-in page, redirected or not.
    fn discover_action(&self, cookie: &str) -> Result<String, Unavailability> {
        let page = self.page(cookie)?;
        if is_login_landing(&page) {
            return Err(Unavailability::SessionExpired);
        }
        // The scripts are the site's public code: asked for without the
        // cookie.
        for chunk in chunk_urls(&page) {
            let Ok(response) = self
                .http
                .client_for_login()
                .get(&chunk)
                .timeout(std::time::Duration::from_secs(5))
                .send()
            else {
                continue;
            };
            let status = response.status().as_u16();
            if status == 429 {
                return Err(Unavailability::RateLimited);
            }
            if status >= 500 {
                return Err(Unavailability::ServerError);
            }
            if !(200..300).contains(&status) {
                continue;
            }
            let Ok(script) = response.text() else {
                continue;
            };
            if let Some(id) = action_id(&script) {
                return Ok(id);
            }
        }
        Err(Unavailability::UnreadableReply)
    }

    /// The read the page itself makes. It changes nothing.
    fn post_action(&self, action: &str, cookie: &str) -> Result<(u16, String), Unavailability> {
        let response = self
            .http
            .client_for_login()
            .post(BILLING_PAGE)
            .header("Cookie", cookie)
            .header("Origin", ORIGIN)
            .header("Next-Action", action)
            .header("Accept", "text/x-component")
            .header("Content-Type", "application/json")
            .body("[]")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .map_err(|_| Unavailability::UnreadableReply)?;
        Ok((status, body))
    }
}

/// The header as pasted, with a leading `Cookie:` taken off if it came
/// along.
pub fn cookie_header(pasted: &str) -> String {
    let trimmed = pasted.trim();
    if trimmed.as_bytes().len() >= 7 && trimmed.as_bytes()[..7].eq_ignore_ascii_case(b"cookie:") {
        return trimmed[7..].trim().to_string();
    }
    trimmed.to_string()
}

// MARK: - Finding the action

/// Whether the body lands on the console's sign-in route, as its own page
/// names it in the React payload — escaped or not.
pub fn is_login_landing(body: &str) -> bool {
    find_sequence(body.as_bytes(), &LOGIN_SEQUENCE).is_some()
}

/// One piece of a payload sequence: literal text, or a quote that may carry
/// the backslash a JSON encoding would give it.
#[derive(Debug, Clone, Copy)]
enum Piece {
    Text(&'static str),
    Quote,
}

/// `"(auth)",{"children":["login"` — every quote behind an optional
/// backslash.
const LOGIN_SEQUENCE: [Piece; 13] = [
    Piece::Quote,
    Piece::Text("(auth)"),
    Piece::Quote,
    Piece::Text(","),
    Piece::Text("{"),
    Piece::Quote,
    Piece::Text("children"),
    Piece::Quote,
    Piece::Text(":"),
    Piece::Text("["),
    Piece::Quote,
    Piece::Text("login"),
    Piece::Quote,
];

/// Whether the pieces, in order, start at `at`, and where they end.
fn sequence_at(bytes: &[u8], at: usize, pieces: &[Piece]) -> Option<usize> {
    let mut cursor = at;
    for piece in pieces {
        match piece {
            Piece::Text(text) => {
                if !at_ci(bytes, cursor, text) {
                    return None;
                }
                cursor += text.len();
            }
            Piece::Quote => match bytes.get(cursor) {
                Some(b'"') => cursor += 1,
                Some(b'\\') if bytes.get(cursor + 1) == Some(&b'"') => cursor += 2,
                _ => return None,
            },
        }
    }
    Some(cursor)
}

fn find_sequence(bytes: &[u8], pieces: &[Piece]) -> Option<usize> {
    (0..bytes.len()).find(|at| sequence_at(bytes, *at, pieces).is_some())
}

/// The page's own script chunks: same origin, JavaScript, in order, each
/// once, and no more than sixty of them.
pub fn chunk_urls(page: &str) -> Vec<String> {
    let bytes = page.as_bytes();
    let mut urls: Vec<String> = Vec::new();
    let mut at = 0;
    while let Some(tag) = next_script_tag(bytes, at) {
        at = tag.start + 1;
        if urls.len() >= MAXIMUM_CHUNKS {
            break;
        }
        let Some(source) = script_src(page, &tag) else {
            continue;
        };
        let absolute = if source.starts_with('/') && !source.starts_with("//") {
            format!("{ORIGIN}{source}")
        } else {
            source
        };
        // Same origin only, and JavaScript — a chunk, not a page or a feed.
        if !absolute.starts_with(ORIGIN_ROOT) || !javascript(&absolute) {
            continue;
        }
        // A source the URL reader will not take is one the request could
        // not have carried either.
        if url::Url::parse(&absolute).is_err() || urls.contains(&absolute) {
            continue;
        }
        urls.push(absolute);
    }
    urls
}

/// `\.js($|\?)` — a name that ends in `.js`, or carries a query after it.
fn javascript(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut at = 0;
    while let Some(found) = find_ci(bytes, at, ".js") {
        match bytes.get(found + 3) {
            None | Some(b'?') => return true,
            _ => at = found + 1,
        }
    }
    false
}

/// An opening tag located in a page: where it starts, and where its
/// attributes sit — the latter ending at the first `>`.
#[derive(Debug, Clone)]
struct Tag {
    start: usize,
    attrs: std::ops::Range<usize>,
}

/// The next `<script …>` opening at or after `from`, letter case aside, the
/// tag name ending at a word boundary the way the page pattern reads it.
fn next_script_tag(bytes: &[u8], from: usize) -> Option<Tag> {
    let mut at = from;
    while let Some(start) = find_byte(bytes, at, b'<') {
        let name_at = start + 1;
        if at_ci(bytes, name_at, "script") {
            let after_name = name_at + "script".len();
            // A ninth letter would be another tag: `<scriptable`.
            let name_ended = match bytes.get(after_name) {
                None => true,
                Some(byte) => !(byte.is_ascii_alphanumeric() || *byte == b'_'),
            };
            if name_ended {
                let mut cursor = after_name;
                while cursor < bytes.len() && bytes[cursor] != b'>' {
                    cursor += 1;
                }
                if cursor < bytes.len() {
                    return Some(Tag {
                        start,
                        attrs: after_name..cursor,
                    });
                }
                return None;
            }
        }
        at = start + 1;
    }
    None
}

/// A script tag's `src`, as the pattern reads it: a word boundary before the
/// name, an `=`, either quote, and no quote inside.
fn script_src(page: &str, tag: &Tag) -> Option<String> {
    let bytes = page.as_bytes();
    let attrs = &bytes[tag.attrs.clone()];
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut at = 0;
    while let Some(found) = find_ci(attrs, at, "src") {
        // The word boundary before the name: a letter, digit or underscore
        // touching it makes it part of a longer word. `data-src` passes,
        // `dsrc` does not — and neither does a bare `scriptsrc`.
        let before = if found == 0 {
            tag.attrs
                .start
                .checked_sub(1)
                .and_then(|at| bytes.get(at))
                .copied()
        } else {
            attrs.get(found - 1).copied()
        };
        if before.is_some_and(is_word) || attrs.get(found + "src".len()) != Some(&b'=') {
            at = found + 1;
            continue;
        }
        let open = found + "src".len() + 1;
        if !matches!(attrs.get(open), Some(b'"') | Some(b'\'')) {
            at = found + 1;
            continue;
        }
        let mut cursor = open + 1;
        while cursor < attrs.len() && !matches!(attrs[cursor], b'"' | b'\'') {
            cursor += 1;
        }
        // The value needs room for itself and its closing quote, both ahead
        // of the `>` the attributes end at.
        if cursor == open + 1 || cursor >= attrs.len() {
            at = found + 1;
            continue;
        }
        return Some(
            std::str::from_utf8(&attrs[open + 1..cursor])
                .ok()?
                .to_string(),
        );
    }
    None
}

/// The id written just before `"getBillingOverviewResult"` in a chunk.
pub fn action_id(script: &str) -> Option<String> {
    let bytes = script.as_bytes();
    let mut at = 0;
    while let Some(open) = find_byte(bytes, at, b'"') {
        let mut cursor = open + 1;
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_hexdigit())
        {
            cursor += 1;
        }
        let length = cursor - (open + 1);
        // The quoted run of hex is the id; the name follows within a few
        // characters, none of them the parenthesis that would close the
        // call the two sit inside.
        if length >= 40 && bytes.get(cursor) == Some(&b'"') {
            let after = cursor + 1;
            let window = &bytes[after..(after + 150).min(bytes.len())];
            if let Some(found) = find_ci(window, 0, "\"getbillingoverviewresult\"") {
                if !window[..found].contains(&b')') {
                    return Some(script[open + 1..open + 1 + length].to_string());
                }
            }
        }
        at = open + 1;
    }
    None
}

// MARK: - Reading the reply

/// The balance on the reply's billing overview, with the plan it names. No
/// window: the balance is the one figure the action states.
pub fn reading(body: &str, account: AccountKey) -> ProviderUsage {
    if is_login_landing(body) {
        return ProviderUsage::unavailable(account, Unavailability::SessionExpired);
    }

    // Each line is `id:payload`; the result is the object with `ok`.
    let mut result: Option<serde_json::Map<String, serde_json::Value>> = None;
    for line in body.split(['\r', '\n']) {
        let Some(colon) = line.find(':') else {
            continue;
        };
        if colon == 0 {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&line[colon + 1..]) else {
            continue;
        };
        let Some(object) = payload.as_object() else {
            continue;
        };
        if object.contains_key("ok") {
            result = Some(object.clone());
            break;
        }
    }
    let Some(result) = result else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    if result.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return ProviderUsage::unavailable(account, Unavailability::ServerError);
    }

    let Some(billing) = result
        .get("data")
        .and_then(|data| data.get("billing"))
        .and_then(serde_json::Value::as_object)
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let Some(balance) = billing
        .get("balance")
        .and_then(serde_json::Value::as_f64)
        .filter(|balance| balance.is_finite())
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.plan = billing
        .get("plan")
        .and_then(serde_json::Value::as_str)
        .and_then(plan_name);
    usage.credit_balance = Some(money(balance));
    usage.credit_remaining = Some(CreditAmount {
        amount: balance,
        currency: "USD".into(),
    });
    usage
}

/// `free_plan` → "Free"; `pro-monthly` → "Pro Monthly".
pub fn plan_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed == "free_plan" {
        return Some("Free".to_string());
    }
    let words: Vec<String> = trimmed
        .split(|character| character == '_' || character == '-')
        .filter(|word| !word.is_empty())
        .map(titlecase)
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(words.join(" "))
}

/// The first letter up, the rest down — as the console's own pages write
/// their plan names.
fn titlecase(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// Dollars, as the console prices credit: two decimals.
pub fn money(amount: f64) -> String {
    format!("${amount:.2}")
}

// MARK: - Page-matching primitives

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn account() -> AccountKey {
        AccountKey::primary(Provider::TypeSafe)
    }

    const ACTION: &str = "a1b2c3d4e5a1b2c3d4e5a1b2c3d4e5a1b2c3d4e5";

    /// The billing page's scripts, as Next.js lists them — second-hand, not
    /// captured from a live console.
    fn billing_page() -> String {
        format!(
            "<html><body><script src=\"/_next/static/chunks/main-{ACTION}.js\" defer></script>\
             <script src=\"/_next/static/chunks/app/settings/billing/page.js\"></script>\
             <script src=\"/_next/static/chunks/shared.js?v=42\"></script></body></html>"
        )
    }

    #[test]
    fn the_chunks_are_same_origin_javascript_each_once() {
        let urls = chunk_urls(&billing_page());
        assert_eq!(urls.len(), 3);
        assert!(urls[0].starts_with(&format!("{ORIGIN}/_next/static/chunks/main-{ACTION}.js")));
        assert_eq!(
            urls[1],
            format!("{ORIGIN}/_next/static/chunks/app/settings/billing/page.js")
        );
        assert_eq!(
            urls[2],
            format!("{ORIGIN}/_next/static/chunks/shared.js?v=42")
        );

        // Another origin, a protocol-relative address and a feed that only
        // looks like JavaScript are all left out; so is a repeat.
        let page = concat!(
            "<script src=\"//cdn.typesafe.ai/x.js\"></script>",
            "<script src=\"https://other.example/x.js\"></script>",
            "<script src=\"/x.json\"></script>",
            "<script src=\"/y.js\"></script><script src=\"/y.js\"></script>",
            "<scriptable src=\"/z.js\"></scriptable>"
        );
        assert_eq!(chunk_urls(page), vec![format!("{ORIGIN}/y.js")]);
    }

    #[test]
    fn the_action_id_sits_beside_its_name_in_a_chunk() {
        let script = format!(
            "createServerReference(\"{ACTION}\",void 0,void 0,\"getBillingOverviewResult\")"
        );
        assert_eq!(action_id(&script).as_deref(), Some(ACTION));

        // The name read letter case aside, and the id found among others.
        let noisy = format!(
            "x=1;register(\"{ACTION}\",null,GET_SERVER_CALL(\"getbillingoverviewresult\"))"
        );
        assert_eq!(action_id(&noisy).as_deref(), Some(ACTION));

        // A parenthesis between the two, or a gap past a hundred and fifty
        // characters, is not the pairing.
        let separated = format!("\"{ACTION}\",(void 0),\"getBillingOverviewResult\"");
        assert_eq!(action_id(&separated), None);
        let far = format!(
            "\"{ACTION}\"{},\"getBillingOverviewResult\"",
            " ".repeat(151)
        );
        assert_eq!(action_id(&far), None);

        // Short or unquoted runs of hex are not ids.
        assert_eq!(action_id("\"abcdef\"\"getBillingOverviewResult\""), None);
        assert_eq!(
            action_id(&format!("{ACTION},\"getBillingOverviewResult\"")),
            None
        );
    }

    #[test]
    fn a_sign_in_landing_is_named_in_the_payload_escaped_or_not() {
        assert!(is_login_landing(
            r#"children:["(auth)",{"children":["login","$1"]}]"#
        ));
        assert!(is_login_landing(r#"\"(auth)\",{\"children\":[\"login\""#));
        assert!(!is_login_landing(
            r#"children:["(dashboard)",{"children":["login""#
        ));
        assert!(!is_login_landing("just a billing page"));
    }

    #[test]
    fn the_pasted_cookie_prefix_comes_off_and_the_rest_stays() {
        assert_eq!(
            cookie_header("Cookie: session=abc; other=1"),
            "session=abc; other=1"
        );
        assert_eq!(cookie_header("cookie:session=abc"), "session=abc");
        assert_eq!(cookie_header("  session=abc  "), "session=abc");
        // A header whose first word merely contains the name is left whole.
        assert_eq!(cookie_header("Cookiejar=1"), "Cookiejar=1");
    }

    #[test]
    fn the_reply_line_with_ok_carries_the_balance_and_the_plan() {
        let body = format!(
            "1:{{\"a\":1}}\n13:{{\"ok\":true,\"data\":{{\"billing\":{{\"balance\":12.5,\"plan\":\"pro-monthly\",\"grants\":[]}}}}}}\n2:{{\"done\":true}}"
        );
        let usage = reading(&body, account());
        assert!(matches!(usage.state, State::Live));
        assert!(usage.windows.is_empty());
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 12.5,
                currency: "USD".into()
            })
        );
        assert_eq!(usage.credit_balance.as_deref(), Some("$12.50"));
        assert_eq!(usage.plan.as_deref(), Some("Pro Monthly"));
    }

    #[test]
    fn a_failed_call_an_absent_result_and_a_stray_page_are_told_apart() {
        // ok, but not true, is the service saying no.
        let body = "13:{\"ok\":false,\"error\":\"boom\"}";
        assert_eq!(
            reading(body, account()).state,
            State::Unavailable(Unavailability::ServerError)
        );
        // No line with an `ok` at all: nothing readable.
        assert_eq!(
            reading("1:{\"a\":1}\nplain text", account()).state,
            State::Unavailable(Unavailability::UnreadableReply)
        );
        // An `ok` without billing figures behind it.
        assert_eq!(
            reading("13:{\"ok\":true}", account()).state,
            State::Unavailable(Unavailability::UnreadableReply)
        );
        // A balance that arrived as a word is no figure.
        assert_eq!(
            reading(
                "13:{\"ok\":true,\"data\":{\"billing\":{\"balance\":\"12.5\"}}}",
                account()
            )
            .state,
            State::Unavailable(Unavailability::UnreadableReply)
        );
        // A lapsed session lands on the sign-in page, redirected or not.
        assert_eq!(
            reading(r#"layout:"(auth)",{"children":["login"]}"#, account()).state,
            State::Unavailable(Unavailability::SessionExpired)
        );
    }

    #[test]
    fn the_plan_names_as_the_console_writes_them() {
        assert_eq!(plan_name("free_plan").as_deref(), Some("Free"));
        assert_eq!(plan_name("pro-monthly").as_deref(), Some("Pro Monthly"));
        assert_eq!(plan_name("TEAM_Yearly").as_deref(), Some("Team Yearly"));
        assert_eq!(plan_name("  pro  ").as_deref(), Some("Pro"));
        assert_eq!(plan_name("   "), None);
    }
}
