//! Qoder's credits, read the way its own account page reads them.
//!
//! **A browser session, not a key.** Qoder publishes no usage API. The
//! account page (`/account/usage`) asks `GET /api/v2/me/usages/big_model_credits`
//! with the signed-in cookies and draws what comes back, so that session is
//! the credential.
//!
//! **Two sites, two accounts.** `qoder.com` and `qoder.com.cn` are separate
//! sign-ins, and upstream never sends one site's session to the other: which
//! one is read is a setting there. The Windows port has one Qoder entry, no
//! site choice, and an import that stores one header without naming the host
//! it came from — so the fetch tries the hosts in the import's own order, and
//! only a refused session (the one signal that the header belongs to the
//! other site) moves it on. A session is sent nowhere else.
//!
//! **Not the IDE's own figures.** The Qoder app caches a credit reading of
//! its own and exposes it over a local socket, but other monitors that read
//! it found it disagreeing with the billing page for paid accounts. The web
//! route is the one the page itself uses.
//!
//! `totalQuota` is the account's own: the plan plus any resource pack bought
//! on top. `sharedQuota` is a team's pool, present only on a team plan. They
//! are **two rings, never one sum** — a spent personal allowance beside an
//! untouched team pool added together reads as "plenty left" about the pool
//! that is actually stopping you. Both spellings are accepted per field: the
//! mainland site's reply mixes them (`nextResetAt` beside `total_quota`).
//!
//! **What is mapped, not ported.** Upstream kinds the personal credits
//! `.credits` and a team pool `.sharedCredits`; the Windows kind set has
//! neither, so `Spend` stands in for both and the id says which is which.
//! Upstream's `.qoderNoCredits` — an account with nothing to display — has no
//! shared case either, and says "no limits reported".

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button. Qoder's session
/// cookie has no published name — upstream forwards everything the host set
/// and strips analytics at send time — so the import takes the host's cookies
/// as they come and `normalize` does the stripping here.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["qoder.com", "qoder.com.cn"],
    cookies: &["*"],
};

const HOSTS: [&str; 2] = ["qoder.com", "qoder.com.cn"];
const USAGE_PATH: &str = "/api/v2/me/usages/big_model_credits";
/// What the page's own request carries. `Bx-V` belongs to the bot screening
/// in front of the site; a request that does not look like the page it stands
/// in for is the one that screening is there to turn away.
const BX_V: &str = "2.5.35";

/// Prefixes of cookies set by third-party analytics and advertising scripts
/// rather than by Qoder. Alibaba's own are kept — the same family carries the
/// bot screening in front of Alibaba's sites, and dropping one of those is
/// how a session that works in the browser gets refused here.
const ANALYTICS: [&str; 21] = [
    "_ga",
    "_gid",
    "_gat",
    "_gcl",
    "_fbp",
    "_fbc",
    "_clck",
    "_clsk",
    "_hj",
    "_uet",
    "_tt_",
    "_ttp",
    "ajs_",
    "amp_",
    "mp_",
    "hm_",
    "hmaccount",
    "intercom-",
    "__stripe",
    "_rdt",
    "_pin",
];

pub struct QoderService {
    http: Arc<HttpClient>,
}

impl QoderService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    fn usage_json(&self, site: usize, cookie: &str) -> Result<Value, Unavailability> {
        let origin = format!("https://{}", HOSTS[site]);
        let headers = [
            ("Cookie", cookie.to_string()),
            ("Accept", "application/json, text/plain, */*".to_string()),
            ("Origin", origin.clone()),
            ("Referer", format!("{origin}/account/usage")),
            ("X-Requested-With", "XMLHttpRequest".to_string()),
            ("Bx-V", BX_V.to_string()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        session_json(
            &self.http,
            Method::Get,
            &format!("{origin}{USAGE_PATH}"),
            &refs,
            None,
        )
    }

    fn reading(&self, reply: &Value, account: AccountKey) -> ProviderUsage {
        let found = parse(reply).and_then(|snapshot| {
            let windows = windows(&snapshot, crate::timeutil::now_ms());
            if windows.is_empty() {
                Err(Unavailability::NoLimitsReported)
            } else {
                Ok(windows)
            }
        });
        match found {
            Ok(windows) => {
                let mut usage = ProviderUsage::live_now(account, windows);
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

impl ProviderService for QoderService {
    fn provider(&self) -> Provider {
        Provider::Qoder
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Qoder);
        let Some(stored) = pasted_or_none(keys.api_key(Provider::Qoder)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let Some(cookie) = normalize(&stored) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        // The import reads the hosts in this order and stores one header
        // without naming the host it came from. A refused session is the one
        // signal that the header belongs to the other site, so only a refusal
        // hops; a rate limit or a fault is the host answering, and says so.
        for site in 0..HOSTS.len() {
            match self.usage_json(site, &cookie) {
                Ok(reply) => return self.reading(&reply, account),
                Err(Unavailability::SessionExpired) => {}
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            }
        }
        ProviderUsage::unavailable(account, Unavailability::SessionExpired)
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

/// The stored header reduced to the cookies worth sending, or None when
/// nothing is left to send. What a browser store hands over or what somebody
/// pasted out of their network tab — which is why the `Cookie:` prefix is
/// tolerated and why each value is checked rather than trusted: a header
/// assembled from an arbitrary string is a header injection if a value
/// carries a newline.
pub fn normalize(input: &str) -> Option<String> {
    if input.chars().any(|c| (c as u32) < 32 || (c as u32) > 126) {
        return None;
    }
    let trimmed = input.trim();
    let header = if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("cookie:") {
        trimmed[7..].trim()
    } else {
        trimmed
    };
    if header.is_empty() || header.len() > 32_768 {
        return None;
    }

    let mut kept: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for pair in header.split(';') {
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() || value.is_empty() {
            continue;
        }
        let lowered = name.to_ascii_lowercase();
        if ANALYTICS.iter().any(|prefix| lowered.starts_with(prefix)) {
            continue;
        }
        // A cookie this cannot pass on unaltered is dropped rather than
        // costing the whole session: an odd preference cookie is not a reason
        // to refuse the sign-in beside it.
        if value.contains(' ') || value.contains('\\') || name.contains(' ') {
            continue;
        }
        // A host-only row and a domain row for one name is normal in every
        // browser store; the first wins, as it does in any cookie header.
        if seen.iter().any(|seen| seen == name) {
            continue;
        }
        seen.push(name.to_string());
        kept.push(format!("{name}={value}"));
    }
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}

// MARK: - Reading the reply

/// One pool of credits: what is used, out of how much, and Qoder's own
/// remainder when it states one. Not recomputed: absent says nothing, and a
/// zero read from silence would mark a pool spent.
#[derive(Debug, Clone, PartialEq)]
pub struct Pool {
    pub used: f64,
    pub limit: f64,
    pub remaining: Option<f64>,
}

/// A part of the personal total with an end date of its own — on the one
/// reply seen, a bonus pack of 100 credits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pack {
    pub remaining: f64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// The account's own credits: plan plus resource packs.
    pub personal: Pool,
    /// A team's shared pool. Absent off a team plan, and absent when Qoder
    /// reports an empty placeholder — a pool of zero is not one anybody can
    /// spend.
    pub shared: Option<Pool>,
    pub resets_at: Option<i64>,
    pub packs: Vec<Pack>,
}

pub fn parse(reply: &Value) -> Result<Snapshot, Unavailability> {
    let total = container(reply, "totalQuota", "total_quota")?;
    let personal = total
        .as_ref()
        .and_then(|c| either(c, "quotaSummary", "quota_summary"))
        .and_then(pool)
        .ok_or(Unavailability::UnreadableReply)?;
    let packs = total
        .as_ref()
        .and_then(|c| either(c, "quotaDetail", "quota_detail"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(pack).collect::<Vec<_>>())
        .unwrap_or_default();
    let shared = match container(reply, "sharedQuota", "shared_quota")? {
        // An absent team pool is normal; an unreadable one is not proof that
        // no allowance remains. Only a valid zero pool is omitted.
        Some(c) => match either(c, "quotaSummary", "quota_summary").and_then(pool) {
            Some(pool) if pool.limit > 0.0 => Some(pool),
            Some(_) => None,
            None => return Err(Unavailability::UnreadableReply),
        },
        None => None,
    };
    let resets_at = either(reply, "nextResetAt", "next_reset_at").and_then(date);
    Ok(Snapshot {
        personal,
        shared,
        resets_at,
        packs,
    })
}

/// The container under its camelCase or snake_case name. A value of the wrong
/// type under either is a reply this cannot read.
fn container<'a>(
    reply: &'a Value,
    camel: &str,
    snake: &str,
) -> Result<Option<&'a Value>, Unavailability> {
    match either(reply, camel, snake) {
        None => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value)),
        Some(_) => Err(Unavailability::UnreadableReply),
    }
}

/// The camelCase key, else the snake_case one. A null under either is an
/// absent one.
fn either<'a>(value: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    [camel, snake]
        .iter()
        .find_map(|key| value.get(*key).filter(|found| !found.is_null()))
}

/// A summary Qoder stated in full, or None. Negative figures are not a
/// reading anybody could have, so they are refused rather than clamped. The
/// figures arrive as JSON numbers — a string under the key is a reply this
/// cannot read.
fn pool(summary: &Value) -> Option<Pool> {
    let used = figure(either(summary, "usedValue", "used_value")?).filter(|used| *used >= 0.0)?;
    let limit =
        figure(either(summary, "limitValue", "limit_value")?).filter(|limit| *limit >= 0.0)?;
    let remaining = either(summary, "remainingValue", "remaining_value")
        .and_then(|value| figure(value))
        .filter(|remaining| *remaining >= 0.0);
    Some(Pool {
        used,
        limit,
        remaining,
    })
}

/// One pack out of the personal total's detail list. Only ones with credits
/// left and a date stated; a plan's entry carries `expires_at: 0`, and an
/// entry this cannot read is left out rather than allowed to cost the
/// summary.
fn pack(detail: &Value) -> Option<Pack> {
    let active = either(detail, "isActive", "is_active");
    if active == Some(&Value::Bool(false)) {
        return None;
    }
    let remaining =
        figure(either(detail, "remainingValue", "remaining_value")?).filter(|left| *left > 0.0)?;
    let expires_at = either(detail, "expiresAt", "expires_at").and_then(date)?;
    Some(Pack {
        remaining,
        expires_at,
    })
}

fn figure(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

/// ISO 8601 text, or a Unix stamp in seconds or milliseconds. Zero and
/// garbage are no date: a reset drawn at 1970 is a countdown that ran out
/// before anybody signed up.
pub fn date(value: &Value) -> Option<i64> {
    match value {
        Value::String(text) => crate::timeutil::parse_iso8601_ms(text),
        Value::Number(_) => {
            let stamp = figure(value).filter(|stamp| *stamp > 0.0)?;
            Some(if stamp > 10_000_000_000.0 {
                stamp as i64
            } else {
                (stamp * 1000.0) as i64
            })
        }
        _ => None,
    }
}

// MARK: - The windows

/// The account's credits, and the team's pool beside them when there is one.
pub fn windows(snapshot: &Snapshot, now_ms: i64) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    // A reset already behind `now` is no reset: a mainland trial account was
    // seen answering with a `nextResetAt` a month in the past beside credits
    // it could still spend (upstream issue #59). The credits are real; the
    // date is not, so the ring is drawn without one.
    let resets_at = snapshot.resets_at.filter(|at| *at > now_ms);
    if let Some(mut window) = window(&snapshot.personal, "qoder.credits", resets_at) {
        window.next_expiry_ms = next_expiry(&snapshot.packs, now_ms);
        windows.push(window);
    }
    // The reset Qoder states is the account's. Whether a team's pool turns
    // over on the same day is not something the reply says, so it is not
    // claimed for it.
    if let Some(shared) = &snapshot.shared {
        if let Some(window) = window(shared, "qoder.shared", None) {
            windows.push(window);
        }
    }
    windows
}

fn window(pool: &Pool, id: &str, resets_at: Option<i64>) -> Option<UsageWindow> {
    // A pool with a limit of zero is not drawn: there is no allowance to
    // divide by, and a ring at 100% would say something was spent that was
    // never granted.
    if pool.limit <= 0.0 {
        return None;
    }
    let mut window = UsageWindow::new(
        id,
        // Upstream kinds these `.credits` and `.sharedCredits`; the Windows
        // kind set has neither, so Spend stands in and the id says which is
        // which.
        Kind::Spend,
        None,
        (pool.used / pool.limit).clamp(0.0, 1.0),
        // Thirty days is a sort key, not a reported length: Qoder states when
        // the credits reset and never how long the period is — a trial runs a
        // fortnight, a plan a billing month.
        30 * 86_400,
        resets_at,
    );
    window.reports_length = false;
    window.is_exhausted = pool
        .remaining
        .map(|remaining| remaining <= 0.0)
        .unwrap_or(pool.used >= pool.limit);
    Some(window)
}

/// The soonest pack to lapse, among the ones still ahead of `now` with
/// something left in them.
fn next_expiry(packs: &[Pack], now_ms: i64) -> Option<i64> {
    packs
        .iter()
        .map(|pack| pack.expires_at)
        .filter(|at| *at > now_ms)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Before every reset in the fixtures.
    const NOW_MS: i64 = 1_700_000_000_000;

    /// The account page's own reply shape, with a personal total that carries
    /// a bonus pack, and a team's pool beside it.
    fn fixture() -> Value {
        json!({
            "quotaKey": "big_model_credits", "status": "active",
            "nextResetAt": "2024-09-01T00:00:00Z",
            "totalQuota": {
                "quotaSummary": {
                    "usedValue": 125, "limitValue": 500,
                    "remainingValue": 375, "usagePercentage": 25, "unit": "credit"
                },
                "quotaDetail": [
                    {"remainingValue": 100, "expiresAt": "2024-08-15T00:00:00Z", "isActive": true},
                    {"remainingValue": 40, "expiresAt": 1_700_010_000_000i64, "isActive": true},
                    {"remainingValue": 0, "expiresAt": "2024-08-20T00:00:00Z"},
                    {"remainingValue": 15, "expiresAt": "2024-09-05T00:00:00Z", "isActive": false}
                ]
            },
            "sharedQuota": {
                "quotaSummary": {"usedValue": 10, "limitValue": 1000, "remainingValue": 990}
            }
        })
    }

    #[test]
    fn the_personal_and_shared_pools_are_two_rings_never_one_sum() {
        let windows = windows(&parse(&fixture()).unwrap(), NOW_MS);
        assert_eq!(
            windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["qoder.credits", "qoder.shared"]
        );

        let personal = &windows[0];
        // Qoder's usedValue over its limitValue, not its usagePercentage —
        // the panel rounds for itself and would otherwise round a rounding.
        assert_eq!(personal.used_fraction, 0.25);
        assert_eq!(personal.kind, Kind::Spend);
        // Thirty days is a sort key; the period's length is never stated.
        assert_eq!(personal.window_seconds, 30 * 86_400);
        assert!(!personal.reports_length);
        assert_eq!(
            personal.resets_at,
            crate::timeutil::parse_iso8601_ms("2024-09-01T00:00:00Z")
        );
        assert!(!personal.is_exhausted);
        // The soonest pack to lapse: the 40-credit pack's stamp beats the
        // 100-credit one, and the spent and inactive packs say nothing.
        assert_eq!(personal.next_expiry_ms, Some(1_700_010_000_000));

        let shared = &windows[1];
        assert_eq!(shared.used_fraction, 0.01);
        // The reset Qoder states is the account's, not the team pool's.
        assert_eq!(shared.resets_at, None);
    }

    /// The mainland site's reply, which mixes the snake_case spellings in.
    #[test]
    fn the_mainland_spellings_are_read_the_same_way() {
        let reply = json!({
            "next_reset_at": "2024-09-01T00:00:00Z",
            "total_quota": {"quota_summary": {"used_value": 125, "limit_value": 500}},
            "shared_quota": {"quota_summary": {"used_value": 0, "limit_value": 0}}
        });
        let windows = windows(&parse(&reply).unwrap(), NOW_MS);
        // The personal ring reads; the shared pool of zero is an empty
        // placeholder, not an allowance, and is omitted.
        assert_eq!(
            windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["qoder.credits"]
        );
        // No remainder stated: spent is Qoder's own used against its limit.
        assert!(!windows[0].is_exhausted);
    }

    #[test]
    fn an_unreadable_reply_is_named_for_what_is_unreadable() {
        // The personal pool is the one the whole reading stands on.
        assert_eq!(
            parse(&json!({"sharedQuota": {"quotaSummary": {"usedValue": 1, "limitValue": 5}}})),
            Err(Unavailability::UnreadableReply)
        );
        assert_eq!(
            parse(
                &json!({"totalQuota": {"quotaSummary": {"usedValue": "125", "limitValue": 500}}})
            ),
            Err(Unavailability::UnreadableReply)
        );
        // A shared pool that is there but unreadable is not proof that no
        // allowance remains.
        assert_eq!(
            parse(&json!({
                "totalQuota": {"quotaSummary": {"usedValue": 1, "limitValue": 5}},
                "sharedQuota": {"quotaSummary": {"limitValue": "0"}}
            })),
            Err(Unavailability::UnreadableReply)
        );
        assert_eq!(
            parse(&json!("not the reply")),
            Err(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn a_reset_already_gone_by_is_no_reset_and_a_zero_pool_is_no_ring() {
        let reply = json!({
            "nextResetAt": NOW_MS / 1000 - 60,
            "totalQuota": {"quotaSummary": {"usedValue": 0, "limitValue": 0}}
        });
        let snapshot = parse(&reply).unwrap();
        assert_eq!(windows(&snapshot, NOW_MS), Vec::new());
    }

    #[test]
    fn a_spent_or_stale_remainder_reads_as_spent() {
        let spent = json!({"totalQuota": {"quotaSummary": {
            "usedValue": 500, "limitValue": 500, "remainingValue": 0}}});
        let rings = windows(&parse(&spent).unwrap(), NOW_MS);
        assert!(rings[0].is_exhausted);

        // No remainder stated at all: Qoder's own figures decide.
        let unstated =
            json!({"totalQuota": {"quotaSummary": {"usedValue": 500, "limitValue": 500}}});
        let rings = windows(&parse(&unstated).unwrap(), NOW_MS);
        assert!(rings[0].is_exhausted);

        // A fraction runs past its ceiling only as far as the ring shows.
        let over = json!({"totalQuota": {"quotaSummary": {"usedValue": 600, "limitValue": 500}}});
        assert_eq!(
            windows(&parse(&over).unwrap(), NOW_MS)[0].used_fraction,
            1.0
        );
    }

    #[test]
    fn a_date_is_iso_text_or_a_stamp_and_zero_is_no_date() {
        assert_eq!(
            date(&json!("2024-09-01T00:00:00Z")),
            crate::timeutil::parse_iso8601_ms("2024-09-01T00:00:00Z")
        );
        assert_eq!(
            date(&json!("2024-09-01T00:00:00.000Z")),
            date(&json!("2024-09-01T00:00:00Z"))
        );
        // Seconds and milliseconds, told apart by size.
        assert_eq!(date(&json!(1_700_000_000)), Some(NOW_MS));
        assert_eq!(date(&json!(1_700_000_000_000i64)), Some(NOW_MS));
        assert_eq!(
            date(&json!("2024-09-01T00:00:00+00:00")),
            date(&json!("2024-09-01T00:00:00Z"))
        );
        // Zero and garbage are no date.
        assert_eq!(date(&json!(0)), None);
        assert_eq!(date(&json!(-5)), None);
        assert_eq!(date(&json!("soon")), None);
        assert_eq!(date(&json!(true)), None);
        assert_eq!(date(&Value::Null), None);
    }

    #[test]
    fn the_stored_header_keeps_the_hosts_cookies_and_drops_the_analytics_ones() {
        // Alibaba's own are kept: the same family carries the bot screening.
        let header = normalize(
            "Cookie: _ga=GA1; ajs_group=1; cna=anon; tfstk=tf; session=abc; \
             session=again; empty=; spaced=a b; flag",
        )
        .expect("cookies are left");
        assert_eq!(header, "cna=anon; tfstk=tf; session=abc");

        // Control characters are an injection, not a session.
        assert_eq!(normalize("session=a\nb"), None);
        // Nothing but analytics is nothing to send.
        assert_eq!(normalize("_ga=GA1; _gid=G2"), None);
        assert_eq!(normalize("Cookie:"), None);
    }
}
