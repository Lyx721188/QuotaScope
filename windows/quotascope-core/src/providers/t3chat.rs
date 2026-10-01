//! T3 Chat: the four-hour window and the monthly allowance, each reported as
//! a percentage by the service itself.
//!
//! Read with the browser session the user imports, from the call T3 Chat's
//! own settings page makes: `GET https://t3.chat/api/trpc/getCustomerData`.
//! The reply is tRPC's streamed JSON, one value per line, with the customer's
//! record somewhere inside one of them. The shape is second-hand — taken from
//! CodexBar's T3 Chat plugin and its tests, not from a captured reply — and
//! the fixture in the tests says so. **Which cookies carry the session is not
//! in CodexBar either** (it forwards every cookie for the site); the names
//! kept here are unverified.
//!
//! A percentage T3 Chat leaves out is left off, never drawn as zero — which
//! is what CodexBar does. `usagePeriodPercentage` is not read in place of the
//! monthly figure: nothing says which period it is.

use super::{pasted_or_none, KeyRing, ProviderService, SessionSpec};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::Value;
use std::sync::Arc;

/// Where the session lives, for the Settings import button.
pub const SESSION: SessionSpec = SessionSpec {
    hosts: &["t3.chat"],
    cookies: &["wos-session", "_vcrcs"],
};

/// The settings page's own call: one batch item, no session id, the input
/// percent-encoded the way the page's request carries it.
const ENDPOINT: &str = concat!(
    "https://t3.chat/api/trpc/getCustomerData",
    "?batch=1&input=%7B%220%22%3A%7B%22json%22%3A%7B%22sessionId%22%3Anull%7D%2C",
    "%22meta%22%3A%7B%22values%22%3A%7B%22sessionId%22%3A%5B%22undefined%22%5D%7D%7D%7D%7D",
);

pub struct T3ChatService {
    http: Arc<HttpClient>,
}

impl T3ChatService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }

    /// The settings page's call, its status read by hand: the shared fetch
    /// folds 401, 403 and the unfollowed redirect into the refused key,
    /// which is the wrong story for a browser session — those all mean the
    /// session no longer works.
    fn get(&self, cookie: &str) -> Result<String, Unavailability> {
        let headers = [
            ("Cookie", cookie.to_string()),
            ("Accept", "*/*".to_string()),
            ("Origin", "https://t3.chat".to_string()),
            (
                "Referer",
                "https://t3.chat/settings/customization".to_string(),
            ),
            // tRPC answers the streamed form only when asked in its own
            // terms.
            ("trpc-accept", "application/jsonl".to_string()),
            ("x-trpc-source", "web-client".to_string()),
            ("x-trpc-batch", "true".to_string()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut request = self
            .http
            .client_for_login()
            .get(ENDPOINT)
            .timeout(std::time::Duration::from_secs(15));
        for (name, value) in refs {
            request = request.header(name, value);
        }
        let response = request.send().map_err(|_| Unavailability::Unreachable)?;
        let status = response.status().as_u16();
        let text = response
            .text()
            .map_err(|_| Unavailability::UnreadableReply)?;
        match status {
            200..=299 => Ok(text),
            300..=399 | 401 | 403 => Err(Unavailability::SessionExpired),
            429 => Err(Unavailability::RateLimited),
            _ => Err(Unavailability::ServerError),
        }
    }
}

impl ProviderService for T3ChatService {
    fn provider(&self) -> Provider {
        Provider::T3Chat
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::T3Chat);
        let Some(cookie) = pasted_or_none(keys.api_key(Provider::T3Chat)) else {
            return ProviderUsage::unavailable(account, Unavailability::SessionMissing);
        };
        let text = match self.get(&cookie) {
            Ok(text) => text,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let usage = reading(&text, account);
        if matches!(usage.state, crate::model::State::Live) {
            return ProviderUsage {
                origin: Some(self.origin_token().into()),
                ..usage
            };
        }
        usage
    }
}

/// The reading out of the streamed reply: every line is a JSON value of its
/// own, and the customer's record sits somewhere inside one of them.
pub fn reading(text: &str, account: AccountKey) -> ProviderUsage {
    let Some(customer) = text
        .split(['\r', '\n'])
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|value| customer_record(&value).cloned())
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    let mut windows = Vec::new();

    // "Four hour" is in the field's name, so the length is stated.
    if let Some(percent) = percent(customer.get("usageFourHourPercentage")) {
        let mut window = UsageWindow::new(
            "t3chat.four_hour",
            Kind::Other(4 * 3_600),
            None,
            percent / 100.0,
            4 * 3_600,
            date(customer.get("usageFourHourNextResetAt"))
                .or_else(|| date(customer.get("usageWindowNextResetAt"))),
        );
        window.is_exhausted = percent >= 100.0;
        windows.push(window);
    }

    // The subscription's billing period: a name and a sort key, with the
    // reset the subscription states. `billingNextResetAt` is the usage
    // window's, not this one's, and is not used for it.
    if let Some(percent) = percent(customer.get("usageMonthPercentage")) {
        let mut window = UsageWindow::new(
            "t3chat.monthly",
            Kind::Monthly,
            None,
            percent / 100.0,
            30 * 86_400,
            customer
                .get("subscription")
                .and_then(|subscription| date(subscription.get("currentPeriodEnd"))),
        );
        window.reports_length = false;
        window.is_exhausted = percent >= 100.0;
        windows.push(window);
    }

    if windows.is_empty() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.plan = plan(&customer);
    usage
}

/// The first object anywhere in `value` that is the customer's record.
/// Objects are searched depth-first; the record is unique in a reply, so the
/// order the map hands its children out decides nothing.
pub fn customer_record(value: &Value) -> Option<&Value> {
    if let Some(object) = value.as_object() {
        if is_customer_record(object) {
            return Some(value);
        }
        for child in object.values() {
            if let Some(found) = customer_record(child) {
                return Some(found);
            }
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            if let Some(found) = customer_record(child) {
                return Some(found);
            }
        }
    }
    None
}

/// The marks of the customer's record: either usage percentage, or the
/// subscription beside the usage band.
fn is_customer_record(object: &serde_json::Map<String, Value>) -> bool {
    object.contains_key("usageFourHourPercentage")
        || object.contains_key("usageMonthPercentage")
        || (object.contains_key("subscription") && object.contains_key("usageBand"))
}

/// A percentage: a finite number, never a boolean, and never a string — a
/// figure that arrived quoted is no figure. Negative ones are left off too.
fn percent(value: Option<&Value>) -> Option<f64> {
    value?
        .as_f64()
        .filter(|percent| percent.is_finite() && *percent >= 0.0)
}

/// Epoch milliseconds, or seconds when the figure is too small to be
/// milliseconds — T3 Chat uses both.
fn date(value: Option<&Value>) -> Option<i64> {
    let raw = value?
        .as_f64()
        .filter(|raw| raw.is_finite() && *raw > 0.0)?;
    Some(if raw > 10_000_000_000.0 {
        raw as i64
    } else {
        (raw * 1000.0) as i64
    })
}

/// "pro" → "Pro", "pro-max" → "Pro Max". T3 Chat's own plan name.
pub fn plan(customer: &Value) -> Option<String> {
    let raw = customer
        .get("subscription")
        .and_then(|subscription| subscription.get("productName"))
        .and_then(Value::as_str)
        .or_else(|| customer.get("subTier").and_then(Value::as_str))?
        .trim();
    if raw.is_empty() {
        return None;
    }
    Some(
        raw.split('-')
            .filter(|part| !part.is_empty())
            .map(titlecase)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The first letter up, the rest as they came.
fn titlecase(part: &str) -> String {
    let mut chars = part.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;
    use serde_json::json;

    fn account() -> AccountKey {
        AccountKey::primary(Provider::T3Chat)
    }

    /// The shape pinned by CodexBar's T3 Chat plugin — second-hand, not
    /// captured from a live account — as one line of the stream.
    const LINE: &str = concat!(
        r#"{"0":{"result":{"data":{"json":{"usageFourHourPercentage":12,"#,
        r#""usageFourHourNextResetAt":1782000000000,"usageWindowNextResetAt":1781000000000,"#,
        r#""usageMonthPercentage":34,"billingNextResetAt":1782500000000,"usageBand":"5","#,
        r#""subscription":{"productName":"pro-max","currentPeriodEnd":1783000000}}}}}}"#
    );

    #[test]
    fn both_windows_are_read_from_the_stream_line() {
        let usage = reading(LINE, account());
        assert!(matches!(usage.state, State::Live));
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            vec!["t3chat.four_hour", "t3chat.monthly"]
        );
        let four_hour = &usage.windows[0];
        assert_eq!(four_hour.kind, Kind::Other(4 * 3_600));
        assert_eq!(four_hour.used_fraction, 0.12);
        assert_eq!(four_hour.window_seconds, 4 * 3_600);
        // The length is stated — "Four hour" is in the field's name.
        assert!(four_hour.reports_length);
        assert_eq!(four_hour.resets_at, Some(1_782_000_000_000));
        let monthly = &usage.windows[1];
        assert_eq!(monthly.kind, Kind::Monthly);
        assert_eq!(monthly.used_fraction, 0.34);
        // The subscription states its own end, in seconds here.
        assert_eq!(monthly.resets_at, Some(1_783_000_000_000));
        // A billing period is only as long as the month it falls in.
        assert!(!monthly.reports_length);
        assert_eq!(usage.plan.as_deref(), Some("Pro Max"));
    }

    #[test]
    fn the_monthly_reset_never_borrows_the_usage_windows_reset() {
        // `billingNextResetAt` sits beside the monthly figure but names the
        // four-hour window's clock; only the subscription's period end is
        // this one's.
        let reply = r#"{"usageMonthPercentage":10,"billingNextResetAt":1782500000000,"subscription":{"currentPeriodEnd":1783000000}}"#;
        let usage = reading(reply, account());
        assert_eq!(usage.windows[0].resets_at, Some(1_783_000_000_000));
    }

    #[test]
    fn the_four_hour_reset_falls_back_to_the_usage_window_when_its_own_is_missing() {
        let reply = r#"{"usageFourHourPercentage":5,"usageWindowNextResetAt":1781000000}"#;
        let usage = reading(reply, account());
        assert_eq!(usage.windows[0].resets_at, Some(1_781_000_000_000));
    }

    #[test]
    fn a_percentage_left_out_or_unusable_is_left_off_never_zero() {
        for reply in [
            r#"{"usageFourHourPercentage":-1}"#,
            r#"{"usageFourHourPercentage":true}"#,
            r#"{"usageFourHourPercentage":"12"}"#,
            r#"{"usageMonthPercentage":null}"#,
        ] {
            let usage = reading(reply, account());
            assert!(usage.windows.is_empty(), "{reply}");
            assert_eq!(
                usage.state,
                State::Unavailable(Unavailability::NoLimitsReported)
            );
        }
    }

    #[test]
    fn a_reply_with_no_customer_record_anywhere_is_unreadable() {
        for text in [
            "",
            "not json at all",
            "{\"0\":{\"result\":{\"data\":{\"json\":{\"other\":1}}}}}",
            "[1,2]",
        ] {
            assert_eq!(
                reading(text, account()).state,
                State::Unavailable(Unavailability::UnreadableReply)
            );
        }
    }

    #[test]
    fn the_record_is_found_through_lines_envelopes_and_arrays() {
        // A preamble line that parses but carries nothing, then the record.
        let text = "{\"0\":{\"result\":{\"data\":{\"json\":null}}}}\n{\"1\":{\"result\":{\"data\":{\"json\":{\"usageMonthPercentage\":3}}}}}";
        let usage = reading(text, account());
        assert_eq!(usage.windows[0].used_fraction, 0.03);

        // The subscription-and-band pair names a record with no percentages.
        let named = json!({"a":[{"b":{"subscription":{},"usageBand":"5"}}]});
        assert!(customer_record(&named).is_some());
        let unnamed = json!({"a":[{"b":{"subscription":{}}}]});
        assert_eq!(customer_record(&unnamed), None);
    }

    #[test]
    fn dates_arrive_as_milliseconds_or_seconds_and_never_as_zero() {
        assert_eq!(
            date(Some(&json!(1_782_000_000_123i64))),
            Some(1_782_000_000_123)
        );
        assert_eq!(date(Some(&json!(1_783_000_000))), Some(1_783_000_000_000));
        assert_eq!(date(Some(&json!(0))), None);
        assert_eq!(date(Some(&json!(-5))), None);
        assert_eq!(date(Some(&json!("1783000000"))), None);
        assert_eq!(date(Some(&json!(true))), None);
        assert_eq!(date(None), None);
    }

    #[test]
    fn the_plan_comes_from_the_subscription_or_the_sub_tier() {
        assert_eq!(
            plan(&json!({"subscription": {"productName": "pro-max"}})).as_deref(),
            Some("Pro Max")
        );
        assert_eq!(plan(&json!({"subTier": "basic"})).as_deref(), Some("Basic"));
        // The subscription's name wins where both are given.
        assert_eq!(
            plan(&json!({"subscription": {"productName": "pro"}, "subTier": "basic"})).as_deref(),
            Some("Pro")
        );
        assert_eq!(plan(&json!({"subscription": {"productName": "  "}})), None);
        assert_eq!(plan(&json!({})), None);
        // Rest as they came: an already-capitalised name is not re-cased.
        assert_eq!(plan(&json!({"subTier": "PRO-X"})).as_deref(), Some("PRO X"));
    }
}
