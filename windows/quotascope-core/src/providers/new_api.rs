//! New API compatible gateway billing routes.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::gateway;
use crate::http::{number, HttpClient};
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, Unavailability};
use std::sync::Arc;

const UNLIMITED: f64 = 100_000_000.0;

pub struct NewApiService {
    http: Arc<HttpClient>,
}
impl NewApiService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for NewApiService {
    fn provider(&self) -> Provider {
        Provider::NewApi
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }
    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::NewApi);
        let Some(key) = pasted_or_none(keys.api_key(Provider::NewApi)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let Some(address) = keys
            .address(Provider::NewApi)
            .filter(|s| !s.trim().is_empty())
        else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressMissing);
        };
        let Some(subscription_url) =
            gateway::url_from(&address, "/v1/dashboard/billing/subscription", &["/v1"])
        else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        let Some(usage_url) = gateway::url_from(&address, "/v1/dashboard/billing/usage", &["/v1"])
        else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        let Some(status_url) = gateway::url_from(&address, "/api/status", &["/v1"]) else {
            return ProviderUsage::unavailable(account, Unavailability::ServerAddressRefused);
        };
        let auth = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let auth_refs: Vec<(&str, &str)> = auth.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let subscription = match self.http.fetch_json(
            crate::http::Method::Get,
            &subscription_url.to_string(),
            &auth_refs,
            None,
        ) {
            Ok(v) => v,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        if is_unlimited(&subscription) {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        }
        let usage = match self.http.fetch_json(
            crate::http::Method::Get,
            &usage_url.to_string(),
            &auth_refs,
            None,
        ) {
            Ok(v) => v,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let status = self
            .http
            .fetch_json(
                crate::http::Method::Get,
                &status_url.to_string(),
                &[("Accept", "application/json")],
                None,
            )
            .ok();
        let Some(remaining) = remaining(&subscription, &usage) else {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        };
        let Some(currency) = currency(status.as_ref()) else {
            return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
        };
        let mut result = ProviderUsage::live_now(account, Vec::new());
        result.credit_remaining = Some(CreditAmount {
            amount: remaining,
            currency: currency.clone(),
        });
        result.credit_balance = Some(format!("{currency} {remaining:.2}"));
        result.plan = status
            .as_ref()
            .and_then(|v| v.pointer("/data/system_name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        result.origin = Some(self.origin_token().into());
        result
    }
}

fn is_unlimited(subscription: &serde_json::Value) -> bool {
    ["hard_limit_usd", "soft_limit_usd", "system_hard_limit_usd"]
        .iter()
        .all(|key| subscription.get(*key).and_then(number) == Some(UNLIMITED))
}

fn remaining(subscription: &serde_json::Value, usage: &serde_json::Value) -> Option<f64> {
    let total = subscription
        .get("hard_limit_usd")
        .and_then(number)
        .filter(|v| v.is_finite())?;
    let used = usage
        .get("total_usage")
        .and_then(number)
        .filter(|v| v.is_finite())?;
    Some(total - used / 100.0)
}

fn currency(status: Option<&serde_json::Value>) -> Option<String> {
    let data = status?.get("data")?;
    if let Some(kind) = data.get("quota_display_type").and_then(|v| v.as_str()) {
        let upper = kind.to_ascii_uppercase();
        if ["USD", "CNY", "EUR", "GBP", "JPY"].contains(&upper.as_str()) {
            return Some(upper);
        }
        return None;
    }
    if data.get("display_in_currency").and_then(|v| v.as_bool()) == Some(true) {
        Some("USD".into())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usage_is_in_hundredths() {
        let subscription = serde_json::json!({"hard_limit_usd": 25.0});
        let usage = serde_json::json!({"total_usage": 1234.5});
        assert_eq!(remaining(&subscription, &usage), Some(12.655));
    }
}
