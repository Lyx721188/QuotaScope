//! LongCat's API platform: the account's token allowance, and the fuel packs
//! (加油包) bought on top of it, each reported as tokens used or left out of a
//! stated total.
//!
//! Read with a browser session from `longcat.chat`, from the routes the
//! platform's usage page calls: `GET /api/v1/user-current` to prove the
//! session, `POST /api/pay/quota/metering/token-packs/summary` for an active
//! token pack, `GET /api/lc-platform/v1/tokenUsage` for the allowance when
//! there is no such pack, and `GET /api/lc-platform/v1/pending-fuel-packages`
//! for the fuel packs. The shape is second-hand — taken from CodexBar's
//! LongCat provider and its tests, not from a captured reply.
//!
//! **Only figures the platform states.** The allowance needs a total and
//! either the tokens used or the tokens left; the fuel packs need their total
//! and at least one pack's tokens left. CodexBar fills a missing used figure
//! with zero and a missing remainder with the whole pack, which draws a full
//! ring nobody reported; here that row is left off. Neither states a length
//! or a reset, so neither claims one: the allowance is a credit allowance,
//! the packs a top-up whose soonest expiry is shown as an expiry, not a
//! reset.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

/// The session comes from `longcat.chat`. The Meituan passport token first,
/// which has to be there, and the account id beside it. Nothing else leaves
/// the browser.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["longcat.chat"],
    cookies: &["passport_token", "uid"],
};

const ORIGIN: &str = "https://longcat.chat";
const USER_PATH: &str = "/api/v1/user-current";
const TOKEN_PACKS_PATH: &str = "/api/pay/quota/metering/token-packs/summary";
const TOKEN_USAGE_PATH: &str = "/api/lc-platform/v1/tokenUsage";
const FUEL_PATH: &str = "/api/lc-platform/v1/pending-fuel-packages";

pub struct LongCatService {
    http: Arc<HttpClient>,
}

impl LongCatService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// The one call that has to succeed before anything is believed: a
    /// session the platform no longer takes answers here, not with zeros
    /// further on.
    fn ask(
        &self,
        path: &str,
        method: Method,
        cookie: &str,
    ) -> Result<serde_json::Value, Unavailability> {
        let headers = [
            ("Cookie", cookie.to_string()),
            ("Accept", "application/json, text/plain, */*".to_string()),
            ("Origin", ORIGIN.to_string()),
            ("Referer", format!("{ORIGIN}/platform/usage")),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let body = (method == Method::Post).then(|| serde_json::json!({}));
        let reply = session_json(
            &self.http,
            method,
            &format!("{ORIGIN}{path}"),
            &refs,
            body.as_ref(),
        )?;
        payload(&reply).cloned()
    }
}

impl ProviderService for LongCatService {
    fn provider(&self) -> Provider {
        Provider::LongCat
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::LongCat);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::LongCat)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        if let Err(reason) = self.ask(USER_PATH, Method::Get, &cookie) {
            return ProviderUsage::unavailable(account, reason);
        }

        // Best-effort: some sessions are not let into this route at all.
        let packs = self.ask(TOKEN_PACKS_PATH, Method::Post, &cookie).ok();

        let token_usage = if active_lot(packs.as_ref()).is_none() {
            match self.ask(TOKEN_USAGE_PATH, Method::Get, &cookie) {
                Ok(value) => Some(value),
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            }
        } else {
            None
        };

        // Best-effort, like the packs: the reading stands without them.
        let fuel = self.ask(FUEL_PATH, Method::Get, &cookie).ok();

        match reading(
            packs.as_ref(),
            token_usage.as_ref(),
            fuel.as_ref(),
            crate::timeutil::now_ms(),
        ) {
            Ok(windows) => {
                let mut usage = ProviderUsage::live_now(account, windows);
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// One call with the imported session as its credential. `fetch_json` folds
/// 401, 403 and the unfollowed redirect into `ApiKeyRefused`, which is the
/// pasted key's refusal — for a session those statuses are the session
/// expiring, so the folded reason is read here and remapped. A request that
/// sends no key can arrive at that folding no other way. 404 lands in
/// `NotFound`; upstream's classifier calls it a server error like any other.
fn session_json(
    http: &HttpClient,
    method: Method,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&serde_json::Value>,
) -> Result<serde_json::Value, Unavailability> {
    match http.fetch_json_detailed(method, url, headers, body) {
        Ok(value) => Ok(value),
        Err(HttpFailure::NotFound) => Err(Unavailability::ServerError),
        Err(HttpFailure::Unavailable(Unavailability::ApiKeyRefused)) => {
            Err(Unavailability::SessionExpired)
        }
        Err(HttpFailure::Unavailable(reason)) => Err(reason),
    }
}

/// A number, or a string that is one — never a boolean, and never a figure
/// that is not finite.
fn figure(value: Option<&serde_json::Value>) -> Option<f64> {
    crate::http::number(value?).filter(|number| number.is_finite())
}

/// The platform's envelope, `{ code, message, data }`: the `data` of a code
/// that is a success, or the reason there is none.
pub fn payload(reply: &serde_json::Value) -> Result<&serde_json::Value, Unavailability> {
    let Some(fields) = reply.as_object() else {
        return Err(Unavailability::UnreadableReply);
    };
    if let Some(raw) = fields.get("code") {
        let Some(code) = exact_int(raw) else {
            return Err(Unavailability::UnreadableReply);
        };
        match code {
            0 | 200 => {}
            401 | 403 => return Err(Unavailability::SessionExpired),
            _ => return Err(Unavailability::ServerError),
        }
    }
    match fields.get("data") {
        // A `data` that is not a dict is a reply that cannot be read.
        Some(data) => data
            .as_object()
            .map(|_| data)
            .ok_or(Unavailability::UnreadableReply),
        // No code and no `data`: the envelope is the payload itself.
        None => Ok(reply),
    }
}

/// The envelope's code as an exact integer — a fraction or an oversized
/// figure is a reply that cannot be read.
fn exact_int(value: &serde_json::Value) -> Option<i64> {
    let number = figure(Some(value))?;
    if number.fract() != 0.0 || number.abs() > 9_007_199_254_740_992.0 {
        return None;
    }
    Some(number as i64)
}

/// The windows, from the unwrapped `data` of each route. `None` for a route
/// not asked or not answered.
pub fn reading(
    packs: Option<&serde_json::Value>,
    token_usage: Option<&serde_json::Value>,
    fuel: Option<&serde_json::Value>,
    now_ms: i64,
) -> Result<Vec<UsageWindow>, Unavailability> {
    let mut windows = Vec::new();

    if let Some(lot) = active_lot(packs) {
        // An active pack without a consumed figure draws nothing here — and
        // leaves the older route unasked, exactly as upstream does.
        if let Some(used) = figure(lot.get("consumedToken")).filter(|used| *used >= 0.0) {
            if let Some(total) = figure(lot.get("totalToken")) {
                windows.push(allowance(used, total));
            }
        }
    } else if let Some(token_usage) = token_usage {
        // The account-wide figure; `extData` beside it is per model.
        let usage = token_usage
            .get("usage")
            .filter(|usage| usage.is_object())
            .unwrap_or(token_usage);
        // The route has to state a total at all; one that does not is not a
        // reply this reads, and saying "no limits" would hide that.
        let Some(total) = figure(usage.get("totalToken")) else {
            return Err(Unavailability::UnreadableReply);
        };
        if total > 0.0 {
            let used = figure(usage.get("usedToken"))
                .or_else(|| figure(usage.get("availableToken")).map(|available| total - available));
            if let Some(used) = used.filter(|used| *used >= 0.0) {
                windows.push(allowance(used, total));
            }
        }
    }

    if let Some(fuel) = fuel {
        if let Some(window) = fuel_window(fuel, now_ms) {
            windows.push(window);
        }
    }

    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    Ok(windows)
}

/// A token pack counts only while the platform calls it active and gives it
/// a size; an expired or empty one leaves the allowance to the older route,
/// which is what the platform's page does.
fn active_lot(packs: Option<&serde_json::Value>) -> Option<&serde_json::Value> {
    let lot = packs?.get("currentLot")?;
    let fields = lot.as_object()?;
    let status = fields
        .get("status")
        .and_then(|status| status.as_str())
        .map(|status| status.to_ascii_uppercase());
    if status.as_deref() != Some("ACTIVE") {
        return None;
    }
    figure(fields.get("totalToken")).filter(|total| *total > 0.0)?;
    Some(lot)
}

fn allowance(used: f64, total: f64) -> UsageWindow {
    let mut window = UsageWindow::new(
        "longcat.tokens",
        // Upstream kinds this `.credits` — an allowance with no length it
        // claims. The Windows kind set has no credits case; Spend is the
        // one kind that never claims a length either.
        Kind::Spend,
        None,
        used / total,
        30 * 86_400,
        None,
    );
    // The month is a sort key; no length or reset is stated.
    window.reports_length = false;
    window.is_exhausted = used >= total;
    window
}

/// The fuel packs as one top-up: their stated total, less what each pack
/// says it has left, with the soonest to lapse as the expiry.
fn fuel_window(fuel: &serde_json::Value, now_ms: i64) -> Option<UsageWindow> {
    let total = figure(fuel.get("totalQuota")).filter(|total| *total > 0.0)?;
    let packs: &[serde_json::Value] = match fuel.get("list").and_then(|list| list.as_array()) {
        Some(list) => list,
        None => &[],
    };
    let left: Vec<f64> = packs
        .iter()
        .filter_map(|pack| figure(pack.get("availableToken")))
        .filter(|amount| *amount >= 0.0)
        .collect();
    if left.is_empty() {
        return None;
    }
    let remaining: f64 = left.iter().sum();
    let used = (total - remaining).max(0.0);

    let mut window = UsageWindow::new(
        "longcat.fuel",
        // Upstream kinds this `.topUp`; the Windows kind set has no top-up
        // case, and Spend is the allowance stand-in the credits row uses.
        Kind::Spend,
        None,
        used / total,
        60 * 86_400,
        None,
    );
    // The sixty days are a sort key; no length or reset is stated.
    window.reports_length = false;
    window.is_exhausted = remaining <= 0.0;
    // The soonest part to lapse — only parts still ahead of now with
    // something left in them count. Upstream also sums the parts that end
    // on the same day; the Windows model carries the timestamp alone.
    window.next_expiry_ms = packs
        .iter()
        .filter_map(|pack| {
            let amount = figure(pack.get("availableToken"))?;
            let at = date(pack.get("expireTime").unwrap_or(&serde_json::Value::Null))?;
            Some((amount, at))
        })
        .filter(|(amount, at)| *amount > 0.0 && *at > now_ms)
        .map(|(_, at)| at)
        .min();
    Some(window)
}

/// Epoch seconds or milliseconds, ISO 8601, or `yyyy-MM-dd HH:mm:ss` — the
/// last read in the machine's own zone, as the platform's page reads it.
pub fn date(value: &serde_json::Value) -> Option<i64> {
    if let Some(epoch) = figure(Some(value)) {
        let seconds = if epoch > 1_000_000_000_000.0 {
            epoch / 1_000.0
        } else {
            epoch
        };
        return (seconds > 1_000_000_000.0).then(|| crate::timeutil::epoch_to_ms(seconds));
    }
    let text = value.as_str()?;
    crate::timeutil::parse_iso8601_ms(text).or_else(|| parse_local_naive(text))
}

fn parse_local_naive(text: &str) -> Option<i64> {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(
        chrono::Local
            .from_local_datetime(&naive)
            .single()?
            .timestamp_millis(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_code_decides_and_data_unwraps() {
        // A success unwraps its `data`.
        let ok = serde_json::json!({"code": 0, "data": {"totalToken": 5}});
        assert_eq!(
            payload(&ok)
                .unwrap()
                .get("totalToken")
                .and_then(|v| v.as_f64()),
            Some(5.0)
        );
        // The code can arrive as a string, and 200 is a success too.
        let text = serde_json::json!({"code": "200", "data": {"a": 1}});
        assert!(payload(&text).is_ok());
        // No code at all: the envelope is the payload.
        let bare = serde_json::json!({"totalToken": 5});
        assert_eq!(
            payload(&bare)
                .unwrap()
                .get("totalToken")
                .and_then(|v| v.as_f64()),
            Some(5.0)
        );
        // A refused session says so in the code, before any body matters.
        for (code, reason) in [
            (serde_json::json!(401), Unavailability::SessionExpired),
            (serde_json::json!(403), Unavailability::SessionExpired),
            (serde_json::json!(500), Unavailability::ServerError),
        ] {
            let reply = serde_json::json!({"code": code, "message": "no"});
            assert_eq!(payload(&reply), Err(reason));
        }
        // A fractional code, or a `data` that is not a dict, is unreadable.
        for reply in [
            serde_json::json!({"code": 200.5}),
            serde_json::json!({"code": 0, "data": "gone"}),
            serde_json::json!([]),
        ] {
            assert_eq!(payload(&reply), Err(Unavailability::UnreadableReply));
        }
    }

    #[test]
    fn an_active_pack_is_the_allowance() {
        let packs = serde_json::json!({
            "currentLot": {"status": "ACTIVE", "totalToken": 1000, "consumedToken": 400}
        });
        let windows = reading(Some(&packs), None, None, 0).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "longcat.tokens");
        assert_eq!(windows[0].used_fraction, 0.4);
        assert_eq!(windows[0].window_seconds, 30 * 86_400);
        assert!(!windows[0].reports_length);
        assert_eq!(windows[0].resets_at, None);
        assert!(!windows[0].is_exhausted);
    }

    #[test]
    fn an_active_pack_without_a_consumed_figure_draws_nothing() {
        let packs = serde_json::json!({
            "currentLot": {"status": "ACTIVE", "totalToken": 1000}
        });
        // Not even the older route is asked — matching upstream.
        assert_eq!(
            reading(Some(&packs), None, None, 0),
            Err(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn without_an_active_pack_the_older_route_counts() {
        let packs = serde_json::json!({
            "currentLot": {"status": "EXPIRED", "totalToken": 1000, "consumedToken": 900}
        });
        let usage = serde_json::json!({"usage": {"totalToken": 1000, "usedToken": 250}});
        let windows = reading(Some(&packs), Some(&usage), None, 0).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_fraction, 0.25);

        // The tokens left is the other shape the route states.
        let left = serde_json::json!({"totalToken": 1000, "availableToken": 750});
        let windows = reading(None, Some(&left), None, 0).unwrap();
        assert_eq!(windows[0].used_fraction, 0.25);

        // A used figure that would be negative is not a reading.
        let over = serde_json::json!({"totalToken": 100, "availableToken": 200});
        assert_eq!(
            reading(None, Some(&over), None, 0),
            Err(Unavailability::NoLimitsReported)
        );
        // A route that states no total is not a reply this reads.
        let no_total = serde_json::json!({"usage": {"usedToken": 1}});
        assert_eq!(
            reading(None, Some(&no_total), None, 0),
            Err(Unavailability::UnreadableReply)
        );
    }

    #[test]
    fn fuel_packs_are_one_top_up_with_the_soonest_expiry() {
        let fuel = serde_json::json!({
            "totalQuota": 100,
            "list": [
                {"availableToken": 20, "expireTime": 2_000_000_000_000i64},
                {"availableToken": 30, "expireTime": 3_000_000_000_000i64}
            ]
        });
        let windows = reading(None, None, Some(&fuel), 1_500_000_000_000).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "longcat.fuel");
        assert_eq!(windows[0].used_fraction, 0.5);
        assert_eq!(windows[0].window_seconds, 60 * 86_400);
        assert!(!windows[0].reports_length);
        assert!(!windows[0].is_exhausted);
        assert_eq!(windows[0].next_expiry_ms, Some(2_000_000_000_000));

        // A lapsed pack is no longer an expiry candidate.
        let lapsed = serde_json::json!({
            "totalQuota": 100,
            "list": [
                {"availableToken": 20, "expireTime": 1_000_000_000_000i64},
                {"availableToken": 30, "expireTime": 3_000_000_000_000i64}
            ]
        });
        let windows = reading(None, None, Some(&lapsed), 1_500_000_000_000).unwrap();
        assert_eq!(windows[0].next_expiry_ms, Some(3_000_000_000_000));

        // A top-up with nothing left is spent, and no expiry is claimed.
        let spent = serde_json::json!({
            "totalQuota": 100,
            "list": [{"availableToken": 0, "expireTime": 2_000_000_000_000i64}]
        });
        let windows = reading(None, None, Some(&spent), 1_500_000_000_000).unwrap();
        assert!(windows[0].is_exhausted);
        assert_eq!(windows[0].next_expiry_ms, None);
    }

    #[test]
    fn dates_read_as_epoch_iso_or_local_naive() {
        // Epoch milliseconds pass through; seconds scale up.
        assert_eq!(
            date(&serde_json::json!(1_759_296_000_000i64)),
            Some(1_759_296_000_000)
        );
        assert_eq!(
            date(&serde_json::json!(1_759_296_000i64)),
            Some(1_759_296_000_000)
        );
        assert_eq!(
            date(&serde_json::json!("2026-10-01T00:00:00Z")),
            crate::timeutil::parse_iso8601_ms("2026-10-01T00:00:00Z")
        );
        // The naive shape reads in the local zone — only its existence is
        // pinned here, not a clock the test cannot control.
        assert!(date(&serde_json::json!("2026-10-01 12:00:00")).is_some());
        // Too small to be a date, or not a date at all.
        assert_eq!(date(&serde_json::json!(5)), None);
        assert_eq!(date(&serde_json::json!(true)), None);
        assert_eq!(date(&serde_json::json!("soon")), None);
    }
}
