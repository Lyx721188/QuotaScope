//! Chutes' subscription windows, read with a key from `GET
//! https://api.chutes.ai/users/me/subscription_usage`.
//!
//! **Narrower than CodexBar on purpose.** CodexBar searches the whole reply
//! for anything with a `limit` and a `used`, guesses a lane from the word
//! "rolling" or "month", fills in four hours or thirty days where no length
//! is given, and reads a percentage under 1 as a fraction. None of that is
//! done here: the rolling window is read only where its length is stated —
//! in minutes, hours or seconds, or in its name — and the monthly one only
//! where it is named so. The per-chute pay-as-you-go quotas CodexBar fetches
//! one request at a time state no period at all, and are left off.
//!
//! The reply's shape is second-hand (taken from CodexBar's Chutes plugin and
//! its tests, not from a captured reply), and keys arrive under several
//! spellings, so every field name is matched loosely — lower-cased, with
//! everything but letters and digits dropped.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, State, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.chutes.ai/users/me/subscription_usage";

/// Field names compared the way CodexBar compares them: lower-cased, with
/// everything but letters and digits dropped, so `rolling_window`,
/// `rollingWindow` and `RollingWindow` are one name.
const MONTHLY_KEYS: [&str; 3] = ["monthly", "monthlyusage", "billingperiod"];
const PLAN_KEYS: [&str; 3] = ["planname", "plan", "tier"];
const LIMIT_KEYS: [&str; 7] = [
    "limit",
    "cap",
    "max",
    "quota",
    "monthlylimit",
    "requestlimit",
    "tokenlimit",
];
const USED_KEYS: [&str; 7] = [
    "used",
    "usage",
    "consumed",
    "requests",
    "requestcount",
    "tokens",
    "tokenusage",
];
const REMAINING_KEYS: [&str; 3] = ["remaining", "available", "left"];
const PERCENT_USED_KEYS: [&str; 3] = ["percentused", "usagepercent", "usedpercent"];
const RESET_KEYS: [&str; 7] = [
    "resetat",
    "resetsat",
    "nextresetat",
    "renewsat",
    "periodend",
    "currentperiodend",
    "windowend",
];

pub struct ChutesService {
    http: Arc<HttpClient>,
}

impl ChutesService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for ChutesService {
    fn provider(&self) -> Provider {
        Provider::Chutes
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Chutes);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Chutes)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self
            .http
            .fetch_json(crate::http::Method::Get, ENDPOINT, &refs, None)
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = reading(&reply, account);
        if matches!(usage.state, State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

/// The rolling lane, and how long each name says it is, where it says. `None`
/// means "a rolling name with no stated length", which is distinct from "not
/// a rolling name at all".
fn rolling_named(name: &str) -> Option<Option<i64>> {
    match name {
        "rolling" | "rollingwindow" => Some(None),
        "rolling4h" | "fourhour" | "fourhourusage" | "window4h" => Some(Some(4 * 3_600)),
        _ => None,
    }
}

fn normalized(key: &str) -> String {
    key.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// The first of the named keys present under any spelling, skipping a null
/// on to the next name. Names are given already normalized.
fn value<'a>(object: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    let map = object.as_object()?;
    for key in keys {
        if let Some((_, entry)) = map.iter().find(|(name, _)| normalized(name) == *key) {
            if !entry.is_null() {
                return Some(entry);
            }
        }
    }
    None
}

/// A number, or a numeric string — the two shapes the reply uses. Text that
/// parses to something no limit could be ("inf") is no figure at all.
fn number(value: Option<&serde_json::Value>) -> Option<f64> {
    let parsed = match value? {
        serde_json::Value::Number(value) => value.as_f64()?,
        serde_json::Value::String(text) => text.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    parsed.is_finite().then_some(parsed)
}

/// ISO 8601, or epoch seconds or milliseconds — the three shapes the reply
/// resets in. Upstream's seconds/millisecond threshold is 1e10, stricter
/// than the shared helper's, so this keeps its own arithmetic.
fn date(value: Option<&serde_json::Value>) -> Option<i64> {
    if let Some(serde_json::Value::String(text)) = value {
        if let Some(ms) = crate::timeutil::parse_iso8601_ms(text) {
            return Some(ms);
        }
    }
    let stamp = number(value).filter(|stamp| *stamp > 0.0)?;
    Some(if stamp > 10_000_000_000.0 {
        stamp as i64
    } else {
        (stamp * 1000.0) as i64
    })
}

/// A length the payload states in so many minutes, hours or seconds. The
/// first unit it names decides: an out-of-range figure is no length at all,
/// not a fall-through to a smaller unit.
fn stated_seconds(payload: &serde_json::Value) -> Option<i64> {
    const UNITS: [(&[&str], f64); 3] = [
        (&["windowminutes", "periodminutes", "durationminutes"], 60.0),
        (&["windowhours", "periodhours", "durationhours"], 3_600.0),
        (&["windowseconds", "periodseconds", "durationseconds"], 1.0),
    ];
    for (keys, multiplier) in UNITS {
        if let Some(stated) = number(value(payload, keys)) {
            if stated > 0.0 {
                let seconds = (stated * multiplier).round();
                return if (3_600.0..f64::from(i32::MAX)).contains(&seconds) {
                    Some(seconds as i64)
                } else {
                    None
                };
            }
        }
    }
    None
}

/// The rolling lane whose length is stated — by the payload itself or by its
/// name — or none. A lane that states none is passed over, so a bare
/// `rolling_window` never hides a `four_hour` beside it.
fn rolling(body: &serde_json::Value) -> Option<(&serde_json::Value, i64)> {
    let map = body.as_object()?;
    let mut entries: Vec<(&str, &serde_json::Value)> =
        map.iter().map(|(k, v)| (k.as_str(), v)).collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (key, entry) in entries {
        let Some(named) = rolling_named(&normalized(key)) else {
            continue;
        };
        if !entry.is_object() {
            continue;
        }
        let Some(seconds) = stated_seconds(entry).or(named) else {
            continue;
        };
        return Some((entry, seconds));
    }
    None
}

/// Used and limit, both as the service states them — or a percentage it
/// states, on a 0–100 scale. A limit of zero is no allowance.
fn figures(payload: &serde_json::Value) -> Option<(f64, f64)> {
    if let Some(percent) = number(value(payload, &PERCENT_USED_KEYS)) {
        if percent >= 0.0 {
            return Some((percent, 100.0));
        }
    }
    let limit = number(value(payload, &LIMIT_KEYS)).filter(|limit| *limit > 0.0)?;
    if let Some(used) = number(value(payload, &USED_KEYS)) {
        if used >= 0.0 {
            return Some((used, limit));
        }
    }
    if let Some(remaining) = number(value(payload, &REMAINING_KEYS)) {
        if remaining >= 0.0 {
            return Some(((limit - remaining).max(0.0), limit));
        }
    }
    None
}

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    if reply.as_object().is_none() {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }
    let empty = serde_json::Value::Object(serde_json::Map::new());
    // Some replies wrap everything in `data` or `result`.
    let body = value(reply, &["data", "result"])
        .filter(|body| body.is_object())
        .unwrap_or(reply);
    let subscription = value(body, &["subscription", "currentsubscription"])
        .filter(|subscription| subscription.is_object())
        .unwrap_or(&empty);

    let mut windows: Vec<UsageWindow> = Vec::new();

    if let Some((payload, seconds)) = rolling(body) {
        if let Some((used, limit)) = figures(payload) {
            // Upstream names a stated day its own `.daily` kind; the Windows
            // Kind has no daily case, and the day as `other` is the stand-in
            // the crate already uses for one.
            let kind = if seconds == 5 * 3_600 {
                Kind::FiveHour
            } else {
                Kind::Other(seconds)
            };
            let mut window = UsageWindow::new(
                "chutes.rolling",
                kind,
                None,
                used / limit,
                seconds,
                date(value(payload, &RESET_KEYS)),
            );
            window.is_exhausted = used >= limit;
            windows.push(window);
        }
    }

    if let Some(payload) = value(body, &MONTHLY_KEYS).filter(|payload| payload.is_object()) {
        if let Some((used, limit)) = figures(payload) {
            let mut window = UsageWindow::new(
                "chutes.monthly",
                Kind::Monthly,
                None,
                used / limit,
                30 * 86_400,
                date(value(payload, &RESET_KEYS)),
            );
            // A billing month: a sort key, not a stated length.
            window.reports_length = false;
            window.is_exhausted = used >= limit;
            windows.push(window);
        }
    }

    if windows.is_empty() {
        let flag = value(subscription, &["active", "isactive"]).and_then(|v| v.as_bool());
        let status = value(subscription, &["status", "state"])
            .and_then(|v| v.as_str())
            .map(str::to_ascii_lowercase);
        let inactive = flag == Some(false)
            || flag.is_none()
                && status.as_deref().is_some_and(|s| {
                    [
                        "free",
                        "inactive",
                        "canceled",
                        "cancelled",
                        "expired",
                        "none",
                    ]
                    .contains(&s)
                });
        return ProviderUsage::unavailable(
            account,
            if inactive {
                Unavailability::NoPlan
            } else {
                Unavailability::NoLimitsReported
            },
        );
    }
    windows.sort_by_key(|window| window.window_seconds);
    let plan = value(subscription, &PLAN_KEYS)
        .and_then(|plan| plan.as_str())
        .map(|plan| plan.trim().to_string())
        .filter(|plan| !plan.is_empty());
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.plan = plan;
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::Chutes))
    }

    fn live_windows(usage: &ProviderUsage) -> &[UsageWindow] {
        match &usage.state {
            State::Live => &usage.windows,
            other => panic!("expected a live reading, got {other:?}"),
        }
    }

    #[test]
    fn rolling_four_hours_by_name_sorts_as_other() {
        let usage = reading_for(serde_json::json!({"rolling_4h": {"used": 10, "limit": 100}}));
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].id, "chutes.rolling");
        // The four-hour lane is upstream's `.other(seconds:)` — `.fiveHour`
        // is reserved for a window that states five hours exactly.
        assert_eq!(windows[0].kind, Kind::Other(4 * 3_600));
        assert_eq!(windows[0].window_seconds, 4 * 3_600);
        assert!((windows[0].used_fraction - 0.1).abs() < 1e-9);
        // The length came from the name, so it is a stated one.
        assert!(windows[0].reports_length);
    }

    #[test]
    fn stated_five_hour_window_maps_to_five_hour() {
        let usage =
            reading_for(serde_json::json!({"rolling": {"window_hours": 5, "used": 1, "limit": 2}}));
        let windows = live_windows(&usage);
        assert_eq!(windows[0].kind, Kind::FiveHour);
    }

    #[test]
    fn stated_length_in_hours_wins_and_accepts_strings() {
        let usage = reading_for(
            serde_json::json!({"rolling": {"window_hours": "6", "used": 1, "limit": 4}}),
        );
        let windows = live_windows(&usage);
        assert_eq!(windows[0].kind, Kind::Other(6 * 3_600));
    }

    #[test]
    fn bare_rolling_without_length_never_hides_a_four_hour() {
        let usage = reading_for(serde_json::json!({
            "rolling_window": {"used": 1, "limit": 2},
            "fourhourusage": {"used": 3, "limit": 4},
        }));
        let windows = live_windows(&usage);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window_seconds, 4 * 3_600);
        assert!((windows[0].used_fraction - 0.75).abs() < 1e-9);
    }

    #[test]
    fn monthly_window_is_a_sort_key_not_a_length() {
        let usage = reading_for(serde_json::json!({"monthly": {"used": 5, "limit": 10}}));
        let windows = live_windows(&usage);
        assert_eq!(windows[0].id, "chutes.monthly");
        assert_eq!(windows[0].kind, Kind::Monthly);
        assert_eq!(windows[0].window_seconds, 30 * 86_400);
        assert!(!windows[0].reports_length);
    }

    #[test]
    fn percentage_lane_reads_on_a_hundred_scale() {
        // The bare `rolling` name states no length, so a lane that carries
        // only a percentage is skipped with it; the name that says four
        // hours carries the length.
        let usage = reading_for(serde_json::json!({"rolling_4h": {"percent_used": 40}}));
        let windows = live_windows(&usage);
        assert!((windows[0].used_fraction - 0.4).abs() < 1e-9);
    }

    #[test]
    fn remaining_reads_as_used() {
        let usage =
            reading_for(serde_json::json!({"monthly": {"remaining": 30, "monthlylimit": 100}}));
        let windows = live_windows(&usage);
        assert!((windows[0].used_fraction - 0.7).abs() < 1e-9);
    }

    #[test]
    fn exhausted_when_used_reaches_the_limit() {
        let usage = reading_for(serde_json::json!({"rolling_4h": {"used": 100, "limit": 100}}));
        let windows = live_windows(&usage);
        assert!(windows[0].is_exhausted);
    }

    #[test]
    fn reset_reads_iso_and_epoch_millis() {
        let usage = reading_for(serde_json::json!({
            "monthly": {"used": 1, "limit": 2, "reset_at": "2026-03-01T00:00:00Z"},
        }));
        let expected = crate::timeutil::parse_iso8601_ms("2026-03-01T00:00:00Z");
        assert_eq!(live_windows(&usage)[0].resets_at, expected);

        let usage = reading_for(serde_json::json!({
            "rolling": {"window_hours": 4, "used": 1, "limit": 2, "resetsAt": 1_700_000_000_000i64},
        }));
        assert_eq!(live_windows(&usage)[0].resets_at, Some(1_700_000_000_000));
    }

    #[test]
    fn windows_sort_shortest_first_and_plan_is_trimmed() {
        let usage = reading_for(serde_json::json!({
            "monthly": {"used": 1, "limit": 2},
            "rolling_4h": {"used": 1, "limit": 2},
            "subscription": {"plan_name": " Pro "},
        }));
        let windows = live_windows(&usage);
        assert_eq!(windows[0].id, "chutes.rolling");
        assert_eq!(windows[1].id, "chutes.monthly");
        assert_eq!(usage.plan.as_deref(), Some("Pro"));
    }

    #[test]
    fn data_envelope_is_unwrapped() {
        let usage = reading_for(serde_json::json!({
            "data": {"monthly": {"used": 5, "limit": 10}},
        }));
        assert_eq!(live_windows(&usage).len(), 1);
    }

    #[test]
    fn no_windows_reports_no_limits() {
        let usage = reading_for(serde_json::json!({"subscription": {"status": "active"}}));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::NoLimitsReported)
        ));
    }

    #[test]
    fn non_object_reply_is_unreadable() {
        let usage = reading_for(serde_json::json!("nope"));
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }
}
