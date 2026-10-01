//! Mistral: the subscription's included allowances — API and Vibe — each as
//! the percentage Mistral reports, and the account's available credit.
//!
//! Read with the Mistral Admin browser session, from what Mistral's own
//! admin pages load: `GET https://admin.mistral.ai/api/billing/credits`, the
//! `https://admin.mistral.ai/subscription` page (whose server-rendered data
//! carries the allowances), and — only when that page has no Vibe allowance
//! — `console.mistral.ai`'s `billing.vibeUsage`. The shapes are second-hand —
//! taken from CodexBar's Mistral provider and its tests, not from captured
//! replies — and the fixtures in the tests say so.
//!
//! **What is not read.** CodexBar's headline figure is the month's spend,
//! which it works out from token counts and a price table of its own. That
//! is an estimate, and only what Mistral itself reports is shown.
//!
//! **The session cookie's name is not fixed.** Mistral signs in with Ory,
//! whose session cookie is `ory_session_` followed by the deployment's own
//! suffix. Read from the browser or pasted, only the `ory_session_…` and
//! `csrftoken` cookies are ever sent.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button. The trailing
/// `*` is the module's wildcard: the Ory cookie's name carries the
/// deployment's suffix, so no fixed list can name it.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["mistral.ai"],
    cookies: &["ory_session_*", "csrftoken"],
};

const CREDITS_ENDPOINT: &str = "https://admin.mistral.ai/api/billing/credits";
const SUBSCRIPTION_PAGE: &str = "https://admin.mistral.ai/subscription";
const VIBE_ENDPOINT: &str = concat!(
    "https://console.mistral.ai/api-ui/trpc/billing.vibeUsage",
    "?batch=1&input=%7B%220%22%3A%7B%22json%22%3Anull%2C%22meta%22%3A%7B",
    "%22values%22%3A%5B%22undefined%22%5D%2C%22v%22%3A1%7D%7D%7D",
);

pub struct MistralService {
    http: Arc<HttpClient>,
}

impl MistralService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

/// The session cookies a request may carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookies {
    /// `ory_session_…=…`, however many there are.
    pub session: Vec<String>,
    pub csrf: Option<String>,
}

impl Cookies {
    pub fn header(&self) -> String {
        let mut parts = self.session.clone();
        if let Some(csrf) = &self.csrf {
            parts.push(format!("csrftoken={csrf}"));
        }
        parts.join("; ")
    }
}

/// Only the Ory session and the CSRF token out of a pasted `Cookie:` header
/// (with or without the `Cookie:` in front). Nothing else leaves. No session
/// cookie at all is no session.
pub fn session_cookies(header: &str) -> Option<Cookies> {
    let trimmed = header.trim();
    // With or without the `Cookie:` in front, in any case.
    let text = if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("cookie:") {
        &trimmed[7..]
    } else {
        trimmed
    };

    let mut session: Vec<String> = Vec::new();
    let mut csrf: Option<String> = None;
    for part in text.split(';') {
        let pair = part.trim();
        let Some(equals) = pair.find('=') else {
            continue;
        };
        let (name, value) = (&pair[..equals], &pair[equals + 1..]);
        if value.is_empty() || value.contains([',', '\r', '\n']) {
            continue;
        }
        if name.starts_with("ory_session_") && name.len() > "ory_session_".len() {
            session.push(format!("{name}={value}"));
        } else if name == "csrftoken" && csrf.is_none() {
            csrf = Some(value.to_string());
        }
    }
    if session.is_empty() {
        None
    } else {
        Some(Cookies { session, csrf })
    }
}

impl ProviderService for MistralService {
    fn provider(&self) -> Provider {
        Provider::Mistral
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Mistral);
        let Some(header) = pasted_or_none(keys.api_key(Provider::Mistral)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let Some(cookies) = session_cookies(&header) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        // Credit first: it is the one JSON route, so it is where a session
        // that no longer works is found out.
        let credit = match self.admin_json(CREDITS_ENDPOINT, &cookies, "application/json") {
            Ok(reply) => credit(&reply),
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };

        // The allowances are best-effort, as they are in Mistral's own pages:
        // an account without a subscription has none.
        let mut allowances = Allowances::default();
        if let Ok((status, page)) = self.get(SUBSCRIPTION_PAGE, &cookies, "text/html", false) {
            if (200..300).contains(&status) {
                allowances = allowances_from_page(&page);
            }
        }
        if allowances.vibe.is_none() && cookies.csrf.is_some() {
            if let Ok(reply) = self.admin_json(VIBE_ENDPOINT, &cookies, "*/*") {
                allowances.vibe = vibe(&reply);
            }
        }

        let usage = reading(allowances, credit, account);
        if matches!(usage.state, crate::model::State::Live) {
            return ProviderUsage {
                origin: Some(self.origin_token().into()),
                ..usage
            };
        }
        usage
    }
}

impl MistralService {
    /// One admin-host GET, its status read by hand: the shared fetch folds
    /// 401/403 and every redirect into the refused key, and for a browser
    /// session those all mean the session is no good any more.
    fn admin_json(
        &self,
        url: &str,
        cookies: &Cookies,
        accept: &str,
    ) -> Result<Value, Unavailability> {
        let (status, body) = self.get(url, cookies, accept, true)?;
        match status {
            401 | 403 | 300..=399 => Err(Unavailability::SessionExpired),
            429 => Err(Unavailability::RateLimited),
            status if (200..300).contains(&status) => {
                serde_json::from_str(&body).map_err(|_| Unavailability::UnreadableReply)
            }
            _ => Err(Unavailability::ServerError),
        }
    }

    fn get(
        &self,
        url: &str,
        cookies: &Cookies,
        accept: &str,
        admin_origin: bool,
    ) -> Result<(u16, String), Unavailability> {
        let mut request = self
            .http
            .client_for_login()
            .get(url)
            .header("Cookie", cookies.header())
            .header("Accept", accept)
            .timeout(std::time::Duration::from_secs(15));
        if admin_origin {
            request = request
                .header("Origin", "https://admin.mistral.ai")
                .header("Referer", "https://admin.mistral.ai/organization/billing");
        }
        if let Some(csrf) = &cookies.csrf {
            // The admin console checks the token under its own spelling, the
            // console API under Django's.
            request = if admin_origin {
                request.header("X-CSRFTOKEN", csrf)
            } else {
                request.header("X-CSRFToken", csrf)
            };
        }
        let response = request.send().map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .map_err(|_| Unavailability::UnreadableReply)?;
        Ok((status, text))
    }
}

// MARK: - Reading the replies

#[derive(Deserialize, Default)]
#[serde(default)]
struct CreditReply {
    wallet_amount: Option<f64>,
    credit_notes_amount: Option<f64>,
    ongoing_usage_balance: Option<f64>,
    currency: Option<String>,
}

/// What is available to spend: the wallet and any credit notes, less the
/// usage already run up against them and not yet settled — all three as
/// Mistral reports them. Left off if the wallet or the currency is missing,
/// or if the result is below zero, which is a debt and not a balance.
pub fn credit(reply: &Value) -> Option<CreditAmount> {
    let parsed: CreditReply = serde_json::from_value(reply.clone()).ok()?;
    let wallet = parsed.wallet_amount?;
    let currency = parsed.currency?.trim().to_uppercase();
    if currency.len() != 3 {
        return None;
    }
    let available = wallet + parsed.credit_notes_amount.unwrap_or(0.0)
        - parsed.ongoing_usage_balance.unwrap_or(0.0);
    if !available.is_finite() || available < 0.0 {
        return None;
    }
    Some(CreditAmount {
        amount: available,
        currency,
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Allowance {
    pub percent_used: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Allowances {
    pub api: Option<Allowance>,
    pub vibe: Option<Allowance>,
}

/// The subscription page is a Next.js page: its data arrives as strings
/// pushed onto `self.__next_f`, which together make one stream. The
/// allowances are `api_budget` and `vibe_budget` objects inside it. Where
/// the stream carries two different values under one name, neither is
/// taken — there is no telling which is this account's.
pub fn allowances_from_page(html: &str) -> Allowances {
    let stream = flight_stream(html);
    Allowances {
        api: budget("api_budget", &stream),
        vibe: budget("vibe_budget", &stream),
    }
}

/// The page's pieces, joined back into the one string they make up.
fn flight_stream(html: &str) -> String {
    const MARKER: &str = "self.__next_f.push(";
    let mut chunks: Vec<String> = Vec::new();
    let mut cursor = 0;
    while let Some(found) = find_from(html, MARKER, cursor) {
        cursor = found + MARKER.len();
        let tail = &html[cursor..];
        let start = cursor + (tail.len() - tail.trim_start().len());
        if !html[start..].starts_with('[') {
            continue;
        }
        let Some(end) = container_end(html, start) else {
            continue;
        };
        if let Ok(array) = serde_json::from_str::<Value>(&html[start..end]) {
            if let Some(items) = array.as_array() {
                // `[1, "<chunk>"]` is a data piece; anything else is not.
                if items.first().and_then(|v| v.as_i64()) == Some(1) {
                    if let Some(chunk) = items.get(1).and_then(|v| v.as_str()) {
                        chunks.push(chunk.to_string());
                    }
                }
            }
        }
        cursor = end;
    }
    chunks.concat()
}

/// Where the JSON array or object opening at `start` closes, skipping
/// whatever is inside strings. None if it never does, or closes wrongly.
pub fn container_end(text: &str, from: usize) -> Option<usize> {
    let mut closers: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for (offset, character) in text[from..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
        } else {
            match character {
                '"' => in_string = true,
                '[' => closers.push(']'),
                '{' => closers.push('}'),
                ']' | '}' => {
                    if closers.last() != Some(&character) {
                        return None;
                    }
                    closers.pop();
                    if closers.is_empty() {
                        return Some(from + offset + character.len_utf8());
                    }
                }
                _ => {}
            }
        }
    }
    None
}

fn find_from(text: &str, needle: &str, from: usize) -> Option<usize> {
    if from > text.len() {
        return None;
    }
    text[from..].find(needle).map(|at| at + from)
}

/// One allowance under `name`, taken only when the stream carries exactly
/// one distinct one.
fn budget(name: &str, stream: &str) -> Option<Allowance> {
    let needle = format!("\"{name}\":");
    let mut seen: Vec<String> = Vec::new();
    let mut budgets: Vec<Allowance> = Vec::new();
    let mut cursor = 0;
    while let Some(at) = find_from(stream, &needle, cursor) {
        cursor = at + needle.len();
        let tail = &stream[cursor..];
        let start = cursor + (tail.len() - tail.trim_start().len());
        if !stream[start..].starts_with('{') {
            continue;
        }
        let Some(end) = container_end(stream, start) else {
            continue;
        };
        let object = stream[start..end].to_string();
        cursor = end;
        if seen.contains(&object) {
            continue;
        }
        seen.push(object.clone());
        if let Some(budget) = allowance_from_budget(&object) {
            budgets.push(budget);
        }
    }
    if budgets.len() == 1 {
        budgets.pop()
    } else {
        None
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BudgetReply {
    usage_percentage: Option<f64>,
    reset_at: Option<String>,
}

/// One allowance, by its reported percentage. A server-rendered date may
/// arrive as React's `$D…` form, which is the same stamp behind a tag.
fn allowance_from_budget(text: &str) -> Option<Allowance> {
    let reply: BudgetReply = serde_json::from_str(text).ok()?;
    allowance(&reply)
}

fn allowance(reply: &BudgetReply) -> Option<Allowance> {
    let percent = reply
        .usage_percentage
        .filter(|p| p.is_finite() && *p >= 0.0)?;
    let resets_at = reply
        .reset_at
        .as_deref()
        .map(|stamp| match stamp.strip_prefix("$D") {
            Some(plain) => plain,
            None => stamp,
        })
        .and_then(|stamp| crate::timeutil::parse_iso8601_ms(stamp));
    Some(Allowance {
        percent_used: percent,
        resets_at,
    })
}

#[derive(Deserialize)]
struct VibeReply {
    #[serde(default)]
    result: Option<VibeResult>,
}

#[derive(Deserialize)]
struct VibeResult {
    #[serde(default)]
    data: Option<VibePayload>,
}

#[derive(Deserialize)]
struct VibePayload {
    #[serde(default)]
    json: Option<BudgetReply>,
}

/// The console's Vibe figure, used only when the subscription page has
/// none: `[{ result: { data: { json: { usage_percentage, reset_at } } } }]`.
pub fn vibe(reply: &Value) -> Option<Allowance> {
    let replies: Vec<VibeReply> = serde_json::from_value(reply.clone()).ok()?;
    let first = replies.first()?;
    let json = first.result.as_ref()?.data.as_ref()?.json.as_ref()?;
    allowance(json)
}

/// Mistral bills a subscription by the month, and the allowances reset with
/// it; the month's length is not a stated one, so it only sorts.
pub fn reading(
    allowances: Allowances,
    credit: Option<CreditAmount>,
    account: AccountKey,
) -> ProviderUsage {
    let windows: Vec<UsageWindow> = [
        ("api", "API", allowances.api),
        ("vibe", "Vibe", allowances.vibe),
    ]
    .iter()
    .filter_map(|(id, scope, allowance)| {
        let allowance = (*allowance)?;
        let mut window = UsageWindow::new(
            &format!("mistral.{id}"),
            Kind::Monthly,
            Some(scope.to_string()),
            allowance.percent_used / 100.0,
            30 * 86_400,
            allowance.resets_at,
        );
        window.reports_length = false;
        window.is_exhausted = allowance.percent_used >= 100.0;
        Some(window)
    })
    .collect();

    if windows.is_empty() && credit.is_none() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.credit_balance = credit.as_ref().map(money);
    usage.credit_remaining = credit;
    usage
}

pub fn money(credit: &CreditAmount) -> String {
    match credit.currency.as_str() {
        "USD" => format!("${:.2}", credit.amount),
        "EUR" => format!("€{:.2}", credit.amount),
        other => format!("{:.2} {other}", credit.amount),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    /// A Next.js page whose server-rendered data is `stream`, pushed in
    /// pieces the way the page pushes it.
    fn page(stream: &str, pieces: usize) -> String {
        let size = (stream.len() / pieces).max(1);
        let mut scripts = Vec::new();
        let mut rest = stream;
        while !rest.is_empty() {
            let cut = size.min(rest.len());
            let piece = &rest[..cut];
            rest = &rest[cut..];
            let pushed = json!([1, piece]).to_string();
            scripts.push(format!("<script>self.__next_f.push({pushed})</script>"));
        }
        format!(
            "<html><body><script>self.__next_f.push([0])</script>{}</body></html>",
            scripts.concat()
        )
    }

    const BUDGETS: &str = concat!(
        "5:[\"$\",\"div\",null,{\"budget\":{\"api_budget\":{\"usage_percentage\":42.5,",
        "\"initial_budget\":10,\"currency\":\"EUR\",\"reset_at\":\"$D2026-10-01T00:00:00.000Z\"},",
        "\"vibe_budget\":{\"usage_percentage\":12,\"initial_budget\":20,\"currency\":\"EUR\",",
        "\"reset_at\":\"2026-10-01T00:00:00Z\"}}}]\n\n"
    );

    #[test]
    fn both_allowances_are_read_from_the_page_as_the_percentages_mistral_reports() {
        let found = allowances_from_page(&page(BUDGETS, 3));
        assert_eq!(
            found.api,
            Some(Allowance {
                percent_used: 42.5,
                resets_at: crate::timeutil::parse_iso8601_ms("2026-10-01T00:00:00Z"),
            })
        );
        assert_eq!(
            found.vibe,
            Some(Allowance {
                percent_used: 12.0,
                resets_at: crate::timeutil::parse_iso8601_ms("2026-10-01T00:00:00Z"),
            })
        );
    }

    #[test]
    fn a_page_with_no_allowance_or_two_that_disagree_gives_none() {
        assert_eq!(
            allowances_from_page(&page("1:[\"$\",\"p\",null,{\"children\":\"Free\"}]", 2)),
            Allowances::default()
        );
        assert_eq!(
            allowances_from_page("<html>signed out</html>"),
            Allowances::default()
        );

        let twice = "1:{\"api_budget\":{\"usage_percentage\":10}}\n2:{\"api_budget\":{\"usage_percentage\":90}}";
        assert_eq!(allowances_from_page(&page(twice, 2)).api, None);
        // The same one twice is still one.
        let same = "1:{\"api_budget\":{\"usage_percentage\":10}}\n2:{\"api_budget\":{\"usage_percentage\":10}}";
        assert_eq!(
            allowances_from_page(&page(same, 2))
                .api
                .map(|a| a.percent_used),
            Some(10.0)
        );
        // A figure that isn't one is left off.
        assert_eq!(
            allowances_from_page(&page("1:{\"vibe_budget\":{\"usage_percentage\":-4}}", 1)).vibe,
            None
        );
    }

    /// Upstream's `mistral-vibe-usage` fixture — second-hand, written from
    /// CodexBar's Mistral provider, not captured from a live account.
    #[test]
    fn the_consoles_vibe_figure_when_the_page_has_none() {
        let fixture = json!([{"result": {"data": {"json": {
            "usage_percentage": 37, "quota_changed_this_month": false,
            "payg_enabled": false, "reset_at": "2026-07-01T00:00:00Z"}}}}]);
        let found = vibe(&fixture);
        assert_eq!(
            found,
            Some(Allowance {
                percent_used: 37.0,
                resets_at: crate::timeutil::parse_iso8601_ms("2026-07-01T00:00:00Z"),
            })
        );
        assert_eq!(vibe(&json!([])), None);
        assert_eq!(vibe(&json!([{"result": {"data": {"json": null}}}]),), None);
    }

    /// Upstream's `mistral-credits` fixture.
    #[test]
    fn available_credit_is_the_wallet_and_credit_notes_less_usage_not_yet_settled() {
        let fixture = json!({
            "wallet_amount": 12.5, "credit_notes_amount": 2.25,
            "ongoing_usage_balance": 1.5, "currency": "USD",
            "minimum_credits_purchase": 10, "maximum_credits_purchase": 1000
        });
        assert_eq!(
            credit(&fixture),
            Some(CreditAmount {
                amount: 13.25,
                currency: "USD".into()
            })
        );

        // Below zero is a debt, not a balance; no currency is no money.
        assert_eq!(
            credit(&json!({"wallet_amount": 1, "ongoing_usage_balance": 3, "currency": "EUR"})),
            None
        );
        assert_eq!(credit(&json!({"wallet_amount": 5})), None);
        assert_eq!(credit(&json!("not the reply")), None);
    }

    #[test]
    fn the_reading_two_monthly_allowances_scoped_by_product_and_the_balance() {
        let allowances = allowances_from_page(&page(BUDGETS, 2));
        let credit = credit(&json!({
            "wallet_amount": 12.5, "credit_notes_amount": 2.25,
            "ongoing_usage_balance": 1.5, "currency": "USD"
        }));
        let usage = reading(allowances, credit, AccountKey::primary(Provider::Mistral));

        assert!(matches!(usage.state, State::Live));
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|w| w.scope.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("API"), Some("Vibe")]
        );
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|w| w.used_fraction)
                .collect::<Vec<_>>(),
            vec![0.425, 0.12]
        );
        assert!(usage
            .windows
            .iter()
            .all(|w| w.kind == Kind::Monthly && !w.reports_length));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 13.25,
                currency: "USD".into()
            })
        );
        assert!(usage
            .credit_balance
            .as_deref()
            .is_some_and(|text| text.contains("13.25")));
    }

    #[test]
    fn nothing_reported_at_all_is_no_limits() {
        let usage = reading(
            Allowances::default(),
            None,
            AccountKey::primary(Provider::Mistral),
        );
        assert_eq!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn only_the_ory_session_and_the_csrf_token_are_kept_from_a_pasted_header() {
        let cookies = session_cookies(
            "Cookie: _ga=GA1; ory_session_coolstack=abc; csrftoken=tok; ajs_user_id=me; ory_session_=empty",
        )
        .expect("a session is there");
        assert_eq!(cookies.session, vec!["ory_session_coolstack=abc"]);
        assert_eq!(cookies.csrf.as_deref(), Some("tok"));
        assert_eq!(cookies.header(), "ory_session_coolstack=abc; csrftoken=tok");

        assert_eq!(session_cookies("csrftoken=tok; _ga=GA1"), None);
        assert_eq!(session_cookies("ory_session_x="), None);
    }

    #[test]
    fn a_container_that_never_closes_or_closes_wrongly_has_no_end() {
        let text = "{\"a\": [1, {\"b\": \"}]\"}]}, after";
        // The root object closes where ", after" begins: the brackets inside
        // the string never counted.
        assert_eq!(container_end(text, 0), Some(text.find(", after").unwrap()));
        assert_eq!(container_end("[{\"unterminated\": ", 0), None);
        assert_eq!(container_end("]wrong", 0), None);
    }
}
