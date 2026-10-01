//! Replicate: the prepaid credit left on the account, read with the
//! signed-in replicate.com session.
//!
//! Replicate's API token runs models; the billing figures are only on the
//! website, behind its session. So the credential is a browser session,
//! imported on request and kept to `sessionid` (the one that has to be
//! there) and `csrftoken`.
//!
//! Two requests, both to replicate.com:
//!
//! 1. `GET /account/billing` as a page. Its server-rendered React props name
//!    the account the page is for — a user or an organization — and that is
//!    the only way to know whose credit to ask for.
//! 2. `GET /api/{users|organizations}/{name}/unused-credit`, the page's own
//!    call for the balance.
//!
//! The shapes are second-hand — taken from CodexBar's Replicate plugin and
//! its tests, not from captured replies — and the fixtures in the tests say
//! so.
//!
//! **What is left out, and why.** The month's spend (from the invoices the
//! page lists) has no limit to measure it against and nowhere to go yet, so
//! it is not asked for. Replicate reports no allowance and no percentage,
//! and none is drawn: the balance is the reading.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, Unavailability};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// Where the session lives, for the Settings import button.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["replicate.com"],
    cookies: &["sessionid", "csrftoken"],
};

const BILLING_PAGE: &str = "https://replicate.com/account/billing";

pub struct ReplicateService {
    http: Arc<HttpClient>,
}

impl ReplicateService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// One request, its status read by hand: a redirect or a refusal is the
    /// session no longer working. `fetch_json` folds those into the refused
    /// key, which is the wrong story for a browser session.
    fn get(&self, url: &str, cookie: &str, accept: &str) -> Result<(u16, String), Unavailability> {
        let response = self
            .http
            .client_for_login()
            .get(url)
            .header("Cookie", cookie)
            .header("Accept", accept)
            .timeout(Duration::from_secs(15))
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        let text = match response.text() {
            Ok(text) => text,
            Err(_) => return Err(Unavailability::UnreadableReply),
        };
        Ok((status, text))
    }
}

impl ProviderService for ReplicateService {
    fn provider(&self) -> Provider {
        Provider::Replicate
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Replicate);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::Replicate)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        let (status, page) = match self.get(BILLING_PAGE, &cookie, "text/html") {
            Ok((status, page)) => (status, page),
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        if !is_success(status) {
            return ProviderUsage::unavailable(account, refused_or_error(status));
        }

        let owner = match account_in_billing_page(&page) {
            PageAccount::Found(found) => found,
            // Replicate's public sign-in page, which a lapsed session is
            // sent to.
            PageAccount::SignedOut => {
                return ProviderUsage::unavailable(account, Unavailability::SessionExpired)
            }
            PageAccount::Unrecognized => {
                return ProviderUsage::unavailable(account, Unavailability::UnreadableReply)
            }
        };

        let (status, body) = match self.get(&owner.credit_url(), &cookie, "application/json") {
            Ok((status, body)) => (status, body),
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        if !is_success(status) {
            return ProviderUsage::unavailable(account, refused_or_error(status));
        }
        let reply: Value = match serde_json::from_str(&body) {
            Ok(reply) => reply,
            Err(_) => return ProviderUsage::unavailable(account, Unavailability::UnreadableReply),
        };
        let usage = reading(&reply, account);
        if matches!(usage.state, crate::model::State::Live) {
            return ProviderUsage {
                origin: Some(self.origin_token().into()),
                ..usage
            };
        }
        usage
    }
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// A status apart from success. For this credential the refusals all mean the
/// session is no good any more.
fn refused_or_error(status: u16) -> Unavailability {
    match status {
        401 | 403 | 300..=399 => Unavailability::SessionExpired,
        429 => Unavailability::RateLimited,
        _ => Unavailability::ServerError,
    }
}

/// The account named in the page's `react-component-props` JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub is_organization: bool,
    pub username: String,
}

impl Account {
    pub fn credit_url(&self) -> String {
        let kind = if self.is_organization {
            "organizations"
        } else {
            "users"
        };
        format!(
            "https://replicate.com/api/{kind}/{}/unused-credit",
            percent_encode(&self.username)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageAccount {
    Found(Account),
    /// Replicate's public sign-in page, which a lapsed session is sent to.
    SignedOut,
    Unrecognized,
}

/// The account, found wherever it is nested in the props JSON. Bounded,
/// because the page is not ours.
pub fn account_in_billing_page(html: &str) -> PageAccount {
    let mut cursor = 0;
    while let Some(start) = find_from(html, "<script", cursor) {
        let attributes_from = start + "<script".len();
        let Some(close) = find_from(html, ">", attributes_from) else {
            break;
        };
        let attributes = &html[attributes_from..close];
        let content_from = close + 1;
        let content_end = find_from(html, "</script", content_from).unwrap_or(html.len());
        cursor = content_end;
        // Props in any other script are not trusted: both the React props id
        // and the JSON type have to name the block.
        if !attribute_is(attributes, "id", |value| {
            value.starts_with("react-component-props")
        }) || !attribute_is(attributes, "type", |value| value == "application/json")
        {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<Value>(html[content_from..content_end].trim())
        else {
            continue;
        };
        if let Some(found) = search_account(&payload) {
            return PageAccount::Found(found);
        }
    }

    // Both markers of Replicate's own sign-in page, not just the word.
    let sign_in = is_replicate_sign_in_title(html);
    let github = has_github_login_link(html);
    if sign_in && github {
        PageAccount::SignedOut
    } else {
        PageAccount::Unrecognized
    }
}

/// `<title>…Sign in | Replicate…</title>`, whitespace around the parts
/// ignored, case ignored.
fn is_replicate_sign_in_title(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let Some(start) = find_from(&lower, "<title>", 0).map(|at| at + "<title>".len()) else {
        return false;
    };
    let Some(end) = find_from(&lower, "</title>", start) else {
        return false;
    };
    let mut parts = lower[start..end].split('|');
    match (parts.next(), parts.next()) {
        (Some(before), Some(after)) => before.trim() == "sign in" && after.trim() == "replicate",
        _ => false,
    }
}

/// The first object, the outer one before what it holds, that carries an
/// `account` naming a user or an organization. Only containers are walked,
/// and only a bounded number of them.
fn search_account(payload: &Value) -> Option<Account> {
    let mut queue: Vec<&Value> = vec![payload];
    let mut visited = 0;
    while visited < 4_000 {
        let Some(item) = queue.get(visited).copied() else {
            return None;
        };
        visited += 1;
        if let Some(object) = item.as_object() {
            if let Some(found) = object.get("account").and_then(account_of) {
                return Some(found);
            }
            queue.extend(
                object
                    .values()
                    .filter(|value| value.is_object() || value.is_array()),
            );
        } else if let Some(items) = item.as_array() {
            queue.extend(
                items
                    .iter()
                    .filter(|value| value.is_object() || value.is_array()),
            );
        }
    }
    None
}

fn account_of(candidate: &Value) -> Option<Account> {
    let object = candidate.as_object()?;
    let kind = object.get("kind")?.as_str()?;
    if kind != "user" && kind != "organization" {
        return None;
    }
    let username = object.get("username")?.as_str()?.trim();
    if username.is_empty() {
        return None;
    }
    Some(Account {
        is_organization: kind == "organization",
        username: username.to_string(),
    })
}

/// Whether the attribute `name` is present with a value `check` accepts.
/// Only the quoted form counts — the shapes the page actually writes.
fn attribute_is(attributes: &str, name: &str, check: impl Fn(&str) -> bool) -> bool {
    let mut at = 0;
    while let Some(found) = find_word_from(attributes, name, at) {
        at = found + name.len();
        let mut rest = &attributes[at..];
        rest = rest.trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        rest = rest[1..].trim_start();
        let quote = match rest.as_bytes().first() {
            Some(&q @ (b'"' | b'\'')) => q as char,
            _ => continue,
        };
        let Some(end) = rest[1..].find(quote) else {
            continue;
        };
        if check(&rest[1..1 + end]) {
            return true;
        }
    }
    false
}

/// An occurrence of `word` at a word boundary, so `id` does not match inside
/// `hidden`.
fn find_word_from(text: &str, word: &str, from: usize) -> Option<usize> {
    if word.is_empty() || text.len() < word.len() {
        return None;
    }
    let bytes = text.as_bytes();
    let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut at = from;
    while at + word.len() <= text.len() {
        let found = text[at..].find(word)? + at;
        let before_ok = found == 0 || !is_word(bytes[found - 1]);
        let after = found + word.len();
        let after_ok = after == text.len() || !is_word(bytes[after]);
        if before_ok && after_ok {
            return Some(found);
        }
        at = found + 1;
    }
    None
}

fn find_from(text: &str, needle: &str, from: usize) -> Option<usize> {
    if from > text.len() {
        return None;
    }
    text[from..].find(needle).map(|at| at + from)
}

/// The one form of the GitHub sign-in link the sign-in page carries:
/// `href="/login/github/"`, with or without a query.
fn has_github_login_link(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(found) = find_from(&lower, "href", at) {
        at = found + "href".len();
        let rest = lower[at..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let rest = rest[1..].trim_start();
        let Some(quote) = rest.as_bytes().first().copied() else {
            continue;
        };
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        let quote = quote as char;
        let Some(end) = rest[1..].find(quote) else {
            continue;
        };
        let value = &rest[1..1 + end];
        if value == "/login/github/" || value.starts_with("/login/github/?") {
            return true;
        }
    }
    false
}

/// Everything but the unreserved characters escaped, so a space or a slash in
/// a name arrives as itself and never as a path.
fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Replicate bills in dollars; the reply carries the figure only.
pub fn money(amount: f64) -> String {
    format!("${amount:.2}")
}

pub fn reading(reply: &Value, account: AccountKey) -> ProviderUsage {
    let Some(root) = reply.as_object() else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    let credit: f64 = match root.get("unused_credit") {
        None | Some(Value::Null) => {
            // No balance reported is nothing to show, not a zero.
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        }
        Some(Value::String(text)) => {
            // Whole dollars, or dollars and cents, after trimming — a
            // minus sign, a word, or an empty string is not a balance.
            let trimmed = text.trim();
            let valid = !trimmed.is_empty()
                && matches!(trimmed.as_bytes()[0], b'0'..=b'9')
                && trimmed.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                && trimmed.matches('.').count() <= 1
                && !trimmed.ends_with('.');
            match valid.then(|| trimmed.parse::<f64>().ok()).flatten() {
                Some(value) if value.is_finite() => value,
                _ => return ProviderUsage::unavailable(account, Unavailability::UnreadableReply),
            }
        }
        Some(Value::Number(value)) => match value.as_f64() {
            Some(credit) if credit.is_finite() && credit >= 0.0 => credit,
            _ => {
                return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
            }
        },
        // A boolean, an object, an array: not a balance, and never zero.
        Some(_) => {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        }
    };

    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.credit_balance = Some(money(credit));
    usage.credit_remaining = Some(CreditAmount {
        amount: credit,
        currency: "USD".into(),
    });
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn reading_for(value: Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::Replicate))
    }

    /// Upstream's `replicate-unused-credit` fixture — second-hand, written
    /// from CodexBar's Replicate plugin, not captured from a live account.
    #[test]
    fn the_balance_is_read_as_reported_in_dollars_with_no_ring() {
        let usage = reading_for(json!({"unused_credit": "80.00"}));
        assert!(matches!(usage.state, State::Live));
        assert!(usage.windows.is_empty());
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 80.0,
                currency: "USD".into()
            })
        );
        assert_eq!(usage.credit_balance.as_deref(), Some("$80.00"));
    }

    const USER_PAGE: &str = "<html><head><title>Billing | Replicate</title></head><body>\n\
        <script type=\"application/json\" id=\"react-component-props-billing-page\">\n\
        {\"page\":{\"account\":{\"kind\":\"user\",\"username\":\"demo-user\"}}}\n\
        </script></body></html>";

    #[test]
    fn the_billing_page_names_a_users_account() {
        assert_eq!(
            account_in_billing_page(USER_PAGE),
            PageAccount::Found(Account {
                is_organization: false,
                username: "demo-user".into()
            })
        );
        let PageAccount::Found(found) = account_in_billing_page(USER_PAGE) else {
            panic!("expected the account");
        };
        assert_eq!(
            found.credit_url(),
            "https://replicate.com/api/users/demo-user/unused-credit"
        );
    }

    #[test]
    fn an_organization_nested_deeper_in_other_props_is_found_and_asked_for_as_one() {
        let page = "<script id='react-component-props-layout' type='application/json'>\n\
            {\"layout\":{\"nested\":[{\"x\":1},{\"account\":{\"kind\":\"organization\",\"username\":\"demo org\"}}]}}\n\
            </script>";
        let PageAccount::Found(found) = account_in_billing_page(page) else {
            panic!("the organization should be found");
        };
        assert!(found.is_organization);
        assert_eq!(
            found.credit_url(),
            "https://replicate.com/api/organizations/demo%20org/unused-credit"
        );
    }

    #[test]
    fn props_in_any_other_script_are_not_trusted() {
        let page = "<script type=\"application/json\" id=\"analytics\">\
            {\"account\":{\"kind\":\"user\",\"username\":\"someone\"}}</script>\n\
            <script id=\"react-component-props-x\">{\"account\":{\"kind\":\"user\",\"username\":\"no-type\"}}</script>";
        assert_eq!(account_in_billing_page(page), PageAccount::Unrecognized);
    }

    #[test]
    fn replicates_sign_in_page_is_a_lapsed_session_not_a_changed_page() {
        let page = "<html><head><title>Sign in | Replicate</title></head>\n\
            <body><a class=\"btn\" href=\"/login/github/?next=/account/billing\">Sign in with GitHub</a></body></html>";
        assert_eq!(account_in_billing_page(page), PageAccount::SignedOut);
        assert_eq!(
            account_in_billing_page("<title>Sign in | Replicate</title>"),
            PageAccount::Unrecognized
        );
    }

    #[test]
    fn a_balance_that_isnt_one_cant_be_read() {
        for value in [
            json!({"unused_credit": "NaN"}),
            json!({"unused_credit": "-1"}),
            json!({"unused_credit": ""}),
            json!({"unused_credit": true}),
            json!({"unused_credit": -3}),
            json!([1, 2, 3]),
            Value::Null,
        ] {
            assert_eq!(
                reading_for(value).state,
                State::Unavailable(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn no_balance_reported_is_nothing_to_show_not_a_zero() {
        assert_eq!(
            reading_for(json!({})).state,
            State::Unavailable(Unavailability::NoLimitsReported)
        );
        assert_eq!(
            reading_for(json!({"unused_credit": Value::Null})).state,
            State::Unavailable(Unavailability::NoLimitsReported)
        );
        assert_eq!(
            reading_for(json!({"unused_credit": 12.5}))
                .credit_remaining
                .map(|c| c.amount),
            Some(12.5)
        );
    }

    #[test]
    fn the_balance_is_trimmed_before_it_reads() {
        // Whitespace around the figure is trimmed away first.
        assert_eq!(
            reading_for(json!({"unused_credit": " 12 "}))
                .credit_remaining
                .map(|c| c.amount),
            Some(12.0)
        );
        // A figure with two decimal points is not one.
        assert_eq!(
            reading_for(json!({"unused_credit": "1.2.3"})).state,
            State::Unavailable(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn word_boundaries_keep_other_attributes_from_reading_as_id() {
        assert!(!attribute_is(
            "hidden id-x=\"react-component-props\"",
            "id",
            |v| v.starts_with("react-component-props")
        ));
        assert!(attribute_is(
            "data-x=\"1\" id = 'react-component-props-y'",
            "id",
            |v| v.starts_with("react-component-props")
        ));
        assert!(has_github_login_link("<a href='/login/github/'>x</a>"));
        assert!(!has_github_login_link(
            "<a href=\"/login/github-other/\">x</a>"
        ));
        assert_eq!(percent_encode("demo-user"), "demo-user");
        assert_eq!(percent_encode("a/b"), "a%2Fb");
    }
}
