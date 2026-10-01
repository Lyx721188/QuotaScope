//! StepFun's Step Plan, read the way its own console reads it.
//!
//! **A browser session, not a key.** The Step API key buys inference; the
//! plan's allowance is only on the console, which asks
//! `POST /api/step.openapi.devcenter.Dashboard/QueryStepPlanRateLimit` with
//! the signed-in cookies. So that session is the credential. The documented
//! `GET /v1/accounts` answers a key, but with the account's prepaid balance,
//! which is a separate system from the plan.
//!
//! **Two sites, two accounts.** `platform.stepfun.com` and
//! `platform.stepfun.ai` are separate sign-ins, and upstream never sends one
//! site's session to the other: which one is read is a setting there. The
//! Windows port has one StepFun entry, no site choice, and an import that
//! stores one header without naming the host it came from — so the fetch
//! tries the hosts in the import's own order, and only a refused session (the
//! one signal that the header belongs to the other site) moves it on. A
//! session is sent nowhere else.
//!
//! **Two plans, two shapes.** Since 2026-06-18 StepFun sells a Token Plan: a
//! monthly pool of Credits plus 30-day top-up packs, each a bucket with its
//! own size, remainder and end date. The Coding Plan before it — still
//! renewed for anybody who kept auto-renew on — meters a five-hour and a
//! weekly window as a remaining fraction and a reset time. One reply carries
//! whichever the account has, the other's fields zeroed. The Token Plan shape
//! was measured on a live account (Plus); the Coding Plan shape is
//! second-hand, and the fixtures in the tests say so.
//!
//! Credit pools and an account without a plan retain their own semantics.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::{json, Value};
use std::sync::Arc;

/// Where the session lives, for the Settings import button. The session
/// itself first — the console answers nothing without it — then the device id
/// the token was issued to and the load balancer's pin, sent when present.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["platform.stepfun.com", "platform.stepfun.ai"],
    cookies: &["Oasis-Token", "Oasis-Webid", "INGRESSCOOKIE"],
};

const HOSTS: [&str; 2] = ["platform.stepfun.com", "platform.stepfun.ai"];
const API_ROOT: &str = "/api/step.openapi.devcenter.Dashboard";
const RATE_LIMIT_METHOD: &str = "QueryStepPlanRateLimit";
const STATUS_METHOD: &str = "GetStepPlanStatus";
/// The console's own app id, on every call it makes.
const APP_ID: &str = "10300";

/// The session itself; the console answers nothing without it.
const REQUIRED: [&str; 1] = ["Oasis-Token"];
/// Sent when present. `Oasis-Webid` has to agree with the device the token
/// was issued to; `INGRESSCOOKIE` pins the load balancer.
const OPTIONAL: [&str; 2] = ["Oasis-Webid", "INGRESSCOOKIE"];

pub struct StepFunService {
    http: Arc<HttpClient>,
}

impl StepFunService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// One RPC-style call, answered with its body.
    fn post(&self, site: usize, method: &str, cookie: &str) -> Result<Value, Unavailability> {
        let origin = format!("https://{}", HOSTS[site]);
        let mut headers = vec![
            ("Cookie", cookie.to_string()),
            ("Accept", "application/json".to_string()),
            ("Origin", origin.clone()),
            ("Referer", format!("{origin}/plan-usage")),
            ("oasis-appid", APP_ID.to_string()),
            ("oasis-platform", "web".to_string()),
        ];
        // The device the token was issued to. A token presented from another
        // device id is refused as stolen, so the id rides along whenever the
        // session names it.
        if let Some(webid) = web_id(cookie) {
            headers.push(("oasis-webid", webid));
        }
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        session_json(
            &self.http,
            Method::Post,
            &format!("{origin}{API_ROOT}/{method}"),
            &refs,
            Some(&json!({})),
        )
    }
}

impl ProviderService for StepFunService {
    fn provider(&self) -> Provider {
        Provider::StepFun
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::StepFun);
        let Some(stored) = pasted_or_none(keys.api_key(Provider::StepFun)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let Some(cookie) = normalize(&stored) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        // The import reads the hosts in this order and stores one header
        // without naming the host it came from. A refused session is the one
        // signal that the header belongs to the other site, so only a refusal
        // hops; a rate limit or a fault is the host answering, and says so.
        let mut answered: Option<(usize, Value)> = None;
        for site in 0..HOSTS.len() {
            match self.post(site, RATE_LIMIT_METHOD, &cookie) {
                Ok(reply) => {
                    answered = Some((site, reply));
                    break;
                }
                Err(Unavailability::SessionExpired) => {}
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            }
        }
        let Some((site, reply)) = answered else {
            return ProviderUsage::unavailable(account, Unavailability::SessionExpired);
        };

        let snapshot = match parse(&reply) {
            Ok(snapshot) => snapshot,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        // The plan's name is a second request and a nicety; its failure costs
        // the name and nothing else.
        let plan_name = self
            .post(site, STATUS_METHOD, &cookie)
            .ok()
            .and_then(|status| plan_name(&status));

        let windows = windows(&snapshot, crate::timeutil::now_ms());
        if windows.is_empty() {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        }
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.plan = plan_name;
        usage.origin = Some(self.origin_token().into());
        usage
    }
}

/// One call with the imported session as its credential. `fetch_json` folds
/// 401, 403 and the unfollowed redirect into `ApiKeyRefused`, which is the
/// pasted key's refusal — for a browser session those statuses are the
/// session expiring, so the folded reason is read here and remapped. A
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

/// The stored header reduced to the names above, or None when the session
/// token is not in it.
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
        // session: the console checks the token against the device it was
        // issued to, and a mangled one is no token.
        if value.is_empty() || value.contains('"') || value.contains('\\') || value.contains(' ') {
            return None;
        }
        // A host-only row and a domain row for one name is normal in every
        // browser store; the first wins, as it does in any cookie header.
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

/// The device id the console sends as `oasis-webid`, which must match the one
/// the token was issued to: the `Oasis-Webid` cookie when the session carries
/// it, else the token's own `device_id` claim. Read, not verified — it only
/// has to be repeated back to the server that signed it.
pub fn web_id(header: &str) -> Option<String> {
    let mut values: std::collections::HashMap<&str, &str> = Default::default();
    for pair in header.split(';') {
        if let Some((name, value)) = pair.split_once('=') {
            values.insert(name.trim(), value.trim());
        }
    }
    if let Some(webid) = values.get("Oasis-Webid").filter(|webid| !webid.is_empty()) {
        return Some(webid.to_string());
    }
    let token = values.get("Oasis-Token")?;
    // An `access...refresh` pair carries the claim on the refresh half; the
    // halves are tried back to front.
    token.rsplit("...").find_map(device_id_in_jwt)
}

/// The `device_id` claim of a JWT's payload half, base64url and unpadded.
fn device_id_in_jwt(jwt: &str) -> Option<String> {
    use base64::Engine as _;
    let payload = jwt.split('.').nth(1)?;
    let text = payload.replace('-', "+").replace('_', "/");
    let padded = match text.len() % 4 {
        2 => format!("{text}=="),
        3 => format!("{text}="),
        _ => text.clone(),
    };
    let claims: Value = serde_json::from_slice(
        &base64::engine::general_purpose::STANDARD
            .decode(padded)
            .ok()?,
    )
    .ok()?;
    let id = claims.get("device_id")?.as_str()?;
    (!id.is_empty()).then(|| id.to_string())
}

// MARK: - Reading the reply

/// One of the Token Plan's credit buckets: the month's pool, or a top-up
/// pack. Sizes in Credits (1M Credit = ¥1).
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket {
    pub total: f64,
    pub remaining: f64,
    /// When what is left in it lapses. None when not stated.
    pub expires_at: Option<i64>,
    /// When it is refilled — a quarterly or yearly plan's next monthly issue.
    /// None when not stated, which on a monthly plan it is not.
    pub next_reset_at: Option<i64>,
}

/// A Coding Plan window: what is left, as StepFun states it, and when it
/// resets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodingWindow {
    pub remaining_fraction: f64,
    pub resets_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// The Token Plan. Buckets when the reply lists them; otherwise only the
    /// remaining fractions it states, which cannot be added together because
    /// their sizes are not given.
    Credits {
        buckets: Vec<Bucket>,
        subscription_left: Option<f64>,
        top_up_left: Option<f64>,
    },
    /// The Coding Plan's two windows. Either may be absent.
    Windows {
        five_hour: Option<CodingWindow>,
        weekly: Option<CodingWindow>,
    },
}

/// What one read of the rate-limit route carries. The subscription's own
/// name ("Plus", "Mini") is a second request's answer and rides on the
/// `ProviderUsage`, not here: the figures do not depend on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub plan: Plan,
}

pub fn parse(reply: &Value) -> Result<Snapshot, Unavailability> {
    if number(reply.get("status")) != Some(1.0) {
        // A refusal inside a 200. The console words an auth failure as such;
        // anything else is a reply this cannot use.
        let said = ["desc", "message", "code"]
            .iter()
            .filter_map(|key| reply.get(*key).and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        return Err(if said.contains("auth") || said.contains("token") {
            Unavailability::SessionExpired
        } else {
            Unavailability::UnreadableReply
        });
    }

    let five_hour = coding_window(
        reply.get("five_hour_usage_left_rate"),
        reply.get("five_hour_usage_reset_time"),
    );
    let weekly = coding_window(
        reply.get("weekly_usage_left_rate"),
        reply.get("weekly_usage_reset_time"),
    );
    // Classified by what the reply carries, not by `plan_family`: a live
    // window — a reset stated — is the Coding Plan; a Token Plan sends its
    // windows as zero with a reset of "0", which is "no window", not "spent".
    if five_hour.is_some() || weekly.is_some() {
        return Ok(Snapshot {
            plan: Plan::Windows { five_hour, weekly },
        });
    }

    let empty = json!({});
    let credit = reply
        .get("plan_credit_rate_limit")
        .filter(|value| value.is_object())
        .unwrap_or(&empty);
    let buckets = credit
        .get("credit_buckets")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(bucket).collect::<Vec<_>>())
        .unwrap_or_default();
    let subscription_left = fraction(credit.get("subscription_credit_left_rate"));
    let top_up_left = fraction(credit.get("topup_credit_left_rate"));
    if buckets.is_empty() && subscription_left.is_none() {
        // The account counts no Step Plan — a complete answer, not a fault.
        return Err(Unavailability::NoPlan);
    }
    Ok(Snapshot {
        plan: Plan::Credits {
            buckets,
            subscription_left,
            top_up_left,
        },
    })
}

/// `subscription.name` from `GetStepPlanStatus`, when the reply succeeded.
pub fn plan_name(reply: &Value) -> Option<String> {
    if number(reply.get("status"))? != 1.0 {
        return None;
    }
    let name = reply.get("subscription")?.get("name")?.as_str()?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(40).collect())
}

// MARK: - Values

fn bucket(entry: &Value) -> Option<Bucket> {
    let total = figure(entry.get("credit_total")).filter(|total| *total > 0.0)?;
    let residual = figure(entry.get("credit_residual")).filter(|residual| *residual >= 0.0)?;
    Some(Bucket {
        total,
        remaining: residual.min(total),
        expires_at: entry.get("expire_at").and_then(stamp_of),
        next_reset_at: entry.get("next_reset_at").and_then(stamp_of),
    })
}

/// A window only when it states a reset: StepFun zeroes both fields for a
/// window the plan does not have.
fn coding_window(left: Option<&Value>, reset: Option<&Value>) -> Option<CodingWindow> {
    let resets_at = stamp(reset)?;
    let remaining = figure(left)?;
    Some(CodingWindow {
        remaining_fraction: remaining.clamp(0.0, 1.0),
        resets_at,
    })
}

/// A stated fraction, 0…1. StepFun sends zero for "not on this plan" as well
/// as for "none left", so a zero on its own is not taken as either.
fn fraction(value: Option<&Value>) -> Option<f64> {
    figure(value)
        .filter(|fraction| *fraction > 0.0)
        .map(|f| f.min(1.0))
}

/// Numbers arrive as JSON numbers or as decimal strings — the bucket sizes
/// are strings, the rates are not.
fn figure(value: Option<&Value>) -> Option<f64> {
    crate::http::number(value?).filter(|number| number.is_finite())
}

fn number(value: Option<&Value>) -> Option<f64> {
    figure(value)
}

/// A Unix stamp in seconds or milliseconds. Zero is no date: StepFun writes
/// "0" for every time it does not have.
fn stamp(value: Option<&Value>) -> Option<i64> {
    value.and_then(stamp_of)
}

fn stamp_of(value: &Value) -> Option<i64> {
    let stamp = figure(Some(value)).filter(|stamp| *stamp > 0.0)?;
    Some(if stamp > 10_000_000_000.0 {
        stamp as i64
    } else {
        (stamp * 1000.0) as i64
    })
}

// MARK: - The windows

pub fn windows(snapshot: &Snapshot, now_ms: i64) -> Vec<UsageWindow> {
    match &snapshot.plan {
        Plan::Windows { five_hour, weekly } => [
            five_hour
                .as_ref()
                .map(|window| coding(window, "stepfun.5h", Kind::FiveHour, 5 * 3_600)),
            weekly
                .as_ref()
                .map(|window| coding(window, "stepfun.weekly", Kind::Weekly, 7 * 86_400)),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Plan::Credits {
            buckets,
            subscription_left,
            top_up_left,
        } => {
            if !buckets.is_empty() {
                // One ring for the month's pool and any packs, as Qoder's
                // plan-plus-packs total is: they are spent from one balance,
                // soonest-lapsing first, so what is left is their sum.
                let total: f64 = buckets.iter().map(|bucket| bucket.total).sum();
                let remaining: f64 = buckets.iter().map(|bucket| bucket.remaining).sum();
                let mut window = UsageWindow::new(
                    "stepfun.credits",
                    Kind::Credits,
                    None,
                    ((total - remaining) / total).clamp(0.0, 1.0),
                    // Thirty days as a sort key, not a stated length: the
                    // pool is a month and a pack is its own thirty days,
                    // counted from different starts.
                    30 * 86_400,
                    // A refill the reply states and that is still ahead. A
                    // monthly plan states none: its pool simply ends, which
                    // is the expiry, not a reset.
                    buckets
                        .iter()
                        .filter_map(|bucket| bucket.next_reset_at)
                        .filter(|at| *at > now_ms)
                        .min(),
                );
                window.reports_length = false;
                window.is_exhausted = remaining <= 0.0;
                window.set_expiring_parts(
                    buckets
                        .iter()
                        .filter_map(|bucket| bucket.expires_at.map(|at| (bucket.remaining, at))),
                    now_ms,
                );
                vec![window]
            } else {
                // No sizes, only fractions. The subscription's is the plan; a
                // pack's fraction of an unstated size cannot be added to it.
                let Some(left) = subscription_left.or(*top_up_left) else {
                    return Vec::new();
                };
                let mut window = UsageWindow::new(
                    "stepfun.credits",
                    Kind::Credits,
                    None,
                    1.0 - left,
                    30 * 86_400,
                    None,
                );
                window.reports_length = false;
                window.is_exhausted = left <= 0.0;
                vec![window]
            }
        }
    }
}

fn coding(remaining: &CodingWindow, id: &str, kind: Kind, seconds: i64) -> UsageWindow {
    let mut window = UsageWindow::new(
        id,
        kind,
        None,
        1.0 - remaining.remaining_fraction,
        seconds,
        Some(remaining.resets_at),
    );
    window.is_exhausted = remaining.remaining_fraction <= 0.0;
    window
}

/// The soonest bucket to lapse, among the ones still ahead of `now` with
/// something left in them.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Before every reset in the fixtures.
    const NOW_MS: i64 = 1_700_000_000_000;

    fn coding_reply() -> Value {
        json!({
            "status": 1,
            "five_hour_usage_left_rate": 0.7, "five_hour_usage_reset_time": 1_700_003_600i64,
            "weekly_usage_left_rate": 0.5, "weekly_usage_reset_time": "1700086400",
            "plan_credit_rate_limit": {
                "credit_buckets": [],
                "subscription_credit_left_rate": 0, "topup_credit_left_rate": 0
            }
        })
    }

    fn token_reply() -> Value {
        json!({
            "status": 1,
            "five_hour_usage_left_rate": 0, "five_hour_usage_reset_time": "0",
            "weekly_usage_left_rate": 0, "weekly_usage_reset_time": "0",
            "plan_credit_rate_limit": {
                "credit_buckets": [
                    {"credit_total": "1000000", "credit_residual": "250000",
                     "expire_at": 1_700_100_000i64, "next_reset_at": 1_700_050_000i64},
                    {"credit_total": 500000, "credit_residual": 0, "expire_at": 1_700_200_000i64}
                ],
                "subscription_credit_left_rate": 0.8,
                "topup_credit_left_rate": 0.5
            }
        })
    }

    #[test]
    fn the_coding_plans_two_windows_are_stated_lengths() {
        let windows = windows(&parse(&coding_reply()).unwrap(), NOW_MS);
        assert_eq!(
            windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["stepfun.5h", "stepfun.weekly"]
        );
        // What StepFun states is what is left; the ring shows what is used.
        assert!((windows[0].used_fraction - 0.3).abs() < 1e-9);
        assert_eq!(windows[0].kind, Kind::FiveHour);
        assert_eq!(windows[0].window_seconds, 5 * 3_600);
        assert!(windows[0].reports_length);
        assert_eq!(windows[0].resets_at, Some(1_700_003_600_000));
        // A reset that arrived as a decimal string reads the same way.
        assert_eq!(windows[1].used_fraction, 0.5);
        assert_eq!(windows[1].kind, Kind::Weekly);
        assert_eq!(windows[1].resets_at, Some(1_700_086_400_000));
    }

    #[test]
    fn the_token_plan_is_one_ring_for_the_pool_and_the_packs() {
        let rings = windows(&parse(&token_reply()).unwrap(), NOW_MS);
        assert_eq!(rings.len(), 1);
        let ring = &rings[0];
        // 1,250,000 spent out of 1.5M Credits.
        assert!((ring.used_fraction - 1_250_000.0 / 1_500_000.0).abs() < 1e-9);
        assert_eq!(ring.kind, Kind::Credits);
        assert!(!ring.reports_length);
        // The refill the reply states, still ahead. The spent pack's expiry
        // says nothing: an empty pack lapses at nothing.
        assert_eq!(ring.resets_at, Some(1_700_050_000_000));
        assert_eq!(ring.next_expiry_ms, Some(1_700_100_000_000));
        assert!(!ring.is_exhausted);

        // Everything spent is spent, however the buckets split it.
        let empty = json!({
            "status": 1,
            "plan_credit_rate_limit": {"credit_buckets": [
                {"credit_total": "100", "credit_residual": "0", "expire_at": 1_700_100_000i64}
            ]}
        });
        assert!(windows(&parse(&empty).unwrap(), NOW_MS)[0].is_exhausted);
    }

    #[test]
    fn sizes_without_buckets_leave_only_the_subscriptions_fraction() {
        let reply = json!({
            "status": 1,
            "plan_credit_rate_limit": {
                "subscription_credit_left_rate": "0.6", "topup_credit_left_rate": 0.2
            }
        });
        let windows = windows(&parse(&reply).unwrap(), NOW_MS);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_fraction, 0.4);
        assert_eq!(windows[0].id, "stepfun.credits");
        // No sizes and no reset: the row sorts on thirty days it never said.
        assert!(!windows[0].reports_length);
        assert_eq!(windows[0].resets_at, None);
    }

    #[test]
    fn an_account_that_counts_no_plan_is_no_limits_not_a_fault() {
        // Zeroed windows and no credit plan: the reply the other plan's
        // account would send.
        let none = json!({
            "status": 1,
            "five_hour_usage_left_rate": 0, "five_hour_usage_reset_time": "0",
            "weekly_usage_left_rate": 0, "weekly_usage_reset_time": "0",
            "plan_credit_rate_limit": {}
        });
        assert_eq!(parse(&none), Err(Unavailability::NoPlan));
    }

    #[test]
    fn a_refusal_inside_a_200_is_read_from_how_the_console_words_it() {
        let expired =
            json!({"status": 0, "code": "unauthenticated", "message": "auth failed: no token"});
        assert_eq!(parse(&expired), Err(Unavailability::SessionExpired));
        let broken = json!({"status": 0, "message": "bad request"});
        assert_eq!(parse(&broken), Err(Unavailability::UnreadableReply));
        assert_eq!(
            parse(&json!("not the reply")),
            Err(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn the_plan_name_is_a_nicety_the_second_request_carries() {
        assert_eq!(
            plan_name(&json!({"status": 1, "subscription": {"name": "  Plus  "}})).as_deref(),
            Some("Plus")
        );
        assert_eq!(
            plan_name(&json!({"status": 1, "subscription": {"name": "Mini"}})).as_deref(),
            Some("Mini")
        );
        // A failed request costs the name and nothing else.
        assert_eq!(
            plan_name(&json!({"status": 0, "subscription": {"name": "Plus"}})),
            None
        );
        assert_eq!(
            plan_name(&json!({"status": 1, "subscription": {"name": "   "}})),
            None
        );
        assert_eq!(plan_name(&json!({"status": 1})), None);
        let long = "a".repeat(50);
        assert_eq!(
            plan_name(&json!({"status": 1, "subscription": {"name": long}})).map(|name| name.len()),
            Some(40)
        );
    }

    #[test]
    fn a_zeroed_window_is_no_window_not_a_spent_one() {
        let reply = json!({
            "status": 1,
            "five_hour_usage_left_rate": 0, "five_hour_usage_reset_time": "0",
            "weekly_usage_left_rate": 0, "weekly_usage_reset_time": "0",
            "plan_credit_rate_limit": {"credit_buckets": [
                {"credit_total": "10", "credit_residual": "5"}
            ]}
        });
        let snapshot = parse(&reply).unwrap();
        // The zeroed windows fall through to the credit plan they belong to.
        assert!(matches!(snapshot.plan, Plan::Credits { .. }));
        assert_eq!(windows(&snapshot, NOW_MS)[0].used_fraction, 0.5);
    }

    #[test]
    fn a_stamp_is_seconds_or_milliseconds_and_zero_is_no_date() {
        assert_eq!(stamp(Some(&json!(1_700_000_000))), Some(NOW_MS));
        assert_eq!(stamp(Some(&json!("1700000000"))), Some(NOW_MS));
        assert_eq!(stamp(Some(&json!(1_700_000_000_000i64))), Some(NOW_MS));
        assert_eq!(stamp(Some(&json!("0"))), None);
        assert_eq!(stamp(Some(&json!(0))), None);
        assert_eq!(stamp(Some(&json!(-1))), None);
        assert_eq!(stamp(Some(&json!(true))), None);
        assert_eq!(stamp(Some(&Value::Null)), None);
        assert_eq!(stamp(None), None);
    }

    #[test]
    fn the_header_keeps_only_the_consoles_cookies_and_requires_the_token() {
        let header = normalize("Cookie: sid=1; Oasis-Token=tok; Oasis-Webid=web; INGRESSCOOKIE=lb")
            .expect("the token is there");
        assert_eq!(header, "Oasis-Token=tok; Oasis-Webid=web; INGRESSCOOKIE=lb");

        // The session itself is the one the console answers nothing without.
        assert_eq!(normalize("sid=1; Oasis-Webid=web"), None);
        // A wanted value that cannot be sent unaltered is no session.
        assert_eq!(normalize("Oasis-Token=a b"), None);
        assert_eq!(normalize("Oasis-Token=\"quoted\""), None);
        assert_eq!(normalize("Oasis-Token=a\\b"), None);
        assert_eq!(normalize("Oasis-Token="), None);
        assert_eq!(normalize("Oasis-Token=tok\ninjected"), None);
        // The first of a host-only and a domain row wins.
        assert_eq!(
            normalize("Oasis-Token=first; Oasis-Token=second").as_deref(),
            Some("Oasis-Token=first")
        );
    }

    #[test]
    fn the_device_id_comes_from_the_cookie_or_the_tokens_own_claim() {
        use base64::Engine as _;
        let claims = json!({"device_id": "dev-9"}).to_string();
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims);
        let jwt = format!("header.{payload}.signature");

        assert_eq!(
            web_id("Oasis-Token=tok; Oasis-Webid=web-1").as_deref(),
            Some("web-1")
        );
        assert_eq!(
            web_id(&format!("Oasis-Token={jwt}")).as_deref(),
            Some("dev-9")
        );
        // An `access...refresh` pair carries the claim on the refresh half,
        // which is the half tried first.
        let plain = json!({"sub": "someone"}).to_string();
        let access = format!(
            "a.{}.c",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(plain)
        );
        assert_eq!(
            web_id(&format!("Oasis-Token={access}...{jwt}")).as_deref(),
            Some("dev-9")
        );

        assert_eq!(web_id("Oasis-Token=tok"), None);
        assert_eq!(web_id("sid=1"), None);
    }
}
