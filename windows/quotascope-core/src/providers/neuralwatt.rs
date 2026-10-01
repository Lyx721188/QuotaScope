//! Neuralwatt, inference priced by energy: a subscription's kilowatt-hour
//! allowance for the current period, the key's own spending allowance where
//! one is set, and the prepaid balance in US dollars.
//!
//! Read with an API key the user pastes, from
//! `GET https://api.neuralwatt.com/v1/quota`. The shape is second-hand —
//! taken from CodexBar's Neuralwatt provider and its tests, not from a
//! captured reply — and the tests here say so.
//!
//! The prepaid balance and the subscription are separate things: credits do
//! not reset and are spent as you go, the allowance is billed against kWh and
//! turns over with the period. Neither is folded into the other. A key that
//! has been blocked is not drawn as a full ring — the flag is not a figure —
//! and the month's spend, which has no limit beside it, is left out.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{number_field, object_field, string_field, HttpClient, Method};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use std::sync::Arc;

const ENDPOINT: &str = "https://api.neuralwatt.com/v1/quota";

/// The sort key for a period whose length is not stated.
const THIRTY_DAYS: i64 = 30 * 86_400;

pub struct NeuralwattService {
    http: Arc<HttpClient>,
}

impl NeuralwattService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for NeuralwattService {
    fn provider(&self) -> Provider {
        Provider::Neuralwatt
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::Neuralwatt);
        let Some(key) = pasted_or_none(keys.api_key(Provider::Neuralwatt)) else {
            return ProviderUsage::unavailable(account, Unavailability::ApiKeyMissing);
        };
        let headers = [
            ("Authorization", format!("Bearer {key}")),
            ("Accept", "application/json".into()),
        ];
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let reply = match self.http.fetch_json(Method::Get, ENDPOINT, &refs, None) {
            Ok(value) => value,
            Err(reason) => return ProviderUsage::unavailable(account, reason),
        };
        let mut usage = reading(&reply, account);
        if matches!(usage.state, crate::model::State::Live) {
            usage.origin = Some(self.origin_token().into());
        }
        usage
    }
}

pub fn reading(reply: &serde_json::Value, account: AccountKey) -> ProviderUsage {
    let Some(balance) = object_field(reply, "balance") else {
        return ProviderUsage::unavailable(account, Unavailability::UnreadableReply);
    };

    // The subscription first: it is what the plan is, and it drives the ring.
    // The key's own allowance, where set, beside it.
    let mut windows = Vec::new();
    let plan = object_field(reply, "subscription").and_then(plan_name);
    if let Some(subscription) = object_field(reply, "subscription") {
        if let Some(window) = subscription_window(subscription) {
            windows.push(window);
        }
    }
    if let Some(allowance) =
        object_field(reply, "key").and_then(|key| object_field(key, "allowance"))
    {
        if let Some(window) = key_allowance(allowance) {
            windows.push(window);
        }
    }

    let prepaid = remaining(balance);
    if windows.is_empty() && prepaid.is_none() {
        return ProviderUsage::unavailable(account, Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(account, windows);
    usage.plan = plan;
    usage.credit_balance = prepaid.map(|amount| money_text(amount, "USD"));
    usage.credit_remaining = prepaid.map(|amount| CreditAmount {
        amount,
        currency: "USD".into(),
    });
    usage
}

/// The subscription's kWh for this period. The allowance is the one stated,
/// or failing that what was used plus what is left — both stated.
fn subscription_window(subscription: &serde_json::Value) -> Option<UsageWindow> {
    let used = non_negative(number_field(subscription, "kwh_used"))?;
    let included = positive(number_field(subscription, "kwh_included")).or_else(|| {
        non_negative(number_field(subscription, "kwh_remaining"))
            .and_then(|left| positive(Some(used + left)))
    })?;

    // The period is stated by its two ends, so its length may be divided by.
    // Without both, thirty days is only where it sorts.
    let start = string_field(subscription, "current_period_start")
        .and_then(crate::timeutil::parse_iso8601_ms);
    let end = string_field(subscription, "current_period_end")
        .and_then(crate::timeutil::parse_iso8601_ms);
    let stated = start
        .and_then(|start| end.map(|end| (end - start) / 1000))
        .filter(|seconds| *seconds > 0);
    // A billing month is named for itself; anything else takes its length
    // from the two ends, or gets no length at all. The Windows model has no
    // credits kind, so a lengthless allowance carries the month as its kind
    // and sort key.
    let kind = match string_field(subscription, "billing_interval")
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("month") | Some("monthly") => Kind::Monthly,
        _ => match stated {
            Some(seconds) => Kind::Other(seconds),
            None => Kind::Other(THIRTY_DAYS),
        },
    };
    let fraction = used / included;
    let mut window = UsageWindow::new(
        "neuralwatt.subscription",
        kind,
        None,
        fraction,
        stated.unwrap_or(THIRTY_DAYS),
        end,
    );
    window.reports_length = stated.is_some();
    window.is_exhausted = fraction >= 1.0;
    Some(window)
}

/// The key's own spending allowance, where one is set.
fn key_allowance(allowance: &serde_json::Value) -> Option<UsageWindow> {
    let spent = non_negative(number_field(allowance, "spent_usd"))?;
    let limit = positive(number_field(allowance, "limit_usd"))?;
    let length = match string_field(allowance, "period")
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("daily") | Some("day") => (86_400, true),
        Some("weekly") | Some("week") => (7 * 86_400, true),
        _ => (THIRTY_DAYS, false),
    };
    let fraction = spent / limit;
    let mut window = UsageWindow::new(
        "neuralwatt.key_allowance",
        Kind::Spend,
        None,
        fraction,
        length.0,
        None,
    );
    window.reports_length = length.1;
    window.is_exhausted = fraction >= 1.0;
    Some(window)
}

/// Prepaid credit left, as stated, or the total less what was used when only
/// those two are.
fn remaining(balance: &serde_json::Value) -> Option<f64> {
    if let Some(left) = non_negative(number_field(balance, "credits_remaining_usd")) {
        return Some(left);
    }
    let total = non_negative(number_field(balance, "total_credits_usd"))?;
    let used = non_negative(number_field(balance, "credits_used_usd"))?;
    Some((total - used).max(0.0))
}

/// The subscription's plan, as Neuralwatt writes it: "pro_plan" reads
/// "Pro Plan".
fn plan_name(subscription: &serde_json::Value) -> Option<String> {
    let plan = string_field(subscription, "plan")
        .map(str::trim)
        .filter(|plan| !plan.is_empty())?;
    Some(capitalized(&plan.replace('_', " ")))
}

fn non_negative(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

fn positive(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::State;

    fn reading_for(value: serde_json::Value) -> ProviderUsage {
        reading(&value, AccountKey::primary(Provider::Neuralwatt))
    }

    #[test]
    fn a_subscription_and_the_prepaid_balance_read_together() {
        let usage = reading_for(serde_json::json!({
            "balance": {"credits_remaining_usd": 4.25},
            "subscription": {
                "plan": "pro_plan",
                "billing_interval": "month",
                "current_period_start": "2026-09-01T00:00:00Z",
                "current_period_end": "2026-10-01T00:00:00Z",
                "kwh_included": 100.0,
                "kwh_used": 40.0,
            },
        }));
        assert!(matches!(usage.state, State::Live));
        assert_eq!(usage.plan.as_deref(), Some("Pro Plan"));
        assert_eq!(usage.windows.len(), 1);

        let window = &usage.windows[0];
        assert_eq!(window.id, "neuralwatt.subscription");
        assert_eq!(window.kind, Kind::Monthly);
        assert_eq!(window.used_fraction, 0.4);
        assert_eq!(window.window_seconds, THIRTY_DAYS);
        assert!(
            window.reports_length,
            "the period is stated by its two ends"
        );
        assert_eq!(
            window.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-01T00:00:00Z")
        );

        assert_eq!(usage.credit_balance.as_deref(), Some("$4.25"));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 4.25,
                currency: "USD".into()
            })
        );
    }

    #[test]
    fn the_allowance_falls_back_to_used_plus_remaining() {
        let usage = reading_for(serde_json::json!({
            "balance": {"credits_remaining_usd": 1.0},
            "subscription": {"kwh_used": 30.0, "kwh_remaining": 70.0, "billing_interval": "hourly"},
        }));
        let window = &usage.windows[0];
        assert_eq!(window.used_fraction, 0.3);
        // Neither end stated, so the length is not claimed and the kind is
        // not a month's.
        assert!(!window.reports_length);
        assert_eq!(window.kind, Kind::Other(THIRTY_DAYS));
        assert!(window.resets_at.is_none());
    }

    #[test]
    fn a_stated_period_names_its_own_length() {
        let usage = reading_for(serde_json::json!({
            "balance": {},
            "subscription": {
                "kwh_used": 1.0,
                "kwh_included": 4.0,
                "current_period_start": "2026-09-10T00:00:00Z",
                "current_period_end": "2026-09-12T00:00:00Z",
            },
        }));
        let window = &usage.windows[0];
        assert_eq!(window.kind, Kind::Other(2 * 86_400));
        assert!(window.reports_length);
        assert_eq!(window.window_seconds, 2 * 86_400);
    }

    #[test]
    fn the_key_allowance_is_a_spend_limit() {
        for (period, seconds, stated) in [
            ("daily", 86_400, true),
            ("week", 7 * 86_400, true),
            (" fortnight ", THIRTY_DAYS, false),
        ] {
            let usage = reading_for(serde_json::json!({
                "balance": {"credits_remaining_usd": 1.0},
                "key": {"allowance": {"spent_usd": 2.0, "limit_usd": 8.0, "period": period}},
            }));
            let window = usage
                .windows
                .iter()
                .find(|window| window.id == "neuralwatt.key_allowance")
                .unwrap();
            assert_eq!(window.kind, Kind::Spend);
            assert_eq!(window.used_fraction, 0.25);
            assert_eq!(window.window_seconds, seconds);
            assert_eq!(window.reports_length, stated);
            assert!(window.resets_at.is_none());
        }
    }

    #[test]
    fn the_balance_falls_back_to_total_less_used() {
        let usage = reading_for(serde_json::json!({
            "balance": {"total_credits_usd": 20.0, "credits_used_usd": 15.75},
        }));
        assert_eq!(
            usage.credit_remaining,
            Some(CreditAmount {
                amount: 4.25,
                currency: "USD".into()
            })
        );

        // Overdrawn is empty, not negative.
        let usage = reading_for(serde_json::json!({
            "balance": {"total_credits_usd": 5.0, "credits_used_usd": 9.0},
        }));
        assert_eq!(
            usage.credit_remaining.map(|credit| credit.amount),
            Some(0.0)
        );
    }

    #[test]
    fn a_used_allowance_without_a_limit_draws_nothing() {
        let usage = reading_for(serde_json::json!({
            "balance": {"credits_remaining_usd": 1.0},
            "subscription": {"kwh_used": 5.0},
            "key": {"allowance": {"spent_usd": 3.0}},
        }));
        assert!(usage.windows.is_empty());
        assert_eq!(usage.credit_balance.as_deref(), Some("$1.00"));
    }

    #[test]
    fn no_balance_object_is_unreadable() {
        assert!(matches!(
            reading_for(serde_json::json!({})).state,
            State::Unavailable(Unavailability::UnreadableReply)
        ));
    }

    #[test]
    fn nothing_reported_is_an_answer() {
        assert_eq!(
            reading_for(serde_json::json!({"balance": {}})),
            ProviderUsage::unavailable(
                AccountKey::primary(Provider::Neuralwatt),
                Unavailability::NoLimitsReported
            )
        );
    }
}
