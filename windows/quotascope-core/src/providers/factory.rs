//! Factory, the company behind the Droid agent: either the token-rate-limit
//! billing — a five-hour, a weekly and a monthly limit, each a percentage
//! Factory itself states — or, on the older billing, a Standard and a Premium
//! token allowance for the billing period. Money bought on top of the limits
//! is shown beside them when the account has it.
//!
//! Read with a key the user pastes (`fk-…`, from app.factory.ai's API keys
//! page), sent as a bearer token to `api.factory.ai` and nowhere else, with
//! the headers Factory's own web app sends. Three requests, in the order the
//! web app makes them:
//!
//! 1. `GET /api/app/auth/me` — the plan's name and the user's id. The one
//!    that says whether the key is any good.
//! 2. `GET /api/billing/limits` — the token-rate-limit billing. Only an
//!    account that says `usesTokenRateLimitsBilling` is read from here; any
//!    other answer, a failure included, means the older billing is asked.
//! 3. `GET /api/organization/subscription/usage` — the Standard and Premium
//!    allowances, for everyone else.
//!
//! The shapes are second-hand — taken from CodexBar's Factory provider and
//! its tests, not from a captured reply — and the tests here say so. Where
//! CodexBar draws a window whose stated end has passed as 0%, the figure is
//! simply left off until the service states a new one; an allowance over a
//! trillion tokens is Factory's way of writing "unlimited" and is left off
//! rather than drawn against a number QuotaScope chose.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{bool_field, number_field, object_field, string_field, HttpClient, Method};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use std::sync::Arc;

const BASE: &str = "https://api.factory.ai";
const ME_PATH: &str = "/api/app/auth/me";
const LIMITS_PATH: &str = "/api/billing/limits";
const USAGE_PATH: &str = "/api/organization/subscription/usage";

/// Above this an allowance is Factory's way of writing "unlimited", and a
/// fraction of it is not a figure anybody reported.
const UNLIMITED_ALLOWANCE: f64 = 1e12;

/// A billing month rather than a fixed thirty days — thirty only sorts.
const THIRTY_DAYS: i64 = 30 * 86_400;

pub struct FactoryService {
    http: Arc<HttpClient>,
}

impl FactoryService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for FactoryService {
    fn provider(&self) -> Provider {
        Provider::Factory
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Factory);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Factory)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = request_headers(&key);
        let refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();

        // The one that says whether the key is any good.
        let me = match self
            .http
            .fetch_json(Method::Get, &format!("{BASE}{ME_PATH}"), &refs, None)
        {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let (plan, user_id) = identity(&me);
        let now_ms = crate::timeutil::now_ms();

        // The newer billing, when the account is on it. Anything else — a
        // failure included — is Factory saying to ask the older route, which
        // is what its own web app does.
        if let Ok(reply) =
            self.http
                .fetch_json(Method::Get, &format!("{BASE}{LIMITS_PATH}"), &refs, None)
        {
            if let Some(reading) = limits_reading(&reply, now_ms) {
                return match reading {
                    Ok(reading) => {
                        let mut usage = ProviderUsage::live_now(account, reading.windows);
                        usage.origin = Some(self.origin_token().into());
                        usage.plan = plan;
                        usage.credit_balance = reading
                            .credit
                            .as_ref()
                            .map(|credit| money_text(credit.amount, &credit.currency));
                        usage.credit_remaining = reading.credit;
                        usage
                    }
                    Err(reason) => ProviderUsage::unavailable(account, reason),
                };
            }
        }

        let mut url = format!("{BASE}{USAGE_PATH}?useCache=true");
        if let Some(user_id) = &user_id {
            url.push_str("&userId=");
            url.push_str(&query_encode(user_id));
        }
        let reply = match self.http.fetch_json(Method::Get, &url, &refs, None) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let windows = match allowance_windows(&reply) {
            Ok(windows) => windows,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.origin = Some(self.origin_token().into());
        usage.plan = plan;
        usage
    }
}

/// The headers Factory's web app sends, which are the ones its API has been
/// seen accepting a pasted key with.
fn request_headers(key: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".into(), format!("Bearer {key}")),
        ("Accept".into(), "application/json".into()),
        ("x-factory-client".into(), "web-app".into()),
        ("Origin".into(), "https://app.factory.ai".into()),
        ("Referer".into(), "https://app.factory.ai/".into()),
    ]
}

/// The plan's name and the user's id, when the reply carries them. Neither is
/// needed for a reading, so a reply without them is not a failure.
pub fn identity(reply: &serde_json::Value) -> (Option<String>, Option<String>) {
    let subscription = reply
        .get("organization")
        .and_then(|org| org.get("subscription"));
    let tier = subscription
        .and_then(|subscription| string_field(subscription, "factoryTier"))
        .map(str::trim)
        .filter(|tier| !tier.is_empty())
        .map(capitalized);
    let plan = subscription
        .and_then(|subscription| subscription.get("orbSubscription"))
        .and_then(|orb| orb.get("plan"))
        .and_then(|plan| string_field(plan, "name"))
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .map(str::to_string)
        .or(tier);
    let user_id = reply
        .get("userProfile")
        .and_then(|profile| string_field(profile, "id"))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    (plan, user_id)
}

/// A date Factory writes as seconds, as milliseconds, as either in a string,
/// or as ISO 8601 — all four have been seen. Returns epoch milliseconds.
pub fn factory_date(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(number) => epoch_ms(number.as_f64()?),
        serde_json::Value::String(text) => {
            let text = text.trim();
            match text.parse::<f64>() {
                Ok(number) => epoch_ms(number),
                Err(_) => crate::timeutil::parse_iso8601_ms(text),
            }
        }
        _ => None,
    }
}

/// Anything past 10¹² is milliseconds: as seconds it would be the year 33,000.
fn epoch_ms(number: f64) -> Option<i64> {
    if !number.is_finite() || number <= 0.0 {
        return None;
    }
    Some(if number > 1e12 {
        number as i64
    } else {
        (number * 1000.0) as i64
    })
}

/// The newer billing's reading, or `None` when this account is not on it and
/// the older route should be asked instead. `Some(Err(..))` is an account on
/// this billing that reports nothing — an answer, not a reason to fall
/// through.
pub fn limits_reading(
    reply: &serde_json::Value,
    now_ms: i64,
) -> Option<Result<LimitsReading, Unavailability>> {
    if bool_field(reply, "usesTokenRateLimitsBilling") != Some(true) {
        return None;
    }
    let pools = object_field(reply, "limits")?;

    let mut windows = pools
        .get("standard")
        .filter(|pool| pool.is_object())
        .map(|pool| pool_windows(pool, None, now_ms))
        .unwrap_or_default();
    // Core is its own pool of models. CodexBar draws it only once it has
    // something in it, and so does this: an empty pool with no clock is a
    // pool the account is not using.
    if let Some(core) = object_field(pools, "core") {
        let core_windows = pool_windows(core, Some("Core"), now_ms);
        if core_windows
            .iter()
            .any(|window| window.used_fraction > 0.0 || window.resets_at.is_some())
        {
            windows.extend(core_windows);
        }
    }

    let credit = extra_usage(reply);
    if windows.is_empty() && credit.is_none() {
        return Some(Err(Unavailability::NoLimitsReported));
    }
    Some(Ok(LimitsReading { windows, credit }))
}

#[derive(Debug)]
pub struct LimitsReading {
    pub windows: Vec<UsageWindow>,
    pub credit: Option<CreditAmount>,
}

/// The three windows, each with the length its name states. `monthly` is a
/// billing month rather than a fixed thirty days, so its length is only a
/// sort key and is not claimed.
fn pool_windows(pool: &serde_json::Value, scope: Option<&str>, now_ms: i64) -> Vec<UsageWindow> {
    let shapes = [
        ("fiveHour", Kind::FiveHour, 5 * 3_600, true),
        ("weekly", Kind::Weekly, 7 * 86_400, true),
        ("monthly", Kind::Monthly, THIRTY_DAYS, false),
    ];
    let mut out = Vec::new();
    for (name, kind, seconds, reports_length) in shapes {
        let Some(window) = object_field(pool, name) else {
            continue;
        };
        let Some(percent) =
            number_field(window, "usedPercent").filter(|p| p.is_finite() && *p >= 0.0)
        else {
            continue;
        };

        let end = window.get("windowEnd").and_then(factory_date);
        let reset =
            match number_field(window, "secondsRemaining").filter(|s| s.is_finite() && *s > 0.0) {
                Some(seconds) => Some(now_ms + (seconds * 1000.0) as i64),
                None => end.filter(|end| *end > now_ms),
            };
        // The window this figure belongs to is over and no new one has been
        // stated. CodexBar draws it as 0%; the figure is simply gone.
        if reset.is_none() && end.is_some() {
            continue;
        }

        let id = match scope {
            Some(scope) => format!("factory.{}.{}", scope.to_lowercase(), name),
            None => format!("factory.{name}"),
        };
        let mut window = UsageWindow::new(
            &id,
            kind,
            scope.map(str::to_string),
            percent / 100.0,
            seconds,
            reset,
        );
        window.reports_length = reports_length;
        window.is_exhausted = percent >= 100.0;
        out.push(window);
    }
    out
}

/// Money bought on top of the limits, in US cents. Shown when the account can
/// use it or has some; an account that has never been offered it is not told
/// it has none.
fn extra_usage(reply: &serde_json::Value) -> Option<CreditAmount> {
    let cents = number_field(reply, "extraUsageBalanceCents")?;
    if cents < 0.0 {
        return None;
    }
    if cents <= 0.0 && bool_field(reply, "extraUsageAllowed") != Some(true) {
        return None;
    }
    Some(CreditAmount {
        amount: cents / 100.0,
        currency: "USD".into(),
    })
}

/// The older billing: the Standard and Premium token allowances for the
/// billing period.
pub fn allowance_windows(reply: &serde_json::Value) -> Result<Vec<UsageWindow>, Unavailability> {
    let period = object_field(reply, "usage").ok_or(Unavailability::UnreadableReply)?;

    let end = period.get("endDate").and_then(factory_date);
    // The period's length is stated when both ends are; otherwise thirty days
    // is a sort key and nothing more. The dates are milliseconds; the length,
    // as everywhere, is seconds.
    let stated = period
        .get("startDate")
        .and_then(factory_date)
        .and_then(|start| end.map(|end| (end - start) / 1000))
        .filter(|seconds| *seconds > 0);

    let mut windows = Vec::new();
    for (id, scope, key) in [
        ("factory.standard", "Standard", "standard"),
        ("factory.premium", "Premium", "premium"),
    ] {
        let Some(tokens) = object_field(period, key) else {
            continue;
        };
        let Some(fraction) = allowance_fraction(tokens) else {
            continue;
        };
        let mut window = UsageWindow::new(
            id,
            Kind::Monthly,
            Some(scope.into()),
            fraction,
            stated.unwrap_or(THIRTY_DAYS),
            end,
        );
        window.reports_length = stated.is_some();
        window.is_exhausted = fraction >= 1.0;
        windows.push(window);
    }
    if windows.is_empty() {
        return Err(Unavailability::NoLimitsReported);
    }
    Ok(windows)
}

/// Factory's own ratio where it gives a usable one, and otherwise the tokens
/// used against the allowance, both as reported.
///
/// The ratio is on a 0…1 scale. CodexBar clamps a hair over either end and so
/// does this; anything further out is a scale nobody stated. A ratio of zero
/// beside tokens used and a real allowance is Factory's cache lagging, and
/// the two counts are read instead — also CodexBar's rule. A zero with no
/// allowance at all is a pool this plan does not have.
fn allowance_fraction(tokens: &serde_json::Value) -> Option<f64> {
    let used = number_field(tokens, "userTokens").filter(|used| used.is_finite() && *used >= 0.0);
    let allowance = number_field(tokens, "totalAllowance").filter(|allowance| {
        allowance.is_finite() && *allowance > 0.0 && *allowance <= UNLIMITED_ALLOWANCE
    });

    if let Some(ratio) = number_field(tokens, "usedRatio")
        .filter(|ratio| ratio.is_finite() && (-0.001..=1.001).contains(ratio))
    {
        let lagging = ratio <= 0.0 && used.unwrap_or(0.0) > 0.0 && allowance.is_some();
        let absent = ratio <= 0.0 && allowance.is_none();
        if !lagging && !absent {
            return Some(ratio.clamp(0.0, 1.0));
        }
    }
    match (used, allowance) {
        (Some(used), Some(allowance)) => Some(used / allowance),
        _ => None,
    }
}

/// Swift's `capitalized`: the first letter of each word up, the rest down.
fn capitalized(text: &str) -> String {
    text.split_whitespace()
        .map(|word| match word.chars().next() {
            Some(first) => {
                first.to_uppercase().collect::<String>() + &word[first.len_utf8()..].to_lowercase()
            }
            None => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The balance as money — symbol first, cents kept.
fn money_text(amount: f64, currency: &str) -> String {
    let symbol = match currency {
        "USD" => "$",
        "CNY" => "¥",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        other => return format!("{other} {amount:.2}"),
    };
    format!("{symbol}{amount:.2}")
}

/// The same percent-encoding a URL query item gets, for the user id.
fn query_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_760_000_000_000;

    fn limits_reply() -> serde_json::Value {
        serde_json::json!({
            "usesTokenRateLimitsBilling": true,
            "limits": {
                "standard": {
                    "fiveHour": {"usedPercent": 42.0, "secondsRemaining": 1800},
                    "weekly": {"usedPercent": 12.5, "windowEnd": NOW + 3 * 86_400_000i64},
                    "monthly": {"usedPercent": 5.0},
                },
            },
        })
    }

    #[test]
    fn token_rate_limits_reads_the_three_standard_windows() {
        let reading = limits_reading(&limits_reply(), NOW).unwrap().unwrap();
        assert!(reading.credit.is_none());
        assert_eq!(reading.windows.len(), 3);

        let five_hour = &reading.windows[0];
        assert_eq!(five_hour.id, "factory.fiveHour");
        assert_eq!(five_hour.kind, Kind::FiveHour);
        assert_eq!(five_hour.used_fraction, 0.42);
        assert!(five_hour.reports_length);
        assert_eq!(five_hour.resets_at, Some(NOW + 1_800_000));

        let weekly = &reading.windows[1];
        assert_eq!(weekly.id, "factory.weekly");
        assert_eq!(weekly.resets_at, Some(NOW + 3 * 86_400_000));

        let monthly = &reading.windows[2];
        assert_eq!(monthly.kind, Kind::Monthly);
        assert!(
            !monthly.reports_length,
            "a billing month is not thirty days"
        );
        assert!(monthly.resets_at.is_none());
    }

    #[test]
    fn a_window_whose_end_has_passed_is_left_off() {
        let reply = serde_json::json!({
            "usesTokenRateLimitsBilling": true,
            "limits": {"standard": {"fiveHour": {"usedPercent": 90.0, "windowEnd": NOW - 1_000}}},
        });
        // The figure is gone, and it was the only one — an answer, not a
        // fall-through to the older billing.
        assert_eq!(
            limits_reading(&reply, NOW).unwrap().unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn a_hundred_percent_window_is_marked_spent() {
        let reply = serde_json::json!({
            "usesTokenRateLimitsBilling": true,
            "limits": {"standard": {"fiveHour": {"usedPercent": 100, "secondsRemaining": 60}}},
        });
        let reading = limits_reading(&reply, NOW).unwrap().unwrap();
        assert!(reading.windows[0].is_exhausted);
        assert_eq!(reading.windows[0].used_fraction, 1.0);
    }

    #[test]
    fn the_core_pool_only_draws_once_it_has_something_in_it() {
        let mut reply = limits_reply();
        reply["limits"]["core"] = serde_json::json!({
            "fiveHour": {"usedPercent": 0.0},
            "weekly": {"usedPercent": 0.0},
            "monthly": {"usedPercent": 0.0},
        });
        let reading = limits_reading(&reply, NOW).unwrap().unwrap();
        assert_eq!(reading.windows.len(), 3, "an unused pool is not drawn");

        reply["limits"]["core"]["fiveHour"] =
            serde_json::json!({"usedPercent": 7.5, "secondsRemaining": 60});
        let reading = limits_reading(&reply, NOW).unwrap().unwrap();
        let core: Vec<_> = reading
            .windows
            .iter()
            .filter(|window| window.scope.as_deref() == Some("Core"))
            .collect();
        assert_eq!(core.len(), 3);
        assert_eq!(core[0].id, "factory.core.fiveHour");
        assert_eq!(core[0].used_fraction, 0.075);
    }

    #[test]
    fn extra_usage_money_shows_when_it_exists() {
        let mut reply = limits_reply();
        reply["extraUsageBalanceCents"] = serde_json::json!(500);
        let reading = limits_reading(&reply, NOW).unwrap().unwrap();
        assert_eq!(
            reading.credit,
            Some(CreditAmount {
                amount: 5.0,
                currency: "USD".into()
            })
        );
        assert_eq!(reading.windows.len(), 3);

        // Zero cents the account cannot spend is no money at all.
        reply["extraUsageBalanceCents"] = serde_json::json!(0);
        let reading = limits_reading(&reply, NOW).unwrap().unwrap();
        assert!(reading.credit.is_none());
        reply["extraUsageAllowed"] = serde_json::json!(true);
        let reading = limits_reading(&reply, NOW).unwrap().unwrap();
        assert_eq!(
            reading.credit,
            Some(CreditAmount {
                amount: 0.0,
                currency: "USD".into()
            })
        );
    }

    #[test]
    fn a_reply_not_on_the_token_billing_falls_through() {
        for reply in [
            serde_json::json!({}),
            serde_json::json!({"usesTokenRateLimitsBilling": false, "limits": {"standard": {}}}),
            serde_json::json!({"usesTokenRateLimitsBilling": true}),
        ] {
            assert!(limits_reading(&reply, NOW).is_none());
        }
    }

    #[test]
    fn an_empty_token_billing_is_an_answer_not_a_fall_through() {
        let reply = serde_json::json!({"usesTokenRateLimitsBilling": true, "limits": {}});
        assert_eq!(
            limits_reading(&reply, NOW).unwrap().unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn a_date_arrives_as_seconds_milliseconds_a_string_or_iso() {
        assert_eq!(
            factory_date(&serde_json::json!(1_760_000_000)),
            Some(1_760_000_000_000)
        );
        assert_eq!(
            factory_date(&serde_json::json!(1.8e15)),
            Some(1_800_000_000_000_000)
        );
        assert_eq!(
            factory_date(&serde_json::json!("1760000000")),
            Some(1_760_000_000_000)
        );
        assert_eq!(
            factory_date(&serde_json::json!("2026-10-01T00:00:00Z")),
            crate::timeutil::parse_iso8601_ms("2026-10-01T00:00:00Z")
        );
        assert_eq!(factory_date(&serde_json::json!(0)), None);
        assert_eq!(factory_date(&serde_json::json!(-5)), None);
    }

    #[test]
    fn the_older_billing_reads_standard_and_premium_allowances() {
        let reply = serde_json::json!({
            "usage": {
                "startDate": "2026-09-01T00:00:00Z",
                "endDate": "2026-10-01T00:00:00Z",
                "standard": {"userTokens": 2_000_000.0, "totalAllowance": 10_000_000.0},
                "premium": {"usedRatio": 0.25},
            },
        });
        let windows = allowance_windows(&reply).unwrap();
        assert_eq!(windows.len(), 2);

        let standard = &windows[0];
        assert_eq!(standard.id, "factory.standard");
        assert_eq!(standard.kind, Kind::Monthly);
        assert_eq!(standard.scope.as_deref(), Some("Standard"));
        assert_eq!(standard.used_fraction, 0.2);
        assert_eq!(standard.window_seconds, THIRTY_DAYS);
        assert!(
            standard.reports_length,
            "the period is stated by its two ends"
        );
        assert_eq!(
            standard.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-01T00:00:00Z")
        );

        let premium = &windows[1];
        assert_eq!(premium.used_fraction, 0.25);
        assert_eq!(premium.scope.as_deref(), Some("Premium"));
    }

    #[test]
    fn a_zero_ratio_beside_real_counts_is_cache_lag() {
        let reply = serde_json::json!({
            "usage": {"standard": {"usedRatio": 0.0, "userTokens": 5.0, "totalAllowance": 10.0}},
        });
        let windows = allowance_windows(&reply).unwrap();
        assert_eq!(windows[0].used_fraction, 0.5);
    }

    #[test]
    fn a_zero_ratio_with_no_allowance_is_a_pool_this_plan_lacks() {
        let reply =
            serde_json::json!({"usage": {"standard": {"usedRatio": 0.0, "userTokens": 5.0}}});
        assert_eq!(
            allowance_windows(&reply).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn an_unstated_ratio_scale_falls_back_to_the_counts() {
        let reply = serde_json::json!({
            "usage": {"standard": {"usedRatio": 7.0, "userTokens": 5.0, "totalAllowance": 10.0}},
        });
        let windows = allowance_windows(&reply).unwrap();
        assert_eq!(windows[0].used_fraction, 0.5);
    }

    #[test]
    fn an_unlimited_allowance_is_left_off() {
        let reply = serde_json::json!({
            "usage": {
                "standard": {"userTokens": 5.0, "totalAllowance": 2e12},
                "premium": {"usedRatio": 0.5},
            },
        });
        let windows = allowance_windows(&reply).unwrap();
        assert_eq!(
            windows
                .iter()
                .map(|window| window.id.as_str())
                .collect::<Vec<_>>(),
            ["factory.premium"]
        );
    }

    #[test]
    fn the_plan_comes_from_the_orb_subscription_or_the_tier() {
        let (plan, user) = identity(&serde_json::json!({
            "organization": {"subscription": {
                "orbSubscription": {"plan": {"name": "Builder Pro"}},
                "factoryTier": "starter",
            }},
            "userProfile": {"id": " usr_42 "},
        }));
        assert_eq!(plan.as_deref(), Some("Builder Pro"));
        assert_eq!(user.as_deref(), Some("usr_42"));

        let (plan, user) = identity(
            &serde_json::json!({"organization": {"subscription": {"factoryTier": "starter"}}}),
        );
        assert_eq!(plan.as_deref(), Some("Starter"));
        assert!(user.is_none());

        let (plan, user) = identity(&serde_json::json!({}));
        assert!(plan.is_none() && user.is_none());
    }

    #[test]
    fn money_and_query_values() {
        assert_eq!(money_text(5.0, "USD"), "$5.00");
        assert_eq!(money_text(5.0, "CHF"), "CHF 5.00");
        assert_eq!(query_encode("usr 42/x"), "usr%2042%2Fx");
    }
}
