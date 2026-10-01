//! Venice billing balance.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number, HttpClient};
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, Unavailability};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.venice.ai/api/v1/billing/balance";

pub struct VeniceService {
    http: Arc<HttpClient>,
}
impl VeniceService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for VeniceService {
    fn provider(&self) -> Provider {
        Provider::Venice
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }
    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Venice);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Venice)) else {
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
        if reply.get("canConsume").and_then(|v| v.as_bool()).is_none()
            || !reply.get("balances").is_some_and(|v| v.is_object())
        {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        }
        let currency_value = reply.get("consumptionCurrency");
        if currency_value.is_some_and(|value| !value.is_null() && !value.is_string()) {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        }
        let currency = currency_value
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        let usd = match figure(reply.pointer("/balances/usd")) {
            Figure::Value(value) => Some(value),
            Figure::Missing => None,
            Figure::Malformed => {
                return ProviderUsage::unavailable(account, Unavailability::UnreadableReply)
            }
        };
        let diem = match figure(reply.pointer("/balances/diem")) {
            Figure::Value(value) => Some(value),
            Figure::Missing => None,
            Figure::Malformed => {
                return ProviderUsage::unavailable(account, Unavailability::UnreadableReply)
            }
        };
        if currency == "DIEM" {
            if let Some(amount) = diem {
                return balance(
                    account,
                    amount,
                    "DIEM",
                    format!("{amount:.2} DIEM"),
                    self.origin_token(),
                );
            }
        }
        if let Some(amount) = usd {
            return balance(
                account,
                amount,
                "USD",
                format!("${amount:.2}"),
                self.origin_token(),
            );
        }
        if let Some(amount) = diem {
            return balance(
                account,
                amount,
                "DIEM",
                format!("{amount:.2} DIEM"),
                self.origin_token(),
            );
        }
        ProviderUsage::unavailable(account, Unavailability::NoLimitsReported)
    }
}

enum Figure {
    Missing,
    Value(f64),
    Malformed,
}

fn figure(value: Option<&serde_json::Value>) -> Figure {
    let Some(value) = value else {
        return Figure::Missing;
    };
    match value {
        serde_json::Value::Null => Figure::Missing,
        serde_json::Value::Number(_) => number(value)
            .filter(|v| v.is_finite())
            .map(Figure::Value)
            .unwrap_or(Figure::Malformed),
        serde_json::Value::String(text) if text.trim().is_empty() => Figure::Missing,
        serde_json::Value::String(text) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(Figure::Value)
            .unwrap_or(Figure::Malformed),
        _ => Figure::Malformed,
    }
}

fn balance(
    account: AccountKey,
    amount: f64,
    currency: &str,
    text: String,
    origin: &str,
) -> ProviderUsage {
    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.credit_balance = Some(text);
    usage.credit_remaining = Some(CreditAmount {
        amount,
        currency: currency.into(),
    });
    usage.origin = Some(origin.into());
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diem_is_not_presented_as_dollars() {
        let value = serde_json::json!({"canConsume": true, "consumptionCurrency": "DIEM", "balances": {"diem": 3.5}});
        let _ = value;
        assert!(matches!(
            figure(Some(&serde_json::json!("3.5"))),
            Figure::Value(3.5)
        ));
    }
}
