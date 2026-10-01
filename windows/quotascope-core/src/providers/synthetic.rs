//! Synthetic: a rolling five-hour allowance, a weekly token allowance and an
//! hourly search allowance, each stated by the service as a percentage or as
//! an amount used out of a limit.
//!
//! Read with a key the user pastes, from Synthetic's documented quota route:
//! `GET https://api.synthetic.new/v2/quotas`. The shape is second-hand —
//! taken from CodexBar's Synthetic plugin and its tests, not from a captured
//! reply.
//!
//! Only the three named slots are read. Falling back to any object anywhere
//! in the reply that carries a number called `limit` or `used` would guess at
//! what a lane is and how long it lasts.
//!
//! **No reset for the rolling lanes.** The five-hour and weekly allowances
//! regenerate a slice at a time: the reply's next-tick field is the next
//! slice, not a turnover. Drawn as a reset it would move forward every few
//! minutes and every one of those would read as the window starting again.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{bool_field, number_field, string_field, HttpClient};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.synthetic.new/v2/quotas";

pub struct SyntheticService {
    http: Arc<HttpClient>,
}

impl SyntheticService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for SyntheticService {
    fn provider(&self) -> Provider {
        Provider::Synthetic
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Synthetic);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Synthetic)) else {
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
        match reading(&reply) {
            Ok(reading) => {
                let mut usage = ProviderUsage::live_now(account, reading.windows);
                usage.plan = reading.plan;
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// What one reply yields, kept apart from the HTTP plumbing so tests can feed
/// it fixtures directly.
#[derive(Debug)]
pub struct SyntheticReading {
    pub windows: Vec<UsageWindow>,
    pub plan: Option<String>,
}

/// The three named slots at one level of the reply. Every field is optional
/// because the slots report different ones: the rolling lane counts down from
/// `max`, the weekly lane states a percentage and a dollar figure, search
/// counts requests.
struct Slots<'a> {
    rolling: Option<&'a serde_json::Value>,
    weekly: Option<&'a serde_json::Value>,
    hourly: Option<&'a serde_json::Value>,
    plan: Option<&'a str>,
}

/// One slot. A slot that is there but is not an object is not a lane anyone
/// can read — the whole reply is unreadable, as a strict decode would be.
fn slot<'a>(
    reply: &'a serde_json::Value,
    key: &str,
) -> Result<Option<&'a serde_json::Value>, Unavailability> {
    match reply.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value)),
        Some(_) => Err(Unavailability::UnreadableReply),
    }
}

fn plan_of(reply: &serde_json::Value) -> Result<Option<&str>, Unavailability> {
    match reply.get("plan") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(Unavailability::UnreadableReply),
    }
}

fn level_slots(reply: &serde_json::Value) -> Result<Slots<'_>, Unavailability> {
    let rolling = slot(reply, "rollingFiveHourLimit")?;
    let weekly = slot(reply, "weeklyTokenLimit")?;
    let hourly = match slot(reply, "search")? {
        None => None,
        Some(search) => slot(search, "hourly")?,
    };
    Ok(Slots {
        rolling,
        weekly,
        hourly,
        plan: plan_of(reply)?,
    })
}

pub fn reading(reply: &serde_json::Value) -> Result<SyntheticReading, Unavailability> {
    let root = level_slots(reply)?;
    // The slots sit at the root or under `data`; both are accepted.
    let slots = if root.rolling.is_some() || root.weekly.is_some() || root.hourly.is_some() {
        root
    } else {
        match reply.get("data") {
            None | Some(serde_json::Value::Null) => root,
            Some(data) if data.is_object() => level_slots(data)?,
            Some(_) => return Err(Unavailability::UnreadableReply),
        }
    };

    let mut windows = Vec::new();
    if let Some(slot) = slots.rolling {
        if let Some(used) = fraction(
            None,
            number_field(slot, "remaining"),
            number_field(slot, "max"),
        ) {
            windows.push(window(
                "synthetic.five_hour",
                Kind::FiveHour,
                None,
                used,
                5 * 3_600,
                None,
                bool_field(slot, "limited"),
            ));
        }
    }
    if let Some(slot) = slots.weekly {
        if let Some(used) = weekly_fraction(slot) {
            windows.push(window(
                "synthetic.weekly",
                Kind::Weekly,
                None,
                used,
                7 * 86_400,
                None,
                bool_field(slot, "limited"),
            ));
        }
    }
    // "Search" is Synthetic's search API, a product of its own, so it is a
    // scope and stays untranslated.
    if let Some(slot) = slots.hourly {
        if let Some(used) = fraction(
            number_field(slot, "requests"),
            number_field(slot, "remaining"),
            number_field(slot, "limit"),
        ) {
            windows.push(window(
                "synthetic.search_hourly",
                Kind::Other(3_600),
                Some("Search"),
                used,
                3_600,
                string_field(slot, "renewsAt").and_then(crate::timeutil::parse_iso8601_ms),
                bool_field(slot, "limited"),
            ));
        }
    }

    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    let plan = slots
        .plan
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .map(str::to_string);
    // Shortest window first, the order the rail shows.
    windows.sort_by_key(|window| window.window_seconds);
    Ok(SyntheticReading { windows, plan })
}

fn window(
    id: &str,
    kind: Kind,
    scope: Option<&str>,
    used: f64,
    window_seconds: i64,
    resets_at: Option<i64>,
    limited: Option<bool>,
) -> UsageWindow {
    let mut window = UsageWindow::new(
        id,
        kind,
        scope.map(str::to_string),
        used,
        window_seconds,
        resets_at,
    );
    window.is_exhausted = limited == Some(true) || used >= 1.0;
    window
}

/// Used out of a limit, from whichever two of used / remaining / limit the
/// slot states. A limit of zero or less, or a negative figure, is not an
/// allowance and gives nothing. Not clamped: a lane may report past its end.
pub fn fraction(used: Option<f64>, remaining: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let limit = limit.filter(|value| value.is_finite() && *value > 0.0)?;
    if let Some(used) = used.filter(|value| value.is_finite() && *value >= 0.0) {
        return Some(used / limit);
    }
    if let Some(remaining) = remaining.filter(|value| value.is_finite() && *value >= 0.0) {
        return Some((limit - remaining).max(0.0) / limit);
    }
    None
}

/// The weekly lane states a percentage left, on a 0–100 scale; failing that,
/// a dollar allowance and what is left of it.
fn weekly_fraction(slot: &serde_json::Value) -> Option<f64> {
    if let Some(left) = number_field(slot, "percentRemaining")
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
    {
        return Some((100.0 - left) / 100.0);
    }
    fraction(
        None,
        dollars(string_field(slot, "remainingCredits")),
        dollars(string_field(slot, "maxCredits")),
    )
}

/// "$36.00" → 36. Anything else is not a figure.
pub fn dollars(text: Option<&str>) -> Option<f64> {
    let cleaned = text?.trim().replace('$', "").replace(',', "");
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn root_level_slots_yield_all_three_windows_sorted_shortest_first() {
        let reply = json!({
            "plan": "Creator",
            "rollingFiveHourLimit": { "max": 100.0, "remaining": 25.0 },
            "weeklyTokenLimit": { "percentRemaining": 40.0 },
            "search": { "hourly": {
                "requests": 5.0, "limit": 10.0, "renewsAt": "2026-10-01T12:00:00Z",
            } },
        });
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Creator"));
        let ids: Vec<&str> = reading.windows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "synthetic.search_hourly",
                "synthetic.five_hour",
                "synthetic.weekly"
            ]
        );
        let five_hour = &reading.windows[1];
        assert_eq!(five_hour.used_fraction, 0.75);
        assert_eq!(five_hour.kind, Kind::FiveHour);
        assert!(five_hour.reports_length);
        assert!(five_hour.resets_at.is_none());
        let weekly = &reading.windows[2];
        assert_eq!(weekly.used_fraction, 0.6);
        assert_eq!(weekly.kind, Kind::Weekly);
        let search = &reading.windows[0];
        assert_eq!(search.scope.as_deref(), Some("Search"));
        assert_eq!(search.kind, Kind::Other(3_600));
        assert!(search.resets_at.is_some());
    }

    #[test]
    fn slots_under_data_are_read_with_data_plan() {
        let reply = json!({
            "plan": "IgnoredRootPlan",
            "data": {
                "plan": "RealPlan",
                "rollingFiveHourLimit": { "max": 4.0, "remaining": 1.0 },
            },
        });
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.plan.as_deref(), Some("RealPlan"));
        assert_eq!(reading.windows[0].used_fraction, 0.75);
    }

    #[test]
    fn empty_data_falls_back_to_the_root_plan_and_reports_nothing() {
        let reply = json!({ "plan": "Solo", "data": {} });
        assert_eq!(
            reading(&reply).unwrap_err(),
            Unavailability::NoLimitsReported
        );
        let bare = json!({ "plan": "Solo" });
        assert_eq!(
            reading(&bare).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn the_weekly_lane_falls_back_from_a_percentage_to_dollars() {
        assert_eq!(
            weekly_fraction(&json!({ "percentRemaining": 12.5 })),
            Some(0.875)
        );
        // Out of range percentages are not a percentage at all.
        assert_eq!(
            weekly_fraction(
                &json!({ "percentRemaining": 140.0, "maxCredits": "$40.00", "remainingCredits": "$10.00" })
            ),
            Some(0.75)
        );
        assert_eq!(
            weekly_fraction(&json!({ "maxCredits": "$1,200.50", "remainingCredits": "$0.50" })),
            Some(1200.0 / 1200.5)
        );
    }

    #[test]
    fn a_limited_lane_is_exhausted_whatever_the_fraction() {
        let reply = json!({
            "rollingFiveHourLimit": { "max": 100.0, "remaining": 90.0, "limited": true },
        });
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.windows[0].used_fraction, 0.1);
        assert!(reading.windows[0].is_exhausted);
    }

    #[test]
    fn a_slot_that_is_not_an_object_makes_the_reply_unreadable() {
        let reply = json!({ "rollingFiveHourLimit": "yes" });
        assert_eq!(
            reading(&reply).unwrap_err(),
            Unavailability::UnreadableReply
        );
    }

    #[test]
    fn fraction_rules() {
        assert_eq!(fraction(None, Some(3.0), Some(4.0)), Some(0.25));
        assert_eq!(fraction(Some(2.0), Some(3.0), Some(4.0)), Some(0.5));
        assert_eq!(fraction(None, Some(9.0), Some(4.0)), Some(0.0));
        assert_eq!(fraction(None, Some(3.0), Some(0.0)), None);
        assert_eq!(fraction(None, Some(-1.0), Some(4.0)), None);
        assert_eq!(fraction(None, None, Some(4.0)), None);
    }

    #[test]
    fn dollars_strips_the_dressing() {
        assert_eq!(dollars(Some("$36.00")), Some(36.0));
        assert_eq!(dollars(Some(" 1,200.50 ")), Some(1200.5));
        assert_eq!(dollars(Some("free")), None);
        assert_eq!(dollars(None), None);
    }
}
