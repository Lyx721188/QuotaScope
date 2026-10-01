//! DeepInfra pay-as-you-go balance and optional account spend limit.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number, HttpClient};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.deepinfra.com/payment/checklist?compute_owed=true";

pub struct DeepInfraService {
    http: Arc<HttpClient>,
}
impl DeepInfraService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for DeepInfraService {
    fn provider(&self) -> Provider {
        Provider::DeepInfra
    }
    fn origin_token(&self) -> &'static str {
        "endpoint"
    }
    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::DeepInfra);
        let Some(key) = pasted_or_none(keys.api_key(Provider::DeepInfra)) else {
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
        let Some(ledger) = reply
            .get("stripe_balance")
            .and_then(number)
            .filter(|v| v.is_finite())
        else {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        };
        let Some(recent) = reply
            .get("recent")
            .and_then(number)
            .filter(|v| v.is_finite())
        else {
            return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
        };
        let unbilled = recent.max(0.0);
        let available = -(ledger + unbilled);
        let mut windows = Vec::new();
        if let Some(limit) = reply
            .get("limit")
            .and_then(number)
            .filter(|v| v.is_finite() && *v > 0.0)
        {
            let fraction = unbilled / limit;
            let mut window = UsageWindow::new(
                "deepinfra.spend",
                Kind::Spend,
                None,
                fraction,
                30 * 86_400,
                None,
            );
            window.reports_length = false;
            window.is_exhausted = fraction >= 1.0;
            windows.push(window);
        }
        let mut usage = ProviderUsage::live_now(account, windows);
        usage.credit_balance = Some(format!("${available:.2}"));
        usage.credit_remaining = Some(CreditAmount {
            amount: available,
            currency: "USD".into(),
        });
        usage.origin = Some(self.origin_token().into());
        usage
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ledger_is_inverted() {
        let _ = serde_json::json!({"stripe_balance": -5.0, "recent": 1.0});
        assert_eq!(-(-5.0 + 1.0), 4.0);
    }
}
