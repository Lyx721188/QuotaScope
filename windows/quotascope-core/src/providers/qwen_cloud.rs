//! Qwen Cloud's individual Token Plan: a five-hour, a weekly and a monthly
//! allowance, each reported by the service as the share of it already used.
//!
//! Read with a browser session from `qwencloud.com`, the way the console's
//! own subscription page reads it: the page's security token first (from the
//! page, a cookie, or the account endpoint), then
//! `POST https://cs-data.qwencloud.com/data/api.json` for the plan's usage
//! and, for its tier's name only, its subscription. The shape is second-hand
//! — taken from CodexBar's Qwen Cloud provider and its tests, not from a
//! captured reply — and the fixtures in the tests say so.
//!
//! **What is left out.** The plan's credit totals per window (the
//! `quota-config` route) are not asked for: there is nowhere to show them,
//! and the share used is already the service's own figure. A reply in the
//! older subscription-summary shape is not read for figures; one that counts
//! no subscription at all is said as "no limits reported" (upstream has its
//! own `.noPlan` case for it; the shared vocabulary here does not).

use super::alibaba_coding_plan::{console_number, console_string, expanded};
use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;

/// Where the session lives, for the Settings import button. A sign-in ticket
/// first — any one of the three — then the account markers, the CSRF token
/// the console checks, the browser id it echoes back, and its security token
/// when kept as a cookie. Nothing else leaves the browser.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["qwencloud.com"],
    cookies: &[
        "login_qwencloud_ticket",
        "login_aliyunid_ticket",
        "qwen_sso_ticket",
        "login_current_pk",
        "login_aliyunid_pk",
        "login_aliyunid_csrf",
        "csrf",
        "cna",
        "sec_token",
    ],
};

const ORIGIN: &str = "https://home.qwencloud.com";
const PAGE: &str = "https://home.qwencloud.com/billing/subscription/token-plan-individual";
const USER_INFO: &str = "https://home.qwencloud.com/tool/user/info.json";
const GATEWAY: &str = "https://cs-data.qwencloud.com/data/api.json";
const USAGE_API: &str = "zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/usage";
const SUBSCRIPTION_API: &str = "zeldaHttp.apikeyMgr./tokenplan/personal/api/v2/subscription";
/// The individual plan, international: the only one Qwen Cloud sells.
const COMMODITY_CODE: &str = "sfm_tokenplansolo_public_intl";

pub struct QwenCloudService {
    http: Arc<HttpClient>,
}

impl QwenCloudService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// The console's `sec_token`, which every gateway call carries: from the
    /// subscription page, then from the session's own cookie, then from the
    /// account endpoint. None of the three is a sign-in the console accepts.
    fn security_token(&self, cookie: &str) -> Result<String, Unavailability> {
        let mut page_failure: Option<Unavailability> = None;

        match self.get(PAGE, cookie, "text/html,application/xhtml+xml") {
            Err(reason) => page_failure = Some(reason),
            Ok((status, body)) => {
                if status == 200 {
                    if !is_sign_in_page(&body) {
                        if let Some(token) = security_token_in_page(&body) {
                            return Ok(token);
                        }
                    }
                } else if (500..600).contains(&status) {
                    page_failure = Some(Unavailability::ServerError);
                }
            }
        }

        if let Some(token) = cookie_value("sec_token", cookie) {
            return Ok(token);
        }

        match self.get(USER_INFO, cookie, "application/json, text/plain, */*") {
            Ok((200, body)) => {
                if let Ok(tree) = serde_json::from_str::<Value>(&body) {
                    let tree = expanded(tree);
                    if let Some(token) =
                        first_string(&["secToken", "sec_token", "csrfToken", "token"], &tree)
                    {
                        return Ok(token);
                    }
                }
            }
            _ => {}
        }

        // A page that could not be reached says more than a token not found.
        Err(page_failure.unwrap_or(Unavailability::SessionExpired))
    }

    /// One gateway call, answered with its body. A failure is already the
    /// session's story: redirects are not followed, so a signed-out session
    /// shows up as the redirect to the sign-in page itself.
    fn ask(
        &self,
        api: &str,
        data: &Value,
        token: &str,
        cookie: &str,
    ) -> Result<String, Unavailability> {
        let request = build_request(api, data, token, cookie);
        let mut call = self
            .http
            .client_for_login()
            .post(&request.url)
            .timeout(Duration::from_secs(15));
        for (name, value) in &request.headers {
            call = call.header(name.as_str(), value.as_str());
        }
        let response = call
            .body(request.body)
            .send()
            .map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        match status {
            300..=399 | 401 | 403 => Err(Unavailability::SessionExpired),
            429 => Err(Unavailability::RateLimited),
            200..=299 => response.text().map_err(|_| Unavailability::UnreadableReply),
            _ => Err(Unavailability::ServerError),
        }
    }

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
        let text = response
            .text()
            .map_err(|_| Unavailability::UnreadableReply)?;
        Ok((status, text))
    }
}

impl ProviderService for QwenCloudService {
    fn provider(&self) -> Provider {
        Provider::QwenCloud
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::QwenCloud);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::QwenCloud)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let token = match self.security_token(&cookie) {
            Ok(token) => token,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let usage_body = match self.ask(USAGE_API, &json!({}), &token, &cookie) {
            Ok(body) => body,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };

        let now_ms = crate::timeutil::now_ms();
        let found = reading(&usage_body, account, now_ms);
        if !matches!(found.state, crate::model::State::Live) {
            return found;
        }
        // The tier's name only; a failure here costs the label, not the
        // figures.
        let plan = self
            .ask(
                SUBSCRIPTION_API,
                &json!({"commodityCode": COMMODITY_CODE}),
                &token,
                &cookie,
            )
            .ok()
            .and_then(|body| plan_name(&body));
        ProviderUsage { plan, ..found }
    }
}

// MARK: - The security token

/// The five forms the console writes the token in, tried in the order the
/// upstream service tries them.
pub fn security_token_in_page(html: &str) -> Option<String> {
    quoted_json_capture(html, "secToken")
        .or_else(|| quoted_json_capture(html, "sec_token"))
        .or_else(|| loose_capture(html, "secToken"))
        .or_else(|| loose_capture(html, "sec_token"))
        .or_else(|| loose_capture(html, "csrfToken"))
}

/// `"secToken" : "…"` — the JSON spelling.
fn quoted_json_capture(html: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut cursor = 0;
    while let Some(at) = find_from(html, &needle, cursor) {
        cursor = at + needle.len();
        let rest = html[cursor..].trim_start();
        if !rest.starts_with(':') {
            continue;
        }
        let rest = rest[1..].trim_start();
        if !rest.starts_with('"') {
            continue;
        }
        let Some(end) = rest[1..].find('"') else {
            continue;
        };
        let token = rest[1..1 + end].trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

/// `secToken = "…"` or `secToken: '…'` — the form the page's own scripts write.
fn loose_capture(html: &str, key: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(at) = find_from(html, key, cursor) {
        cursor = at + key.len();
        let after = html[cursor..]
            .strip_prefix(['\'', '"'])
            .unwrap_or(&html[cursor..]);
        let rest = after.trim_start();
        let Some(sep) = rest.chars().next() else {
            continue;
        };
        if sep != ':' && sep != '=' {
            continue;
        }
        let rest = rest[1..].trim_start();
        let Some(quote) = rest.chars().next() else {
            continue;
        };
        if quote != '\'' && quote != '"' {
            continue;
        }
        let Some(end) = rest[1..].find(['\'', '"']) else {
            continue;
        };
        let token = rest[1..1 + end].trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

/// The console's own sign-in page, which a lapsed session is sent to — known
/// by the hosts it names and the form it carries, not just the word.
pub fn is_sign_in_page(html: &str) -> bool {
    let page = html.to_lowercase();
    [
        "passport.alibabacloud.com",
        "signin.aliyun.com",
        "account.alibabacloud.com/login",
        "login.qwencloud.com",
    ]
    .iter()
    .any(|marker| page.contains(marker))
        || (page.contains("login") && page.contains("password") && page.contains("sign in"))
}

/// One cookie's value out of a `name=value; …` header.
pub fn cookie_value(name: &str, header: &str) -> Option<String> {
    header.split(';').find_map(|part| {
        let pair = part.trim();
        let value = pair.strip_prefix(name)?.strip_prefix('=')?;
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn find_from(text: &str, needle: &str, from: usize) -> Option<usize> {
    if from > text.len() {
        return None;
    }
    text[from..].find(needle).map(|at| at + from)
}

// MARK: - The gateway

/// The request as the console page builds it, kept whole so the tests can
/// pin it without a network.
pub struct GatewayRequest {
    pub url: String,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

pub fn build_request(api: &str, data: &Value, token: &str, cookie: &str) -> GatewayRequest {
    let url = format!(
        "{GATEWAY}?action=IntlBroadScopeAspnGateway&product=sfm_bailian&api={}&_v=undefined",
        percent_encode(api)
    );

    // The envelope the console page wraps every call in.
    let mut cornerstone = json!({
        "feTraceId": trace_id(),
        "feURL": PAGE,
        "protocol": "V2",
        "console": "ONE_CONSOLE",
        "productCode": "p_efm",
        "domain": "home.qwencloud.com",
        "consoleSite": "QWENCLOUD",
        "userNickName": "",
        "userPrincipalName": "",
        "xsp_lang": "en-US",
    });
    if let Some(browser) = cookie_value("cna", cookie) {
        cornerstone["X-Anonymous-Id"] = json!(browser);
    }
    let mut payload = data.clone();
    if let Some(object) = payload.as_object_mut() {
        object.insert("cornerstoneParam".into(), cornerstone);
    }
    let params = json!({"Api": api, "V": "1.0", "Data": payload}).to_string();
    let body = form(&[
        ("product", "sfm_bailian"),
        ("action", "IntlBroadScopeAspnGateway"),
        ("sec_token", token),
        ("region", "ap-southeast-1"),
        ("language", "en-US"),
        ("params", &params),
    ]);

    let mut headers = vec![
        (
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded".to_string(),
        ),
        (
            "Accept".to_string(),
            "application/json, text/plain, */*".to_string(),
        ),
        ("Cookie".to_string(), cookie.to_string()),
        ("Origin".to_string(), ORIGIN.to_string()),
        ("Referer".to_string(), PAGE.to_string()),
        ("X-Requested-With".to_string(), "XMLHttpRequest".to_string()),
    ];
    let csrf = cookie_value("login_aliyunid_csrf", cookie).or_else(|| cookie_value("csrf", cookie));
    if let Some(csrf) = csrf {
        headers.push(("x-xsrf-token".to_string(), csrf.clone()));
        headers.push(("x-csrf-token".to_string(), csrf));
    }
    GatewayRequest { url, body, headers }
}

/// A version-4-shaped id, which is all the console wants: it echoes the id
/// back in its logs and checks nothing about its contents.
fn trace_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut state = nanos ^ ((std::process::id() as u128) << 64);
    let mut bytes = [0u8; 16];
    for byte in bytes.iter_mut() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *byte = (state >> 33) as u8;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// A form body with everything but the unreserved characters escaped, so a
/// `+`, `&` or `=` inside the token or the JSON arrives as itself.
pub fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(name, value)| format!("{}={}", percent_encode(name), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

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

// MARK: - Reading the replies

/// What a reply that is not a reading says went wrong, read the way the
/// console's own page reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsoleFailure {
    /// The console wants a sign-in, or refused the one it was given.
    SignedOut,
    /// Anything else the console says failed.
    Failed,
}

pub fn reading(body: &str, account: AccountKey, now_ms: i64) -> ProviderUsage {
    let tree = serde_json::from_str::<Value>(body)
        .ok()
        .map(expanded)
        .filter(|tree| tree.is_object());
    let Some(tree) = tree else {
        // A sign-in page in the place of a reply is a lapsed session, not a
        // changed shape.
        let mut end = body.len().min(4_096);
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        let text = body[..end].to_lowercase();
        let reason = if text.contains("<html") && is_sign_in_page(&text) {
            Unavailability::SessionExpired
        } else {
            Unavailability::UnreadableReply
        };
        return ProviderUsage::unavailable(account, reason);
    };

    match console_failure(&tree) {
        Some(ConsoleFailure::SignedOut) => {
            return ProviderUsage::unavailable(account, Unavailability::SessionExpired)
        }
        Some(ConsoleFailure::Failed) => {
            return ProviderUsage::unavailable(account, Unavailability::ServerError)
        }
        None => {}
    }

    let windows = rolling_windows(&tree, "qwenCloud", now_ms);
    if !windows.is_empty() {
        return ProviderUsage::live_now(account, windows);
    }

    let reason = if first_int(&["TotalCount", "totalCount"], &tree) == Some(0) {
        Unavailability::NoPlan
    } else {
        Unavailability::NoLimitsReported
    };
    ProviderUsage::unavailable(account, reason)
}

/// The console's frame of failure, read from wherever it sits in the reply.
fn console_failure(tree: &Value) -> Option<ConsoleFailure> {
    let frame = first_object(tree, &mut |object| {
        ["successResponse", "success", "Success"].iter().any(|key| {
            object
                .get(*key)
                .map(|value| matches!(value, Value::Bool(false)))
                .unwrap_or(false)
        })
    });
    let code_keys = ["errorCode", "code", "Code", "status", "statusCode"];
    let message_keys = ["errorMsg", "message", "Message", "msg", "statusMessage"];
    let as_tree = |map: &Map<String, Value>| Value::Object(map.clone());
    let code = frame
        .and_then(|map| first_string(&code_keys, &as_tree(map)))
        .or_else(|| first_string(&code_keys, tree));
    let message = frame
        .and_then(|map| first_string(&message_keys, &as_tree(map)))
        .or_else(|| first_string(&message_keys, tree));
    let said = [code, message]
        .into_iter()
        .flatten()
        .map(|text| text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");

    let sign_in = [
        "needlogin",
        "login",
        "tokenerror",
        "request has expired",
        "refresh page",
        "请求已经过期",
    ];
    // A workspace the account may not use is a permission, not a session:
    // signing in again would change nothing.
    let refused = [
        "notauthorised",
        "notauthorized",
        "not authorised",
        "not authorized",
        "unauthorised",
        "unauthorized",
        "access denied",
        "forbidden",
    ];
    if sign_in.iter().any(|phrase| said.contains(phrase)) {
        return Some(ConsoleFailure::SignedOut);
    }
    if !said.contains("workspace.notauthori") && refused.iter().any(|phrase| said.contains(phrase))
    {
        return Some(ConsoleFailure::SignedOut);
    }
    if frame.is_some() {
        return Some(ConsoleFailure::Failed);
    }

    if let Some(status) = first_int(
        &["statusCode", "status_code", "code", "httpStatusCode"],
        tree,
    ) {
        if status != 0 && status != 200 {
            return Some(if status == 401 || status == 403 {
                ConsoleFailure::SignedOut
            } else {
                ConsoleFailure::Failed
            });
        }
    }
    None
}

/// The rolling windows of the plan's personal usage, which the console
/// returns in the same words the Bailian CLI prints: a share used, as a
/// ratio, and a reset in epoch milliseconds, per window.
///
/// A negative figure is never read. A month is a billing month, so its
/// length is not claimed.
pub fn rolling_windows(tree: &Value, id_prefix: &str, now_ms: i64) -> Vec<UsageWindow> {
    struct Rolling {
        key: &'static str,
        reset: &'static str,
        id: &'static str,
        kind: Kind,
        seconds: i64,
        reports_length: bool,
    }
    const ROLLING: [Rolling; 3] = [
        Rolling {
            key: "per5HourPercentage",
            reset: "per5HourResetTime",
            id: "fiveHour",
            kind: Kind::FiveHour,
            seconds: 5 * 3_600,
            reports_length: true,
        },
        Rolling {
            key: "per1WeekPercentage",
            reset: "per1WeekResetTime",
            id: "weekly",
            kind: Kind::Weekly,
            seconds: 7 * 86_400,
            reports_length: true,
        },
        Rolling {
            key: "per1MonthPercentage",
            reset: "per1MonthResetTime",
            id: "monthly",
            kind: Kind::Monthly,
            seconds: 30 * 86_400,
            reports_length: false,
        },
    ];

    let keys: Vec<&str> = ROLLING.iter().map(|window| window.key).collect();
    let Some(usage) = first_object(tree, &mut |object| {
        keys.iter().any(|key| object.contains_key(*key))
    }) else {
        return Vec::new();
    };
    ROLLING
        .iter()
        .filter_map(|rolling| {
            let ratio = usage
                .get(rolling.key)
                .and_then(console_number)
                .filter(|ratio| *ratio >= 0.0)?;
            // Epoch milliseconds only: a reset in the past is a stale
            // figure, not a new window, and is left off rather than moved
            // forward.
            let resets_at = usage
                .get(rolling.reset)
                .and_then(console_number)
                .map(|at| at as i64)
                .filter(|at| *at > now_ms);
            let mut window = UsageWindow::new(
                &format!("{id_prefix}.{}", rolling.id),
                rolling.kind,
                None,
                ratio,
                rolling.seconds,
                resets_at,
            );
            window.reports_length = rolling.reports_length;
            window.is_exhausted = ratio >= 1.0;
            Some(window)
        })
        .collect()
}

/// The tier the subscription names, as the console spells it on the page.
pub fn plan_name(body: &str) -> Option<String> {
    let tree = expanded(serde_json::from_str::<Value>(body).ok()?);
    let code = first_string(&["specCode", "spec_code", "planName", "plan_name"], &tree)?;
    let lower = code.to_lowercase();
    let tier = ["lite", "standard", "pro", "max"].contains(&lower.as_str());
    if tier {
        let mut chars = code.chars();
        let first = chars.next()?.to_uppercase().collect::<String>();
        Some(first + chars.as_str().to_lowercase().as_str())
    } else {
        Some(code)
    }
}

// MARK: - The console's JSON, searched

/// The first object, the outer one before what it holds, that `matches`.
fn first_object<'a, F>(value: &'a Value, matches: &mut F) -> Option<&'a Map<String, Value>>
where
    F: FnMut(&Map<String, Value>) -> bool,
{
    if let Value::Object(map) = value {
        if matches(map) {
            return Some(map);
        }
        for nested in map.values() {
            if let Some(found) = first_object(nested, matches) {
                return Some(found);
            }
        }
    } else if let Value::Array(items) = value {
        for nested in items {
            if let Some(found) = first_object(nested, matches) {
                return Some(found);
            }
        }
    }
    None
}

fn first_string(keys: &[&str], value: &Value) -> Option<String> {
    let mut hit: Option<String> = None;
    first_object(value, &mut |object| {
        for key in keys {
            if let Some(text) = object.get(*key).and_then(console_string) {
                hit = Some(text.to_string());
                return true;
            }
        }
        false
    });
    hit
}

fn first_int(keys: &[&str], value: &Value) -> Option<i64> {
    let mut hit: Option<i64> = None;
    first_object(value, &mut |object| {
        for key in keys {
            if let Some(number) = object.get(*key).and_then(console_number) {
                if let Some(exact) = exact_int(number) {
                    hit = Some(exact);
                    return true;
                }
            }
        }
        false
    });
    hit
}

fn exact_int(number: f64) -> Option<i64> {
    if number.is_finite()
        && number.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&number)
    {
        Some(number as i64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn account() -> AccountKey {
        AccountKey::primary(Provider::QwenCloud)
    }

    /// Before every reset in the fixtures.
    const NOW_MS: i64 = 1_700_000_000_000;

    fn reading_at(body: String) -> ProviderUsage {
        reading(&body, account(), NOW_MS)
    }

    /// Upstream's `qwen-cloud-usage` fixture — second-hand, written from
    /// CodexBar's Qwen Cloud provider, not captured from a live account.
    fn usage_fixture() -> String {
        json!({
            "code": "200",
            "data": {"DataV2": {"data": json!({
                "success": true, "code": 0,
                "data": {
                    "per5HourPercentage": 0.03, "per5HourResetTime": 1700003600000i64,
                    "per1WeekPercentage": 0.01, "per1WeekResetTime": 1700086400000i64
                }}).to_string(),
                "success": true, "httpStatus": 200}},
            "httpStatusCode": 200,
            "successResponse": true
        })
        .to_string()
    }

    #[test]
    fn the_windows_are_read_as_the_shares_the_console_reports_through_its_envelope() {
        let usage = reading_at(usage_fixture());
        assert!(matches!(usage.state, State::Live));
        assert_eq!(
            usage.windows.iter().map(|w| w.kind).collect::<Vec<_>>(),
            vec![Kind::FiveHour, Kind::Weekly]
        );
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|w| w.used_fraction)
                .collect::<Vec<_>>(),
            vec![0.03, 0.01]
        );
        assert_eq!(usage.windows[0].resets_at, Some(1_700_003_600_000));
        assert_eq!(usage.windows[1].resets_at, Some(1_700_086_400_000));
    }

    #[test]
    fn a_monthly_window_is_read_without_claiming_a_length() {
        let usage = reading_at(
            json!({"data": {"per1MonthPercentage": 0.4, "per1MonthResetTime": 1701000000000i64}})
                .to_string(),
        );
        assert_eq!(
            usage.windows.iter().map(|w| w.kind).collect::<Vec<_>>(),
            vec![Kind::Monthly]
        );
        assert!(!usage.windows[0].reports_length);
    }

    #[test]
    fn a_reset_already_gone_by_is_dropped_not_moved_forward() {
        let usage = reading_at(
            json!({"data": {"per5HourPercentage": 0.1, "per5HourResetTime": NOW_MS / 1000 - 60}})
                .to_string(),
        );
        assert_eq!(usage.windows.len(), 1);
        assert_eq!(usage.windows[0].resets_at, None);
    }

    /// Upstream's `qwen-cloud-subscription` fixture.
    #[test]
    fn the_subscription_names_the_tier_as_the_page_spells_it() {
        let subscription = json!({
            "code": "200",
            "data": {"DataV2": {"data": {
                "success": true,
                "data": {
                    "instanceCode": "sfm_tokenplansolo_public_intl-redacted",
                    "specCode": "standard", "status": "VALID", "remainingDays": 29
                },
                "success": true, "httpStatus": 200}},
            },
            "successResponse": true
        });
        assert_eq!(
            plan_name(&subscription.to_string()).as_deref(),
            Some("Standard")
        );
        assert_eq!(plan_name("{}"), None);
    }

    /// Upstream's `qwen-cloud-no-subscription` fixture.
    #[test]
    fn an_account_that_counts_no_subscription_has_no_plan() {
        let fixture = json!({
            "requestId": "00000000-0000-4000-8000-000000000001",
            "code": "200", "message": Value::Null,
            "data": {"RequestId": "00000000-0000-4000-8000-000000000001",
                     "Message": "Successful!", "Uid": 7,
                     "TotalSurplusValue": "0", "TotalCount": 0, "TotalValue": "0",
                     "ProductCode": "sfm_tokenplansolo_public_intl",
                     "Code": "Success", "Success": true},
            "httpStatusCode": "200",
            "successResponse": true
        });
        let usage = reading_at(fixture.to_string());
        assert_eq!(usage.state, State::Unavailable(Unavailability::NoPlan));
    }

    #[test]
    fn a_console_that_wants_a_sign_in_or_refuses_the_session_is_an_expired_session() {
        for reply in [
            json!({"code": "ConsoleNeedLogin", "message": "You need to log in.", "successResponse": false}),
            json!({"statusCode": 403, "message": "Forbidden"}),
            json!({"code": "200",
                   "data": {"success": false, "errorCode": "PostonlyOrTokenError", "errorMsg": "refresh page"},
                   "successResponse": true}),
        ] {
            assert_eq!(
                reading_at(reply.to_string()).state,
                State::Unavailable(Unavailability::SessionExpired),
                "{reply}"
            );
        }
    }

    #[test]
    fn a_workspace_the_account_may_not_use_is_a_failure_not_a_session_to_renew() {
        let reply = json!({"code": "200",
                           "data": {"success": false, "errorCode": "BailianGateway.Workspace.NotAuthorised"},
                           "successResponse": true});
        assert_eq!(
            reading_at(reply.to_string()).state,
            State::Unavailable(Unavailability::ServerError)
        );
    }

    #[test]
    fn a_reply_that_cant_be_read_and_a_sign_in_page_in_its_place() {
        assert_eq!(
            reading_at("not-json".to_string()).state,
            State::Unavailable(Unavailability::UnreadableReply)
        );
        let page = "<html><a href=\"https://passport.alibabacloud.com/login\">Sign in</a></html>";
        assert_eq!(
            reading_at(page.to_string()).state,
            State::Unavailable(Unavailability::SessionExpired)
        );
    }

    #[test]
    fn a_figure_that_isnt_one_is_left_off_and_nothing_left_is_no_limits() {
        let reply =
            json!({"data": {"per5HourPercentage": -0.2, "per1WeekPercentage": Value::Null}});
        assert_eq!(
            reading_at(reply.to_string()).state,
            State::Unavailable(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn the_pages_security_token_is_found_in_the_forms_the_console_writes_it() {
        assert_eq!(
            security_token_in_page("<script>sec_token = \"abc+/=\";</script>").as_deref(),
            Some("abc+/=")
        );
        assert_eq!(
            security_token_in_page("{\"secToken\":\"xyz\"}").as_deref(),
            Some("xyz")
        );
        assert_eq!(security_token_in_page("<html></html>"), None);
        assert_eq!(
            security_token_in_page("var csrfToken = 'tok-9';").as_deref(),
            Some("tok-9")
        );
    }

    #[test]
    fn the_form_keeps_reserved_characters_in_the_token_and_the_json_as_themselves() {
        let body = form(&[("sec_token", "a+b&c=d/東"), ("params", "{\"Api\":\"x\"}")]);
        assert_eq!(
            body,
            "sec_token=a%2Bb%26c%3Dd%2F%E6%9D%B1&params=%7B%22Api%22%3A%22x%22%7D"
        );
    }

    #[test]
    fn the_gateway_request_goes_to_qwen_clouds_own_host_with_the_session_and_csrf() {
        let cookie = "login_qwencloud_ticket=t; login_aliyunid_csrf=c1; cna=anon";
        let request = build_request(USAGE_API, &json!({}), "tok", cookie);
        assert!(request
            .url
            .starts_with("https://cs-data.qwencloud.com/data/api.json?"));
        assert!(request.url.contains("action=IntlBroadScopeAspnGateway"));
        let cookie_header = request
            .headers
            .iter()
            .find(|(name, _)| name == "Cookie")
            .map(|(_, value)| value.as_str());
        assert_eq!(cookie_header, Some(cookie));
        assert!(request
            .headers
            .iter()
            .any(|(name, value)| name == "x-csrf-token" && value == "c1"));
        assert!(request.body.contains("sec_token=tok"));
        assert!(request.body.contains("product=sfm_bailian"));
        // The browser id rides inside the envelope, escaped with the rest.
        assert!(request.body.contains("X-Anonymous-Id%22%3A%22anon"));
        assert!(request
            .headers
            .iter()
            .any(|(name, _)| name == "X-Requested-With"));
    }

    #[test]
    fn one_cookie_of_a_name_is_read_out_of_the_header() {
        assert_eq!(
            cookie_value("sec_token", "a=1; sec_token=t0k; b=2").as_deref(),
            Some("t0k")
        );
        // A name that merely starts the same does not match.
        assert_eq!(cookie_value("sec_token", "sec_token_extra=1"), None);
        assert_eq!(cookie_value("sec_token", "sec_token="), None);
    }

    #[test]
    fn numeric_strings_read_as_numbers_but_booleans_do_not() {
        assert_eq!(console_number(&json!("1.5")), Some(1.5));
        assert_eq!(console_number(&json!(true)), None);
        assert_eq!(first_int(&["code"], &json!({"code": "200"})), Some(200));
        assert_eq!(first_int(&["code"], &json!({"code": "200.5"})), None);
    }
}
