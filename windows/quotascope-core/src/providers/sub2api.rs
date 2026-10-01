//! Usage accounting for a user supplied sub2api deployment.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::gateway;
use crate::http::{number, string_field, HttpClient};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use std::sync::Arc;

pub struct Sub2ApiService {
    http: Arc<HttpClient>,
}

impl Sub2ApiService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for Sub2ApiService {
    fn provider(&self) -> Provider {
        Provider::Sub2api
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Sub2api);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Sub2api)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let Some(address) = keys
            .address(Provider::Sub2api)
            .filter(|s| !s.trim().is_empty())
        else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressMissing);
        };
        let Some(endpoint) = gateway::url_from(&address, "/v1/usage", &["/v1", "/v1/usage"]) else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply =
            match self
                .http
                .fetch_json(crate::http::Method::Get, &endpoint.to_string(), &refs, None)
            {
                Ok(value) => value,
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            };
        if reply.get("isValid").and_then(|v| v.as_bool()) == Some(false) {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyRefused);
        }
        let windows = windows(&reply);
        let wallet = wallet(&reply);
        if windows.is_empty() && wallet.is_none() {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        }
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.plan = string_field(&reply, "planName")
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(wallet) = wallet {
            usage.credit_balance = Some(format!("{} {:.2}", wallet.currency, wallet.amount));
            usage.credit_remaining = Some(wallet);
        }
        usage.origin = Some(self.origin_token().into());
        usage
    }
}

fn fraction(used: Option<f64>, limit: Option<f64>) -> Option<f64> {
    let limit = limit.filter(|v| v.is_finite() && *v > 0.0)?;
    let used = used.filter(|v| v.is_finite())?;
    Some((used / limit).clamp(0.0, 1.0))
}

fn is_spent(remaining: Option<f64>) -> bool {
    remaining.is_some_and(|v| v.is_finite() && v <= 0.0)
}

fn window_period(text: &str) -> Option<(Kind, i64)> {
    let text = text.trim().to_ascii_lowercase();
    let count: i64 = text.get(..text.len().saturating_sub(1))?.parse().ok()?;
    if count <= 0 {
        return None;
    }
    match text.chars().last()? {
        'h' => Some((
            if count == 5 {
                Kind::FiveHour
            } else {
                Kind::Other(count * 3_600)
            },
            count * 3_600,
        )),
        'd' => Some((
            if count == 7 {
                Kind::Weekly
            } else if count == 1 {
                Kind::Other(86_400)
            } else {
                Kind::Other(count * 86_400)
            },
            count * 86_400,
        )),
        _ => None,
    }
}

fn windows(reply: &serde_json::Value) -> Vec<UsageWindow> {
    let mut out = Vec::new();
    for (index, item) in crate::http::array_field(reply, "rate_limits")
        .iter()
        .enumerate()
    {
        let Some(label) = string_field(item, "window")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some((kind, seconds)) = window_period(label) else {
            continue;
        };
        let Some(used) = fraction(
            item.get("used").and_then(number),
            item.get("limit").and_then(number),
        ) else {
            continue;
        };
        let mut window = UsageWindow::new(
            &format!("rate.{}", label.to_ascii_lowercase()),
            kind,
            None,
            used,
            seconds,
            item.get("reset_at")
                .and_then(|v| v.as_str())
                .and_then(crate::timeutil::parse_iso8601_ms),
        );
        window.is_exhausted = is_spent(item.get("remaining").and_then(number));
        if out.iter().any(|w: &UsageWindow| w.id == window.id) {
            window.id = format!("rate.{}.{}", label.to_ascii_lowercase(), index);
        }
        out.push(window);
    }
    if let Some(quota) = reply.get("quota") {
        if let Some(used) = fraction(
            quota.get("used").and_then(number),
            quota.get("limit").and_then(number),
        ) {
            let mut window = UsageWindow::new("quota", Kind::Spend, None, used, 30 * 86_400, None);
            window.reports_length = false;
            window.is_exhausted = is_spent(quota.get("remaining").and_then(number));
            out.push(window);
        }
    }
    if let Some(subscription) = reply.get("subscription") {
        for (id, used_key, limit_key, seconds, kind) in [
            (
                "daily",
                "daily_usage_usd",
                "daily_limit_usd",
                86_400,
                Kind::Other(86_400),
            ),
            (
                "weekly",
                "weekly_usage_usd",
                "weekly_limit_usd",
                7 * 86_400,
                Kind::Weekly,
            ),
            (
                "monthly",
                "monthly_usage_usd",
                "monthly_limit_usd",
                30 * 86_400,
                Kind::Monthly,
            ),
        ] {
            if let Some(used) = fraction(
                subscription.get(used_key).and_then(number),
                subscription.get(limit_key).and_then(number),
            ) {
                let mut window = UsageWindow::new(
                    &format!("subscription.{id}"),
                    kind,
                    None,
                    used,
                    seconds,
                    None,
                );
                window.reports_length = false;
                out.push(window);
            }
        }
    }
    out.sort_by_key(|w| w.window_seconds);
    out
}

fn wallet(reply: &serde_json::Value) -> Option<CreditAmount> {
    let wallet_only = reply.get("quota").is_none() && reply.get("subscription").is_none();
    let amount = reply.get("balance").and_then(number).or_else(|| {
        wallet_only
            .then(|| reply.get("remaining").and_then(number))
            .flatten()
    })?;
    if !amount.is_finite() {
        return None;
    }
    let unit = reply
        .get("unit")
        .and_then(|v| v.as_str())
        .or_else(|| reply.pointer("/quota/unit").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("USD")
        .to_ascii_uppercase();
    if unit.len() != 3 || !unit.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(CreditAmount {
        amount,
        currency: unit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_rate_and_wallet_shapes() {
        let value = serde_json::json!({"rate_limits":[{"window":"5h","limit":100,"used":10,"remaining":90}], "balance": 4.0, "unit":"USD"});
        assert_eq!(windows(&value).len(), 1);
        assert_eq!(wallet(&value).unwrap().amount, 4.0);
    }
}
