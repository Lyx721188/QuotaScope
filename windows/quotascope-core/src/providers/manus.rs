//! Manus, the agent: the plan's monthly credits, the credits that refresh on
//! a shorter clock, and the account's total credit balance. Each allowance
//! is a size and a remainder the service states.
//!
//! Read with the manus.im browser session the user imports: the `session_id`
//! cookie's value goes as a bearer token to
//! `POST https://api.manus.im/user.v1.UserService/GetAvailableCredits`, the
//! way CodexBar sends it. The shape is second-hand — taken from CodexBar's
//! Manus provider and its tests, not from a captured reply.
//!
//! A figure the reply leaves out is left out here, never read as zero: an
//! allowance with no remainder has no share used. The refresh's reset is
//! read only as an ISO 8601 date; its length only when the reply names the
//! interval as daily.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::{HttpClient, HttpFailure, Method};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

/// The session comes from `manus.im`; only the `session_id` cookie's value
/// is the credential, and it travels as a bearer token rather than a Cookie
/// header.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["manus.im"],
    cookies: &["session_id"],
};

const ENDPOINT: &str = "https://api.manus.im/user.v1.UserService/GetAvailableCredits";
/// The browser's name the site's own Connect call travels under — the one
/// request shape here that overrides the client's own agent.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";

pub struct ManusService {
    http: Arc<HttpClient>,
}

impl ManusService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for ManusService {
    fn provider(&self) -> Provider {
        Provider::Manus
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Manus);
        let Some(token) = pasted_or_none(keys.api_key(Provider::Manus))
            .as_deref()
            .and_then(session_token)
        else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };

        // The headers CodexBar sends: a Connect call with an empty body,
        // from the site's own origin, in a browser's name. Nothing else
        // about the session goes with it. The Content-Type for the body is
        // set by the shared fetch.
        let headers = [
            ("Authorization", format!("Bearer {token}")),
            ("Accept", "application/json".to_string()),
            ("Connect-Protocol-Version", "1".to_string()),
            ("Origin", "https://manus.im".to_string()),
            ("Referer", "https://manus.im/".to_string()),
            ("User-Agent", USER_AGENT.to_string()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();

        let reply = match session_json(
            &self.http,
            Method::Post,
            ENDPOINT,
            &refs,
            Some(&serde_json::json!({})),
        ) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        match reading(&reply) {
            Ok((windows, balance)) => {
                let mut usage = ProviderUsage::live_now(account, windows);
                usage.origin = Some(self.origin_token().into());
                usage.credit_balance = balance;
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// One call with the imported session as its credential. `fetch_json` folds
/// 401, 403 and the unfollowed redirect into `ApiKeyRefused`, which is the
/// pasted key's refusal — for a session those statuses are the session
/// expiring, so the folded reason is read here and remapped. 404 lands in
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

/// The `session_id` value out of the kept `name=value` header.
pub fn session_token(header: &str) -> Option<String> {
    header
        .split(';')
        .filter_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name.trim() == "session_id" && !value.is_empty()).then(|| value.to_string())
        })
        .next()
}

/// A count that arrives as a number or as a numeric string; anything else is
/// no figure.
fn figure(value: Option<&serde_json::Value>) -> Option<f64> {
    match value? {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

/// The figures the reply carries. `nextRefreshTime` and `refreshInterval`
/// are read only as strings.
struct Credits {
    total_credits: Option<f64>,
    periodic_credits: Option<f64>,
    pro_monthly_credits: Option<f64>,
    refresh_credits: Option<f64>,
    max_refresh_credits: Option<f64>,
    next_refresh_time: Option<String>,
    refresh_interval: Option<String>,
}

impl Credits {
    fn from_value(value: &serde_json::Value) -> Option<Credits> {
        let fields = value.as_object()?;
        Some(Credits {
            total_credits: figure(fields.get("totalCredits")),
            periodic_credits: figure(fields.get("periodicCredits")),
            pro_monthly_credits: figure(fields.get("proMonthlyCredits")),
            refresh_credits: figure(fields.get("refreshCredits")),
            max_refresh_credits: figure(fields.get("maxRefreshCredits")),
            next_refresh_time: fields
                .get("nextRefreshTime")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            refresh_interval: fields
                .get("refreshInterval")
                .and_then(|value| value.as_str())
                .map(str::to_string),
        })
    }

    fn has_any_figure(&self) -> bool {
        self.total_credits.is_some()
            || self.periodic_credits.is_some()
            || self.pro_monthly_credits.is_some()
            || self.refresh_credits.is_some()
            || self.max_refresh_credits.is_some()
    }
}

/// The credits object, bare or inside one of the envelopes Manus has used.
/// A named envelope field that arrives as anything but a dict spoils the
/// whole envelope, the way a decode failure would; the bare object is still
/// read.
fn credits_object(reply: &serde_json::Value) -> Option<Credits> {
    let envelope = ["data", "result", "response", "availableCredits"]
        .iter()
        .all(|key| match reply.get(key) {
            None | Some(serde_json::Value::Null) => true,
            Some(value) => value.is_object(),
        });
    let candidates: [Option<&serde_json::Value>; 5] = if envelope {
        [
            reply.get("data"),
            reply.get("result"),
            reply.get("response"),
            reply.get("availableCredits"),
            Some(reply),
        ]
    } else {
        [None, None, None, None, Some(reply)]
    };
    candidates
        .into_iter()
        .flatten()
        .filter_map(Credits::from_value)
        .find(Credits::has_any_figure)
}

/// The windows and the total balance. The balance stands alone when no
/// allowance is reported; a reply with neither is unreadable, and one with
/// neither figure is no limits.
pub fn reading(
    reply: &serde_json::Value,
) -> Result<(Vec<UsageWindow>, Option<String>), Unavailability> {
    let Some(credits) = credits_object(reply) else {
        return Err(Unavailability::UnreadableReply);
    };

    let mut windows = Vec::new();
    let daily = credits
        .refresh_interval
        .as_deref()
        .map(|interval| interval.to_ascii_lowercase().contains("daily"))
        .unwrap_or(false);
    if let Some(window) = allowance_window(
        "manus.refresh",
        // Upstream kinds a daily refresh `.daily`; the Windows kind set has
        // no daily case, so the day rides as `other`, as elsewhere in the
        // port. Otherwise it is the credits allowance Spend stands in for.
        if daily {
            Kind::Other(86_400)
        } else {
            Kind::Spend
        },
        credits.max_refresh_credits,
        credits.refresh_credits,
        86_400,
        daily,
        credits
            .next_refresh_time
            .as_deref()
            .and_then(crate::timeutil::parse_iso8601_ms),
    ) {
        windows.push(window);
    }
    // The plan's monthly credits. No renewal date is reported, so no reset
    // is claimed and the month is a sort key.
    if let Some(window) = allowance_window(
        "manus.monthly",
        Kind::Monthly,
        credits.pro_monthly_credits,
        credits.periodic_credits,
        30 * 86_400,
        false,
        None,
    ) {
        windows.push(window);
    }

    let total = credits
        .total_credits
        .filter(|total| total.is_finite() && *total >= 0.0);
    if windows.is_empty() && total.is_none() {
        return Err(Unavailability::NoLimitsReported);
    }
    Ok((windows, total.map(count_text)))
}

/// Used is the size less what is left, both as reported. A size of zero is
/// no allowance.
fn allowance_window(
    id: &str,
    kind: Kind,
    size: Option<f64>,
    left: Option<f64>,
    seconds: i64,
    reports_length: bool,
    resets_at: Option<i64>,
) -> Option<UsageWindow> {
    let size = size.filter(|size| size.is_finite() && *size > 0.0)?;
    let left = left.filter(|left| left.is_finite() && *left >= 0.0)?;
    let mut window = UsageWindow::new(
        id,
        kind,
        None,
        (size - left).max(0.0) / size,
        seconds,
        resets_at,
    );
    window.reports_length = reports_length;
    window.is_exhausted = left <= 0.0;
    Some(window)
}

/// Manus's credits are its own unit, not money: the number alone, grouped
/// for the reader, as Swift's decimal formatter printed it.
pub fn count_text(value: f64) -> String {
    let whole = value.round();
    let digits = (whole.abs() as i128).to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if whole < 0.0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credits_json() -> serde_json::Value {
        serde_json::json!({
            "data": {
                "totalCredits": 1200,
                "periodicCredits": 300,
                "proMonthlyCredits": 900,
                "refreshCredits": 200,
                "maxRefreshCredits": 250,
                "nextRefreshTime": "2026-10-02T00:00:00Z",
                "refreshInterval": "daily"
            }
        })
    }

    #[test]
    fn a_daily_refresh_and_the_monthly_plan_with_the_balance() {
        let (windows, balance) = reading(&credits_json()).unwrap();
        assert_eq!(windows.len(), 2);

        let refresh = &windows[0];
        assert_eq!(refresh.id, "manus.refresh");
        // Upstream kinds a daily refresh `.daily`; the day rides as `other`.
        assert_eq!(refresh.kind, Kind::Other(86_400));
        assert_eq!(refresh.used_fraction, 50.0 / 250.0);
        assert!(refresh.reports_length);
        assert_eq!(
            refresh.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-02T00:00:00Z")
        );

        let monthly = &windows[1];
        assert_eq!(monthly.id, "manus.monthly");
        assert_eq!(monthly.kind, Kind::Monthly);
        assert_eq!(monthly.used_fraction, 600.0 / 900.0);
        // No renewal date is reported, so no reset and no stated length.
        assert!(!monthly.reports_length);
        assert_eq!(monthly.resets_at, None);

        assert_eq!(balance.as_deref(), Some("1,200"));
    }

    #[test]
    fn a_refresh_not_named_daily_claims_no_length() {
        let reply = serde_json::json!({"refreshCredits": 200, "maxRefreshCredits": 250, "refreshInterval": "weekly"});
        let (windows, _) = reading(&reply).unwrap();
        assert_eq!(windows[0].kind, Kind::Spend);
        assert!(!windows[0].reports_length);
        assert_eq!(windows[0].resets_at, None);
    }

    #[test]
    fn envelopes_are_tried_in_order_and_the_bare_object_last() {
        // The full figure set makes two windows.
        let (windows, _) = reading(&credits_json()).unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].used_fraction, 50.0 / 250.0);

        for reply in [
            serde_json::json!({"result": {"refreshCredits": 1, "maxRefreshCredits": 4}}),
            serde_json::json!({"response": {"refreshCredits": 1, "maxRefreshCredits": 4}}),
            serde_json::json!({"availableCredits": {"refreshCredits": 1, "maxRefreshCredits": 4}}),
            serde_json::json!({"refreshCredits": 1, "maxRefreshCredits": 4}),
        ] {
            let (windows, _) = reading(&reply).unwrap();
            assert_eq!(windows.len(), 1);
            assert_eq!(windows[0].used_fraction, 0.75);
        }
    }

    #[test]
    fn an_envelope_spoiled_by_a_wrong_shaped_field_falls_back_to_the_bare_object() {
        // `data` that is not a dict spoils the envelope decode, but the
        // bare root still carries a figure.
        let reply =
            serde_json::json!({"data": "gone", "refreshCredits": 1, "maxRefreshCredits": 4});
        let (windows, _) = reading(&reply).unwrap();
        assert_eq!(windows.len(), 1);

        // Counts can arrive as strings.
        let reply = serde_json::json!({"refreshCredits": "1", "maxRefreshCredits": "4"});
        let (windows, _) = reading(&reply).unwrap();
        assert_eq!(windows[0].used_fraction, 0.75);
    }

    #[test]
    fn a_balance_alone_stands_without_a_window() {
        let reply = serde_json::json!({"totalCredits": 500});
        let (windows, balance) = reading(&reply).unwrap();
        assert!(windows.is_empty());
        assert_eq!(balance.as_deref(), Some("500"));
    }

    #[test]
    fn an_envelope_with_no_figures_at_all_is_unreadable_not_empty() {
        // Upstream picks the first candidate that carries a figure; one that
        // carries none never reaches the limit check at all.
        for reply in [
            serde_json::json!({}),
            serde_json::json!({"data": {}}),
            serde_json::json!([]),
        ] {
            assert_eq!(
                reading(&reply).map(|_| ()),
                Err(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn a_size_of_zero_or_a_missing_remainder_is_no_window() {
        let reply = serde_json::json!({"refreshCredits": 200, "maxRefreshCredits": 0});
        assert_eq!(
            reading(&reply).map(|_| ()),
            Err(Unavailability::NoLimitsReported)
        );
        let reply = serde_json::json!({"maxRefreshCredits": 250});
        assert_eq!(
            reading(&reply).map(|_| ()),
            Err(Unavailability::NoLimitsReported)
        );
    }

    #[test]
    fn the_balance_is_grouped_and_never_shows_decimals() {
        assert_eq!(count_text(1_234_567.0), "1,234,567");
        assert_eq!(count_text(0.0), "0");
        assert_eq!(count_text(12.6), "13");
    }

    #[test]
    fn the_session_id_value_is_the_bearer_token() {
        assert_eq!(session_token("session_id=abc"), Some("abc".into()));
        assert_eq!(session_token("other=1; session_id=xyz"), Some("xyz".into()));
        // An empty value, a neighbour name, or no pair at all is no token.
        assert_eq!(session_token("session_id="), None);
        assert_eq!(session_token("sid=abc"), None);
        assert_eq!(session_token("no cookie here"), None);
    }
}
