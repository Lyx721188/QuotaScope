//! Vercel AI Gateway: the team's remaining credit, in US dollars. There is no
//! allowance and no period, so there is no ring to draw — only a balance.
//!
//! Read with an AI Gateway API key the user pastes, from
//! `GET https://ai-gateway.vercel.sh/v1/credits`. The shape is second-hand —
//! taken from CodexBar's Vercel provider and Vercel's public API reference it
//! cites, not from a captured reply.
//!
//! The reply also carries lifetime spend. Spend with no limit beside it has
//! nowhere to go, so it is left out.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, Unavailability};
use std::sync::Arc;

const ENDPOINT: &str = "https://ai-gateway.vercel.sh/v1/credits";

pub struct VercelAiGatewayService {
    http: Arc<HttpClient>,
}

impl VercelAiGatewayService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for VercelAiGatewayService {
    fn provider(&self) -> Provider {
        Provider::VercelAiGateway
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::VercelAiGateway);
        let Some(key) = pasted_or_none(keys.api_key(Provider::VercelAiGateway)) else {
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
            Ok(amount) => {
                let mut usage = ProviderUsage::live_now(account, Vec::new());
                usage.credit_balance = Some(format!("${amount:.2}"));
                usage.credit_remaining = Some(CreditAmount {
                    amount,
                    currency: "USD".into(),
                });
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// Both the balance and the lifetime spend are decimal strings, "95.50", in
/// US dollars; only the balance is read. Zero and below are kept: an empty or
/// overdrawn team is a reading.
pub fn reading(reply: &serde_json::Value) -> Result<f64, Unavailability> {
    let text = match reply.get("balance") {
        Some(serde_json::Value::String(text)) => text.trim(),
        _ => return Err(Unavailability::UnreadableReply),
    };
    text.parse::<f64>()
        .ok()
        .filter(|amount| amount.is_finite())
        .ok_or(Unavailability::UnreadableReply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_decimal_string_balance_is_a_reading() {
        assert_eq!(reading(&json!({ "balance": "95.50" })), Ok(95.5));
        assert_eq!(reading(&json!({ "balance": " 8.10 " })), Ok(8.1));
        // Zero and below are kept: an empty or overdrawn team is a reading.
        assert_eq!(reading(&json!({ "balance": "0.00" })), Ok(0.0));
        assert_eq!(reading(&json!({ "balance": "-3.25" })), Ok(-3.25));
    }

    #[test]
    fn anything_but_a_decimal_string_is_unreadable() {
        assert_eq!(reading(&json!({})), Err(Unavailability::UnreadableReply));
        assert_eq!(
            reading(&json!({ "balance": null })),
            Err(Unavailability::UnreadableReply)
        );
        assert_eq!(
            reading(&json!({ "balance": 95.5 })),
            Err(Unavailability::UnreadableReply)
        );
        assert_eq!(
            reading(&json!({ "balance": "unlimited" })),
            Err(Unavailability::UnreadableReply)
        );
    }
}
