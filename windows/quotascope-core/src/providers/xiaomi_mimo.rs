//! Xiaomi's MiMo open platform, read through the console's own endpoints.
//!
//! **A browser session, not a key.** The platform issues API keys for
//! inference, and none of them answer the console's account routes — the plan
//! and the balance are behind the same `api-platform_serviceToken` cookie the
//! web console uses. So the credential is a session.
//!
//! **The ring is the Coding Plan, not the balance.** The account carries two
//! separate things: a monthly token allowance bought as a plan, and a prepaid
//! cash balance for anything past it. Only the first has a denominator, so
//! only the first can be a ring — the balance rides along as money on the
//! card, with no "warn below" line, because nothing the platform reports says
//! it is running out. An account with no plan is not a fault; it is an
//! account that buys tokens by the yuan, and it says so (upstream has its own
//! `.xiaomiNoCodingPlan`; the shared vocabulary says "no limits reported").
//!
//! **The platform answers a refused session with HTTP 200** and a code in the
//! body — the shape that had Zhipu reporting "the service returned an error"
//! for the commonest mistake there is — so the body is read on every route,
//! not just the failing one. What each route threw is kept, not discarded: if
//! nothing answered at all, the worst of the three failures is reported, so a
//! session problem outranks a timeout and the reader is sent to the right
//! remedy.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button. The two the
/// platform will not answer without first, then the two it carries when
/// present. Nothing else leaves the browser.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["platform.xiaomimimo.com"],
    cookies: &[
        "api-platform_serviceToken",
        "userId",
        "api-platform_ph",
        "api-platform_slh",
    ],
};

const HOST: &str = "platform.xiaomimimo.com";
const BASE: &str = "https://platform.xiaomimimo.com/api/v1";
/// The page the console fetches these from, so the `Referer` is true rather
/// than invented.
const CONSOLE_URL: &str = "https://platform.xiaomimimo.com/#/console/balance";
const PLAN_DETAIL: &str = "/tokenPlan/detail";
const PLAN_USAGE: &str = "/tokenPlan/usage";
const BALANCE: &str = "/balance";

/// The two the platform will not answer without.
const REQUIRED: [&str; 2] = ["api-platform_serviceToken", "userId"];
/// Carried rather than required: the console includes them and the endpoints
/// work without them, so a session that only has the two above is still a
/// session.
const OPTIONAL: [&str; 2] = ["api-platform_ph", "api-platform_slh"];

pub struct XiaomiMiMoService {
    http: Arc<HttpClient>,
}

impl XiaomiMiMoService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    fn get(&self, path: &str, cookie: &str) -> Result<Value, Unavailability> {
        let headers = [
            ("Cookie", cookie.to_string()),
            ("Accept", "application/json, text/plain, */*".to_string()),
            ("Origin", format!("https://{HOST}")),
            ("Referer", CONSOLE_URL.to_string()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        session_json(
            &self.http,
            Method::Get,
            &format!("{BASE}{path}"),
            &refs,
            None,
        )
    }
}

impl ProviderService for XiaomiMiMoService {
    fn provider(&self) -> Provider {
        Provider::XiaomiMiMo
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::XiaomiMiMo);
        let Some(stored) = pasted_or_none(keys.api_key(Provider::XiaomiMiMo)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let Some(cookie) = normalize(&stored) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        // The plan is what the ring is for, so its failure is the call's
        // failure. The balance is a line on the card, so a balance route that
        // does not answer costs that line and nothing else.
        let routes: [Result<Value, Unavailability>; 3] = [
            self.get(PLAN_DETAIL, &cookie),
            self.get(PLAN_USAGE, &cookie),
            self.get(BALANCE, &cookie),
        ];

        // Every route is the same envelope, so one expired session shows up
        // on all three. Reported from whichever answered rather than from a
        // fourth request made only to ask.
        for reply in routes.iter().flatten() {
            if let Some(reason) = refusal(reply) {
                return ProviderUsage::unavailable(account, reason);
            }
        }

        // Nothing answered. The worst of what the routes actually said, so a
        // session problem outranks a timeout.
        let failures: Vec<Unavailability> = routes
            .iter()
            .filter_map(|route| route.as_ref().err())
            .copied()
            .collect();
        if failures.len() == 3 {
            return ProviderUsage::unavailable(account, worst(&failures));
        }

        let Some(plan) = parse_plan(routes[0].as_ref().ok(), routes[1].as_ref().ok()) else {
            // Not a failure: an account can buy tokens by the yuan with no
            // plan at all. Saying so beats a ring at 0%, which would read as
            // a full month nobody has.
            return ProviderUsage::unavailable(account, Unavailability::NoPlan);
        };
        let balance = routes[2].as_ref().ok().and_then(parse_balance);
        let mut found = reading(&plan, balance, account);
        found.origin = Some(self.origin_token().into());
        found
    }
}

/// One call with the imported session as its credential. `fetch_json` folds
/// 401, 403 and the unfollowed redirect into `ApiKeyRefused`, which is the
/// pasted key's refusal — for a browser session those statuses are the
/// session expiring (an expired session is answered by redirecting the API
/// call at the login flow), so the folded reason is read here and remapped. A
/// request that sends no key can arrive at that folding no other way.
fn session_json(
    http: &HttpClient,
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Value, Unavailability> {
    match http.fetch_json_detailed(method, url, headers, body) {
        Ok(value) => Ok(value),
        Err(HttpFailure::NotFound) => Err(Unavailability::ServerError),
        Err(HttpFailure::Unavailable(Unavailability::ApiKeyRefused)) => {
            Err(Unavailability::SessionExpired)
        }
        Err(HttpFailure::Unavailable(reason)) => Err(reason),
    }
}

// MARK: - The stored header

/// The stored header reduced to the names above, or None when the two
/// required ones are not both in it. Only these are kept: a browser store for
/// this host also holds analytics and preference cookies, and a credential
/// that forwards everything it found is a credential that leaks whatever the
/// site adds next.
pub fn normalize(input: &str) -> Option<String> {
    let header = header_body(input)?;
    let mut kept: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for pair in header.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        if !REQUIRED.contains(&name) && !OPTIONAL.contains(&name) {
            continue;
        }
        // A wanted value that cannot be sent unaltered refuses the whole
        // session, rather than a mangled token standing in for a real one.
        if value.is_empty() || value.contains('"') || value.contains('\\') || value.contains(' ') {
            return None;
        }
        // A host-only row and a domain row for one name is normal in every
        // browser store, and the host match returns both. The first wins, as
        // it does in any cookie header — refusing here would discard the
        // whole browser over something that is not a fault.
        if seen.iter().any(|seen| seen == name) {
            continue;
        }
        seen.push(name.to_string());
        kept.push(format!("{name}={value}"));
    }
    if !REQUIRED
        .iter()
        .all(|required| seen.iter().any(|seen| seen == required))
    {
        return None;
    }
    Some(kept.join("; "))
}

/// The header without its optional `Cookie:` front, when it is one a request
/// may carry at all.
fn header_body(input: &str) -> Option<&str> {
    // A header assembled from an arbitrary string is an injection if a value
    // carries a newline, and a cookie value is never anything but printable
    // ASCII.
    if input.chars().any(|c| (c as u32) < 32 || (c as u32) > 126) {
        return None;
    }
    let trimmed = input.trim();
    let header = if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("cookie:") {
        trimmed[7..].trim()
    } else {
        trimmed
    };
    let carryable = !header.is_empty() && header.len() <= 32_768;
    carryable.then_some(header)
}

// MARK: - Reading the replies

/// The plan's month of tokens: used, and out of how many.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub used: i64,
    pub limit: i64,
    pub period_end: Option<i64>,
    pub code: Option<String>,
}

/// The month's plan, from the two routes that carry it: `usage` for the
/// figures, `detail` for the period and the plan's code. None is an account
/// with no plan running — a real state, not a failed read.
pub fn parse_plan(detail: Option<&Value>, usage: Option<&Value>) -> Option<Plan> {
    // The envelope first. The platform answers over HTTP 200 whatever
    // happened, so a body whose `code` is not zero is a failure wearing a
    // success's clothes.
    let usage = usage?;
    if envelope_code(usage) != Some(0) {
        return None;
    }
    // `monthUsage.items` is a list because the console draws a row per
    // bucket; the plan's own allowance is the first. An empty list is an
    // account with no plan — which is why this returns None rather than a
    // zero: a ring at 0% would say "you have a full month left".
    let item = usage
        .get("data")?
        .get("monthUsage")?
        .get("items")?
        .as_array()?
        .first()?;
    let used = item.get("used")?.as_i64()?;
    let limit = item.get("limit")?.as_i64()?;
    if limit <= 0 {
        return None;
    }

    let detail = detail.and_then(|body| {
        if envelope_code(body) != Some(0) {
            return None;
        }
        body.get("data")
    });
    // An expired plan reports last month's numbers until it is renewed.
    // Those are not a current allowance, so they are not drawn.
    if detail
        .and_then(|data| data.get("expired"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        return None;
    }
    Some(Plan {
        used,
        limit,
        period_end: detail
            .and_then(|data| data.get("currentPeriodEnd"))
            .and_then(Value::as_str)
            .and_then(period_end_ms),
        code: detail
            .and_then(|data| data.get("planCode"))
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// The prepaid balance, as the platform states it: an amount as a decimal
/// string, and a currency. Reported by every account, including one with no
/// plan.
pub fn parse_balance(reply: &Value) -> Option<CreditAmount> {
    if envelope_code(reply) != Some(0) {
        return None;
    }
    let body = reply.get("data")?;
    let amount = body
        .get("balance")?
        .as_str()?
        .parse::<f64>()
        .ok()
        .filter(|amount| amount.is_finite())?;
    let currency = body.get("currency")?.as_str()?.trim();
    if currency.is_empty() {
        return None;
    }
    Some(CreditAmount {
        amount,
        currency: currency.to_string(),
    })
}

/// The envelope's `code` as an integer, when it is one.
fn envelope_code(reply: &Value) -> Option<i64> {
    reply.get("code").and_then(Value::as_i64)
}

/// The envelope's own verdict: the platform answers a refused session with a
/// code in the body. Any other code is left to the payload readers, which
/// draw nothing from a body that fails its own envelope.
pub fn refusal(reply: &Value) -> Option<Unavailability> {
    match envelope_code(reply) {
        Some(401) | Some(403) => Some(Unavailability::SessionExpired),
        _ => None,
    }
}

/// The most actionable of several failures. A session that has to be signed
/// in again outranks a rate limit, which outranks a timeout, which outranks
/// anything a body that could not be read would have said.
pub fn worst(failures: &[Unavailability]) -> Unavailability {
    fn rank(reason: &Unavailability) -> u8 {
        match reason {
            Unavailability::SessionExpired => 3,
            Unavailability::RateLimited | Unavailability::ServerError => 2,
            Unavailability::Unreachable => 1,
            _ => 0,
        }
    }
    failures
        .iter()
        .copied()
        .max_by_key(|reason| rank(reason))
        .unwrap_or(Unavailability::Unreachable)
}

/// The console's own format, `yyyy-MM-dd HH:mm:ss`, in UTC. Not ISO-8601, so
/// the shared ISO reader returns nothing on it and the card would silently
/// lose its reset.
fn period_end_ms(text: &str) -> Option<i64> {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(chrono::Utc.from_utc_datetime(&naive).timestamp_millis())
}

// MARK: - The reading

/// The plan as the one ring, the balance as a line on the card. The balance
/// is a display string rather than a `credit_remaining` on purpose: there is
/// no allowance to compare it against, so nothing can be said to be running
/// out.
pub fn reading(plan: &Plan, balance: Option<CreditAmount>, account: AccountKey) -> ProviderUsage {
    let mut window = UsageWindow::new(
        "xiaomi.plan",
        Kind::Monthly,
        None,
        plan.used as f64 / plan.limit as f64,
        // Thirty days is a sort key, not a reported length: the platform
        // states when the period ends and never how long it is, and a billing
        // month is not a fixed number of seconds.
        30 * 86_400,
        plan.period_end,
    );
    window.reports_length = false;
    window.is_exhausted = plan.used >= plan.limit;
    let mut usage = ProviderUsage::live_now(account, vec![window]);
    usage.plan = plan.code.clone();
    usage.credit_balance = balance.map(|money| money.rail_text());
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn account() -> AccountKey {
        AccountKey::primary(Provider::XiaomiMiMo)
    }

    fn plan_reply() -> (Value, Value) {
        (
            json!({"code": 0, "data": {
                "planCode": "mimo-coding",
                "currentPeriodEnd": "2026-10-31 23:59:59",
                "expired": false
            }}),
            json!({"code": 0, "data": {"monthUsage": {"items": [
                {"name": "tokens", "used": 1200, "limit": 10000}
            ]}}}),
        )
    }

    #[test]
    fn the_plan_is_the_first_bucket_of_the_usage_and_the_details_period() {
        let (detail, usage) = plan_reply();
        let plan = parse_plan(Some(&detail), Some(&usage)).expect("a plan is running");
        assert_eq!(plan.used, 1200);
        assert_eq!(plan.limit, 10_000);
        assert_eq!(plan.code.as_deref(), Some("mimo-coding"));
        // The console's own format, read as UTC (2026-10-31 23:59:59).
        assert_eq!(plan.period_end, Some(1_793_491_199_000));
        // A ring at 0% would say "a full month nobody has" — the detail
        // route is optional and its absence costs the period, not the ring.
        let plan = parse_plan(None, Some(&usage)).expect("a plan is running");
        assert_eq!(plan.period_end, None);
    }

    #[test]
    fn an_expired_plan_reports_nothing_until_it_is_renewed() {
        let (detail, usage) = plan_reply();
        let expired = json!({"code": 0, "data": {"expired": true}});
        // Last month's numbers are not a current allowance.
        assert_eq!(parse_plan(Some(&expired), Some(&usage)), None);
        assert!(parse_plan(Some(&detail), Some(&usage)).is_some());
    }

    #[test]
    fn a_body_whose_code_is_not_zero_is_a_failure_in_successs_clothes() {
        let (detail, _) = plan_reply();
        // A code 500 carrying an empty items list is a fault, not "no plan".
        let faulted = json!({"code": 500, "message": "boom", "data": null});
        assert_eq!(parse_plan(Some(&detail), Some(&faulted)), None);
        assert_eq!(parse_plan(None, Some(&faulted)), None);
        // An envelope that reads but carries no plan draws nothing.
        let empty = json!({"code": 0, "data": {"monthUsage": {"items": []}}});
        assert_eq!(parse_plan(Some(&detail), Some(&empty)), None);
        let zero = json!({"code": 0, "data": {"monthUsage": {"items": [
            {"used": 0, "limit": 0}
        ]}}});
        assert_eq!(parse_plan(Some(&detail), Some(&zero)), None);
        // A body that is not the envelope at all draws nothing.
        assert_eq!(parse_plan(Some(&detail), Some(&json!("no"))), None);
    }

    #[test]
    fn the_balance_is_the_amount_and_the_currency_the_platform_names() {
        let balance = json!({"code": 0, "data": {"balance": "12.34", "currency": " CNY "}});
        assert_eq!(
            parse_balance(&balance),
            Some(CreditAmount {
                amount: 12.34,
                currency: "CNY".into()
            })
        );
        // A fault wearing a success's clothes, an amount that is not one, and
        // a currency that names nothing all draw nothing.
        assert_eq!(
            parse_balance(&json!({"code": 500, "data": {"balance": "12.34", "currency": "CNY"}})),
            None
        );
        assert_eq!(
            parse_balance(&json!({"code": 0, "data": {"balance": "n/a", "currency": "CNY"}})),
            None
        );
        assert_eq!(
            parse_balance(&json!({"code": 0, "data": {"balance": "12.34", "currency": "  "}})),
            None
        );
    }

    #[test]
    fn a_refused_session_is_answered_in_the_body_over_http_200() {
        assert_eq!(
            refusal(&json!({"code": 401, "message": "auth"})),
            Some(Unavailability::SessionExpired)
        );
        assert_eq!(
            refusal(&json!({"code": 403})),
            Some(Unavailability::SessionExpired)
        );
        // Any other code is the payload readers' business.
        assert_eq!(refusal(&json!({"code": 0})), None);
        assert_eq!(refusal(&json!({"code": 500})), None);
        assert_eq!(refusal(&json!("not the envelope")), None);
    }

    #[test]
    fn the_worst_failure_is_the_one_with_the_remedy() {
        assert_eq!(
            worst(&[Unavailability::Unreachable, Unavailability::SessionExpired]),
            Unavailability::SessionExpired
        );
        assert_eq!(
            worst(&[Unavailability::RateLimited, Unavailability::Unreachable]),
            Unavailability::RateLimited
        );
        assert_eq!(
            worst(&[Unavailability::ServerError, Unavailability::RateLimited]),
            Unavailability::RateLimited
        );
        assert_eq!(
            worst(&[Unavailability::UnreadableReply, Unavailability::Unreachable]),
            Unavailability::Unreachable
        );
        assert_eq!(worst(&[]), Unavailability::Unreachable);
    }

    #[test]
    fn the_reading_is_the_plan_as_one_monthly_ring_and_the_balance_as_money() {
        let plan = Plan {
            used: 1200,
            limit: 10_000,
            period_end: Some(1_793_491_199_000),
            code: Some("mimo-coding".into()),
        };
        let usage = reading(
            &plan,
            Some(CreditAmount {
                amount: 12.34,
                currency: "CNY".into(),
            }),
            account(),
        );
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.windows.len(), 1);
        let window = &usage.windows[0];
        assert_eq!(window.id, "xiaomi.plan");
        assert_eq!(window.used_fraction, 0.12);
        assert_eq!(window.kind, Kind::Monthly);
        // The platform states when the period ends and never how long it is.
        assert!(!window.reports_length);
        assert_eq!(window.window_seconds, 30 * 86_400);
        assert_eq!(window.resets_at, Some(1_793_491_199_000));
        assert!(!window.is_exhausted);
        assert_eq!(usage.plan.as_deref(), Some("mimo-coding"));
        assert_eq!(usage.credit_balance.as_deref(), Some("¥12.34"));
        // There is no allowance to compare the balance against, so it is a
        // display string only.
        assert_eq!(usage.credit_remaining, None);

        // A plan used to its limit is the platform's own verdict.
        let spent = Plan {
            used: 10_000,
            limit: 10_000,
            period_end: None,
            code: None,
        };
        assert!(reading(&spent, None, account()).windows[0].is_exhausted);
    }

    #[test]
    fn the_header_keeps_the_consoles_cookies_and_requires_the_pair() {
        let header = normalize(
            "api-platform_serviceToken=tok; userId=7; theme=dark; api-platform_ph=p; \
             api-platform_slh=s",
        )
        .expect("both required cookies are there");
        assert_eq!(
            header,
            "api-platform_serviceToken=tok; userId=7; api-platform_ph=p; api-platform_slh=s"
        );

        // The two the platform will not answer without.
        assert_eq!(normalize("api-platform_serviceToken=tok"), None);
        assert_eq!(normalize("userId=7"), None);
        assert_eq!(normalize(""), None);
        // A wanted value that cannot be sent unaltered refuses the session.
        assert_eq!(normalize("api-platform_serviceToken=a b; userId=7"), None);
        assert_eq!(normalize("api-platform_serviceToken=\"q\"; userId=7"), None);
        // The first of a host-only and a domain row wins.
        assert_eq!(
            normalize("userId=first; userId=second; api-platform_serviceToken=tok").as_deref(),
            Some("userId=first; api-platform_serviceToken=tok")
        );
        // A pasted header's `Cookie:` front is tolerated.
        assert_eq!(
            normalize("Cookie: userId=7; api-platform_serviceToken=tok").as_deref(),
            Some("userId=7; api-platform_serviceToken=tok")
        );
        assert_eq!(normalize("userId=7\n; api-platform_serviceToken=tok"), None);
    }
}
