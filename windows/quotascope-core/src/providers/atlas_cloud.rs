//! Atlas Cloud's available balance, in the currency the service states, read
//! with a key from `GET https://api.atlascloud.ai/public/v1/balance`.
//!
//! Reading it needs a key with account-balance permission: the account
//! owner's, or a team's Account Admin or Finance key. A balance, not an
//! allowance — so there is no ring, only the credit line. Coding Plan quotas
//! are a separate meter behind another route and are not read. A negative
//! balance is kept as reported: it is money owed. The reply's shape is
//! second-hand (taken from CodexBar's Atlas Cloud plugin and its docs, not
//! from a captured reply), which is one reason the reader stays strict.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::HttpClient;
use crate::model::{AccountKey, CreditAmount, Provider, ProviderUsage, State, Unavailability};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.atlascloud.ai/public/v1/balance";

pub struct AtlasCloudService {
    http: Arc<HttpClient>,
}

impl AtlasCloudService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for AtlasCloudService {
    fn provider(&self) -> Provider {
        Provider::AtlasCloud
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::AtlasCloud);
        let Some(key) = pasted_or_none(keys.api_key(Provider::AtlasCloud)) else {
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

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    // The envelope names the object and its scope; the amount arrives as a
    // decimal string and the currency as the ISO code the service states.
    // Anything else is a reply this reader does not understand — never a
    // balance read as zero, which would draw a spent account.
    let available = reply.get("available");
    let Some(value_text) = available
        .and_then(|available| available.get("value"))
        .and_then(|value| value.as_str())
        .map(str::trim)
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let Some(amount) = value_text
        .parse::<f64>()
        .ok()
        .filter(|amount| amount.is_finite())
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    let Some(currency) = available
        .and_then(|available| available.get("currency"))
        .and_then(|currency| currency.as_str())
        .map(str::trim)
        .map(|currency| currency.to_uppercase())
        .filter(|currency| {
            currency.chars().count() == 3 && currency.chars().all(|c| c.is_alphabetic())
        })
    else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };
    if crate::http::string_field(reply, "object") != Some("balance")
        || crate::http::string_field(reply, "scope") != Some("account")
    {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    }

    let mut usage = ProviderUsage::live_now(account, Vec::new());
    usage.credit_balance = Some(money_text(amount, &currency));
    usage.credit_remaining = Some(CreditAmount { amount, currency });
    usage
}

/// The balance as money — symbol first, cents kept. The amount also rides
/// beside it as a number, for anything that has to compare.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::AtlasCloud))
    }

    fn balance(value: &str, currency: &str) -> serde_json::Value {
        serde_json::json!({
            "object": "balance",
            "scope": "account",
            "available": {"value": value, "currency": currency},
        })
    }

    #[test]
    fn balance_reads_live_without_a_ring() {
        let usage = reading_for(balance(" 12.34 ", "usd"));
        assert!(matches!(usage.state, State::Live));
        assert!(usage.windows.is_empty());
        assert_eq!(usage.credit_balance.as_deref(), Some("$12.34"));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 12.34,
                currency: "USD".into(),
            })
        );
    }

    #[test]
    fn wrong_object_or_scope_is_unreadable() {
        let mut wrong_object = balance("12.34", "USD");
        wrong_object["object"] = serde_json::json!("other.thing");
        let mut wrong_scope = balance("12.34", "USD");
        wrong_scope["scope"] = serde_json::json!("team");
        for reply in [wrong_object, wrong_scope] {
            let usage = reading_for(reply);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn amount_must_arrive_as_a_string() {
        let reply = serde_json::json!({
            "object": "balance",
            "scope": "account",
            "available": {"value": 12.34, "currency": "USD"},
        });
        let usage = reading_for(reply);
        assert!(matches!(
            usage.state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }

    #[test]
    fn currency_must_be_three_letters() {
        for currency in ["USDX", "U3D", "us d", ""] {
            let usage = reading_for(balance("12.34", currency));
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn missing_amount_or_currency_is_unreadable_not_zero() {
        for reply in [
            serde_json::json!({"object": "balance", "scope": "account", "available": {}}),
            serde_json::json!({"object": "balance", "scope": "account"}),
            balance("not a number", "USD"),
            balance("inf", "USD"),
        ] {
            let usage = reading_for(reply);
            assert!(matches!(
                usage.state,
                State::Unavailable(Unavailability::UnreadableReply)
            ));
        }
    }

    #[test]
    fn negative_balance_is_kept_as_owed() {
        let usage = reading_for(balance("-5.00", "USD"));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: -5.0,
                currency: "USD".into(),
            })
        );
    }
}
