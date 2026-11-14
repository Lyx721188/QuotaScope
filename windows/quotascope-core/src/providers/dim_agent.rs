//! DimAgent's `dim usage --json`, ported from Pulse 338c796.
use super::{KeyRing, ProviderService};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde_json::Value;

pub struct DimAgentService;
impl ProviderService for DimAgentService {
    fn provider(&self) -> Provider {
        Provider::DimAgent
    }
    fn origin_token(&self) -> &'static str {
        "dimCLI"
    }
    fn fetch(&self, _: &KeyRing) -> ProviderUsage {
        use crate::local_process::{probe, Failure};
        match probe("dim", &["usage", "--json"]) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(|v| reading(&v, crate::timeutil::now_ms()))
                .unwrap_or_else(|_| unavailable(Unavailability::UnreadableReply)),
            Err(Failure::Missing | Failure::Start) => unavailable(Unavailability::LocalAppMissing),
            Err(Failure::Exited(text)) => unavailable(exit_reason(&text)),
            Err(Failure::TooMuchOutput) => unavailable(Unavailability::UnreadableReply),
            Err(Failure::TimedOut) => unavailable(Unavailability::Unreachable),
        }
    }
}
fn unavailable(reason: Unavailability) -> ProviderUsage {
    ProviderUsage::unavailable(AccountKey::primary(Provider::DimAgent), reason)
}
fn exit_reason(text: &str) -> Unavailability {
    let text = text.to_ascii_lowercase();
    if ["not logged in", "not signed in"]
        .iter()
        .any(|s| text.contains(s))
    {
        Unavailability::LocalLoginMissing
    } else if ["dim auth login", "unauthorized", "token expired"]
        .iter()
        .any(|s| text.contains(s))
    {
        Unavailability::LocalLoginExpired
    } else if text.contains("429") || text.contains("rate limit") {
        Unavailability::RateLimited
    } else {
        Unavailability::Unreachable
    }
}
fn number(row: &Value, key: &str) -> Option<f64> {
    row.get(key)?
        .as_f64()
        .filter(|v| v.is_finite() && *v >= 0.0)
}
fn fraction(row: &Value, used_key: &str, total_key: &str) -> Option<f64> {
    let used = number(row, used_key)?;
    let total = number(row, total_key).filter(|v| *v > 0.0)?;
    Some(used / total).filter(|v| v.is_finite())
}
fn date(row: &Value, key: &str) -> Option<i64> {
    row.get(key)?
        .as_str()
        .and_then(crate::timeutil::parse_iso8601_ms)
}

pub fn reading(root: &Value, now: i64) -> ProviderUsage {
    if !root.is_object() {
        return unavailable(Unavailability::UnreadableReply);
    }
    let mut windows = Vec::new();
    let credits = root.get("credits");
    let subscription = credits
        .and_then(|v| v.get("subscription_bucket"))
        .into_iter()
        .map(|v| (v, "dimagent.subscription".to_string(), Kind::Credits));
    let packs = credits
        .and_then(|v| v.get("addon_buckets"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            Some((
                v,
                format!("dimagent.addon.{}", v.get("id")?.as_i64()?),
                Kind::TopUp,
            ))
        });
    let mut seen = std::collections::HashSet::new();
    for (bucket, id, kind) in subscription.chain(packs) {
        let status = bucket.get("status").and_then(Value::as_str);
        if !matches!(status, Some("active" | "exhausted")) || seen.contains(&id) {
            continue;
        }
        let Some(fraction) = fraction(bucket, "used_units", "total_units") else {
            continue;
        };
        let expiry = date(bucket, "expires_at");
        let remaining = number(bucket, "remaining_units");
        if bucket.get("remaining_units").is_some_and(|v| !v.is_null()) && remaining.is_none() {
            continue;
        }
        if expiry.is_some_and(|at| at <= now || remaining.is_none()) {
            continue;
        }
        seen.insert(id.clone());
        let mut window = UsageWindow::new(&id, kind, None, fraction, 0, None);
        window.amounts = Some(crate::model::WindowAmounts {
            used: number(bucket, "used_units").unwrap(),
            limit: number(bucket, "total_units").unwrap(),
            remaining,
        });
        window.reports_length = false;
        window.is_exhausted = status == Some("exhausted") || fraction >= 1.0;
        window.next_expiry_ms = expiry;
        window.next_expiry_amount = expiry.and(remaining);
        windows.push(window);
        let mut occurrences = std::collections::HashMap::<String, usize>::new();
        for token in bucket
            .get("window_states")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(seconds) = number(token, "window_duration_hours")
                .map(|h| h * 3600.0)
                .filter(|s| *s > 0.0 && *s <= 366.0 * 86400.0 && s.fract() == 0.0)
            else {
                continue;
            };
            let Some(fraction) = fraction_for_token(token) else {
                continue;
            };
            let reset = date(token, "window_expires_at");
            if token.get("window_expires_at").is_some_and(|v| !v.is_null()) && reset.is_none()
                || reset.is_some_and(|at| at <= now)
            {
                continue;
            }
            let seconds = seconds as i64;
            let key = token
                .get("id")
                .and_then(Value::as_i64)
                .map(|id| format!("id:{id}"))
                .unwrap_or_else(|| format!("seconds:{seconds}"));
            let occurrence = occurrences.entry(key.clone()).or_default();
            let token_id = format!("{id}.tokens.{key}.{occurrence}");
            *occurrence += 1;
            let kind = match seconds {
                18000 => Kind::FiveHour,
                86400 => Kind::Daily,
                604800 => Kind::Weekly,
                _ => Kind::Other(seconds),
            };
            let mut w = UsageWindow::new(&token_id, kind, None, fraction, seconds, reset);
            w.amounts = Some(crate::model::WindowAmounts {
                used: number(token, "window_token_used").unwrap(),
                limit: number(token, "window_token_cap").unwrap(),
                remaining: None,
            });
            w.is_exhausted = fraction >= 1.0;
            w.next_expiry_ms = expiry;
            w.next_expiry_amount = expiry.and_then(|_| {
                let cap = number(token, "window_token_cap")?;
                let used = number(token, "window_token_used")?;
                Some((cap - used).max(0.0))
            });
            windows.push(w);
        }
    }
    for meter in root
        .get("feature_meters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if meter.get("feature_key").and_then(Value::as_str) != Some("web_search")
            || meter.get("unlimited").and_then(Value::as_bool) == Some(true)
            || windows.iter().any(|w| w.id == "dimagent.web_search")
        {
            continue;
        }
        let Some(fraction) = fraction(meter, "total_used", "total_allowance") else {
            continue;
        };
        let mut w = UsageWindow::new(
            "dimagent.web_search",
            Kind::Credits,
            Some("Web Search".into()),
            fraction,
            0,
            date(meter, "period_end"),
        );
        w.reports_length = false;
        w.is_exhausted = fraction >= 1.0;
        windows.push(w);
    }
    if windows.is_empty() {
        return unavailable(Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(AccountKey::primary(Provider::DimAgent), windows);
    usage.observed_at = Some(now);
    usage.origin = Some("dimCLI".into());
    usage.plan = root
        .pointer("/subscription/product/name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    usage
}
fn fraction_for_token(row: &Value) -> Option<f64> {
    fraction(row, "window_token_used", "window_token_cap")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn packs_and_subscription_keep_separate_denominators_and_expiry() {
        let usage = reading(
            &json!({"subscription":{"product":{"name":"Pro"}},"credits":{
                "subscription_bucket":{"id":1,"status":"active","total_units":100,"used_units":20,"remaining_units":80,"expires_at":"2027-01-01T00:00:00Z","window_states":[{"id":7,"window_duration_hours":5,"window_token_cap":1000,"window_token_used":250}]},
                "addon_buckets":[{"id":2,"status":"active","total_units":50,"used_units":10,"remaining_units":40,"expires_at":"2027-02-01T00:00:00Z"},{"id":2,"status":"active","total_units":50,"used_units":10}]
            }}),
            1,
        );
        assert_eq!(usage.windows.len(), 3);
        assert_eq!(usage.windows[0].used_fraction, 0.2);
        assert_eq!(usage.windows[0].resets_at, None);
        assert_eq!(usage.windows[0].next_expiry_amount, Some(80.0));
        assert_eq!(usage.windows[1].kind, Kind::FiveHour);
        assert_eq!(usage.windows[1].used_fraction, 0.25);
        assert_eq!(usage.windows[2].kind, Kind::TopUp);
    }
    #[test]
    fn expired_and_invalid_buckets_do_not_become_zero_usage() {
        let usage = reading(
            &json!({"credits":{"addon_buckets":[{"id":1,"status":"active","used_units":0,"total_units":0},{"id":2,"status":"active","used_units":2,"total_units":10,"remaining_units":8,"expires_at":"2020-01-01T00:00:00Z"}]}}),
            crate::timeutil::now_ms(),
        );
        assert_eq!(
            usage.state,
            crate::model::State::Unavailable(Unavailability::NoLimitsReported)
        );
        assert_eq!(
            exit_reason("not logged in; run dim auth login"),
            Unavailability::LocalLoginMissing
        );
        assert_eq!(exit_reason("HTTP 429"), Unavailability::RateLimited);
    }
}
