//! Moonshot Open Platform balance, with international and mainland endpoints.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number, HttpClient};
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, Unavailability};
use std::sync::Arc;

const INTERNATIONAL: &str = "https://api.moonshot.ai/v1/users/me/balance";
const CHINA: &str = "https://api.moonshot.cn/v1/users/me/balance";

pub struct MoonshotService {
    http: Arc<HttpClient>,
}

impl MoonshotService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for MoonshotService {
    fn provider(&self) -> Provider {
        Provider::Moonshot
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Moonshot);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Moonshot)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let mut last = Unavailability::ApiKeyRefused;
        for (endpoint, currency) in [(INTERNATIONAL, "USD"), (CHINA, "CNY")] {
            match self
                .http
                .fetch_json(crate::http::Method::Get, endpoint, &refs, None)
            {
                Ok(reply) => return reading(reply, account, currency, self.origin_token()),
                Err(Unavailability::ApiKeyRefused) => last = Unavailability::ApiKeyRefused,
                Err(reason) => return ProviderUsage::unavailable(account, reason),
            }
        }
        ProviderUsage::unavailable(account, last)
    }
}

pub fn reading(
    reply: serde_json::Value,
    account: AccountKey,
    currency: &str,
    origin: &str,
) -> ProviderUsage {
    if reply.get("code").and_then(number) != Some(0.0)
        || reply.get("status").and_then(|v| v.as_bool()) != Some(true)
    {
        return ProviderUsage::unavailable(account, Unavailability::ServerError);
    }
    let Some(amount) = reply
        .pointer("/data/available_balance")
        .and_then(number)
        .filter(|v| v.is_finite())
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.origin = Some(origin.into());
    usage.credit_remaining = Some(CreditAmount {
        amount,
        currency: currency.into(),
    });
    usage.credit_balance = Some(format_money(amount, currency));
    usage
}

fn format_money(amount: f64, currency: &str) -> String {
    let symbol = match currency {
        "USD" => "$",
        "CNY" => "¥",
        _ => currency,
    };
    format!("{symbol}{amount:.2}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_currency_from_region() {
        let value =
            serde_json::json!({"code": 0, "status": true, "data": {"available_balance": "1.25"}});
        let reading = reading(
            value,
            AccountKey::primary(Provider::Moonshot),
            "USD",
            "endpoint",
        );
        assert_eq!(reading.credit_remaining.unwrap().amount, 1.25);
    }
}
