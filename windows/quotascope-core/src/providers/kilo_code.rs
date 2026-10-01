//! Kilo Code: the account's credit balance, and the Kilo Pass allowance for
//! the current billing period when the account has a pass.
//!
//! Read from the tRPC routes Kilo's own dashboard calls, in one batched GET:
//! `https://app.kilo.ai/api/trpc/user.getCreditBlocks,kiloPass.getState`. The
//! shape is second-hand — taken from CodexBar's Kilo provider and its tests,
//! not from a captured reply.
//!
//! **What is not read.** The credit blocks' summed start against what is left
//! of them would draw a ring nobody sells — the blocks start and expire at
//! different times — so the balance Kilo reports is shown as a balance
//! instead. Only the field names seen in a Kilo reply are read.
//!
//! On macOS Pulse also falls back to the Kilo CLI's saved login
//! (`~/.local/share/kilo/auth.json`); QuotaScope keeps no CLI logins, so a
//! missing key is simply a missing key.

use super::{pasted_or_none, KeyRing, ProviderService};
use crate::http::{array_field, number_field, string_field, HttpClient};
use crate::model::{
    AccountKey, CreditAmount, Kind, Provider, ProviderUsage, Unavailability, UsageWindow,
};
use std::sync::Arc;

/// The two procedures, batched, each with no input. The quotes get
/// percent-encoded by the URL parser, the same way Swift's URLComponents
/// encoded them.
const ENDPOINT: &str = "https://app.kilo.ai/api/trpc/user.getCreditBlocks,kiloPass.getState?batch=1&input={\"0\":{\"json\":null},\"1\":{\"json\":null}}";

pub struct KiloCodeService {
    http: Arc<HttpClient>,
}

impl KiloCodeService {
    pub fn new(http: Arc<HttpClient>) -> Self {
        Self { http }
    }
}

impl ProviderService for KiloCodeService {
    fn provider(&self) -> Provider {
        Provider::KiloCode
    }

    fn origin_token(&self) -> &'static str {
        "endpoint"
    }

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage {
        let account = AccountKey::primary(Provider::KiloCode);
        let Some(key) = pasted_or_none(keys.api_key(Provider::KiloCode)) else {
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
            Ok(reading) => {
                let mut usage = ProviderUsage::live_now(account, reading.windows);
                usage.plan = reading.plan;
                if let Some(amount) = reading.balance {
                    usage.credit_balance = Some(format!("${amount:.2}"));
                    usage.credit_remaining = Some(CreditAmount {
                        amount,
                        currency: "USD".into(),
                    });
                }
                usage.origin = Some(self.origin_token().into());
                usage
            }
            Err(reason) => ProviderUsage::unavailable(account, reason),
        }
    }
}

/// What one reply yields, kept apart from the HTTP plumbing so tests can feed
/// it fixtures directly.
#[derive(Debug)]
pub struct KiloReading {
    pub windows: Vec<UsageWindow>,
    pub plan: Option<String>,
    /// The account's spendable balance, in dollars.
    pub balance: Option<f64>,
}

/// The tiers Kilo sells, by the names its own pricing uses. Any other tier is
/// still a Kilo Pass, and is called only that.
const TIERS: [(&str, &str); 3] = [
    ("tier_19", "Starter"),
    ("tier_49", "Pro"),
    ("tier_199", "Expert"),
];

pub fn reading(root: &serde_json::Value) -> Result<KiloReading, Unavailability> {
    let Some(entries) = batch(root) else {
        return Err(Unavailability::UnreadableReply);
    };
    // A procedure answering with an error: refused if it says so, and
    // otherwise nothing QuotaScope can read. Each carries its own, and the
    // first one in batch order wins.
    for entry in entries.iter().flatten() {
        if let Some(error) = entry.get("error") {
            let text = error.to_string().to_lowercase();
            return Err(
                if text.contains("unauthorized") || text.contains("forbidden") {
                    Unavailability::ApiKeyRefused
                } else {
                    Unavailability::UnreadableReply
                },
            );
        }
    }
    if !entries
        .iter()
        .flatten()
        .any(|entry| entry.get("result").is_some() || entry.get("error").is_some())
    {
        return Err(Unavailability::UnreadableReply);
    }

    let blocks = entries[0].and_then(payload);
    let pass = entries[1].and_then(payload);

    let balance = blocks.and_then(balance_of);
    let subscription = pass
        .and_then(|pass| pass.get("subscription"))
        .filter(|value| value.is_object());
    let windows: Vec<UsageWindow> = subscription.and_then(pass_window).into_iter().collect();

    if windows.is_empty() && balance.is_none() {
        return Err(Unavailability::NoLimitsReported);
    }

    let plan = subscription.map(|subscription| {
        let tier = string_field(subscription, "tier");
        match tier.map(tier_name) {
            Some(name) => name.to_string(),
            None => "Kilo Pass".to_string(),
        }
    });
    Ok(KiloReading {
        windows,
        plan,
        balance,
    })
}

fn tier_name(tier: &str) -> &'static str {
    TIERS
        .iter()
        .find(|(id, _)| *id == tier)
        .map(|(_, name)| *name)
        .unwrap_or("Kilo Pass")
}

/// The batch's replies in procedure order, as a JSON array or as an object
/// keyed by index. A procedure with no reply is `None`.
fn batch(root: &serde_json::Value) -> Option<[Option<&serde_json::Value>; 2]> {
    match root {
        serde_json::Value::Array(items) => Some([
            items.first().filter(|entry| entry.is_object()),
            items.get(1).filter(|entry| entry.is_object()),
        ]),
        serde_json::Value::Object(_) => Some([
            root.get("0").filter(|entry| entry.is_object()),
            root.get("1").filter(|entry| entry.is_object()),
        ]),
        _ => None,
    }
}

/// `result.data`, or `result.data.json` where the router wraps it.
fn payload(entry: &serde_json::Value) -> Option<&serde_json::Value> {
    let data = entry.get("result")?.get("data")?;
    if !data.is_object() {
        return None;
    }
    match data.get("json") {
        Some(json) if json.is_object() => Some(json),
        // The wrapper's `json` key is there but holds nothing readable.
        Some(_) => None,
        None => Some(data),
    }
}

/// In micro-dollars on the wire. The account's total where Kilo states it;
/// otherwise what is left in each block, added up.
pub fn balance_of(blocks: &serde_json::Value) -> Option<f64> {
    let micro = match number_field(blocks, "totalBalance_mUsd") {
        Some(total) => Some(total),
        None => {
            let left: Vec<f64> = array_field(blocks, "creditBlocks")
                .iter()
                .filter_map(|block| number_field(block, "balance_mUsd"))
                .collect();
            (!left.is_empty()).then(|| left.iter().sum())
        }
    };
    let micro = micro.filter(|value| value.is_finite() && *value >= 0.0)?;
    Some(micro / 1_000_000.0)
}

/// The pass's period: what has been used of the base credits and the bonus on
/// top, both as Kilo reports them. A period with no stated size draws
/// nothing. It resets when the pass bills again; how long that is is not
/// stated, so the thirty days are a sort key only.
pub fn pass_window(subscription: &serde_json::Value) -> Option<UsageWindow> {
    let used = number_field(subscription, "currentPeriodUsageUsd")
        .filter(|value| value.is_finite() && *value >= 0.0)?;
    let base = number_field(subscription, "currentPeriodBaseCreditsUsd")
        .filter(|value| value.is_finite() && *value >= 0.0)?;
    let bonus = number_field(subscription, "currentPeriodBonusCreditsUsd")
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.0);
    let size = base + bonus;
    if size <= 0.0 {
        return None;
    }
    let mut window = UsageWindow::new(
        "kilocode.pass",
        Kind::Credits,
        None,
        used / size,
        30 * 86_400,
        string_field(subscription, "nextBillingAt").and_then(crate::timeutil::parse_iso8601_ms),
    );
    window.reports_length = false;
    window.is_exhausted = used >= size;
    Some(window)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wrapped(data: serde_json::Value) -> serde_json::Value {
        json!({ "result": { "data": { "json": data } } })
    }

    fn blocks_reply(total_micro: f64) -> serde_json::Value {
        wrapped(json!({ "totalBalance_mUsd": total_micro }))
    }

    fn pass_reply(subscription: serde_json::Value) -> serde_json::Value {
        wrapped(json!({ "subscription": subscription }))
    }

    #[test]
    fn array_batch_yields_balance_and_pass() {
        let reply = json!([
            blocks_reply(12_500_000.0),
            pass_reply(json!({
                "tier": "tier_49",
                "currentPeriodUsageUsd": 7.0,
                "currentPeriodBaseCreditsUsd": 25.0,
                "nextBillingAt": "2026-10-15T00:00:00Z",
            })),
        ]);
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Pro"));
        assert_eq!(reading.balance, Some(12.5));
        let window = &reading.windows[0];
        assert_eq!(window.id, "kilocode.pass");
        assert_eq!(window.used_fraction, 7.0 / 25.0);
        assert!(!window.reports_length);
        assert!(!window.is_exhausted);
        assert_eq!(
            window.resets_at,
            crate::timeutil::parse_iso8601_ms("2026-10-15T00:00:00Z")
        );
    }

    #[test]
    fn object_keyed_batch_reads_the_same() {
        let reply = json!({
            "0": blocks_reply(1_000_000.0),
            "1": pass_reply(json!({
                "currentPeriodUsageUsd": 36.0,
                "currentPeriodBaseCreditsUsd": 25.0,
                "currentPeriodBonusCreditsUsd": 5.0,
            })),
        });
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.plan.as_deref(), Some("Kilo Pass"));
        let window = &reading.windows[0];
        // Over 100% stands: a provider may report past a full ring, and the
        // exhausted flag is Kilo's own arithmetic, not a clamp.
        assert_eq!(window.used_fraction, 1.2);
        assert!(window.is_exhausted);
    }

    #[test]
    fn bonus_credits_widen_the_denominator_only_when_positive() {
        let reply = json!([
            blocks_reply(0.0),
            pass_reply(json!({
                "currentPeriodUsageUsd": 12.0,
                "currentPeriodBaseCreditsUsd": 10.0,
                "currentPeriodBonusCreditsUsd": 10.0,
            })),
        ]);
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.windows[0].used_fraction, 12.0 / 20.0);
    }

    #[test]
    fn balance_falls_back_to_what_is_left_of_the_blocks() {
        let reply = json!([
            wrapped(json!({ "creditBlocks": [
                { "balance_mUsd": 1_500_000.0 },
                { "balance_mUsd": 250_000.0 },
            ] })),
            pass_reply(json!({})),
        ]);
        let reading = reading(&reply).unwrap();
        assert_eq!(reading.balance, Some(1.75));
        // A subscription with no stated size draws no window, but the balance
        // still stands on its own.
        assert!(reading.windows.is_empty());
    }

    #[test]
    fn negative_or_unstated_balance_is_no_balance() {
        assert_eq!(balance_of(&json!({ "totalBalance_mUsd": -1.0 })), None);
        assert_eq!(balance_of(&json!({ "totalBalance_mUsd": "nope" })), None);
        assert_eq!(balance_of(&json!({ "creditBlocks": [] })), None);
    }

    #[test]
    fn an_error_procedure_refuses_the_credential_in_its_own_words() {
        let reply = json!([
            { "error": { "code": "UNAUTHORIZED", "message": "bad key" } },
            pass_reply(json!({})),
        ]);
        assert_eq!(reading(&reply).unwrap_err(), Unavailability::ApiKeyRefused);
        let other = json!([{ "error": { "code": "INTERNAL_SERVER_ERROR" } }]);
        assert_eq!(
            reading(&other).unwrap_err(),
            Unavailability::UnreadableReply
        );
    }

    #[test]
    fn a_reply_with_no_procedure_answers_is_unreadable() {
        assert_eq!(
            reading(&json!([{}, {}])).unwrap_err(),
            Unavailability::UnreadableReply
        );
        assert_eq!(
            reading(&json!("not a batch")).unwrap_err(),
            Unavailability::UnreadableReply
        );
    }

    #[test]
    fn neither_balance_nor_pass_is_no_limits() {
        let reply = json!([wrapped(json!({})), wrapped(json!({}))]);
        assert_eq!(
            reading(&reply).unwrap_err(),
            Unavailability::NoLimitsReported
        );
    }

    #[test]
    fn unknown_tiers_are_still_a_kilo_pass() {
        assert_eq!(tier_name("tier_19"), "Starter");
        assert_eq!(tier_name("tier_199"), "Expert");
        assert_eq!(tier_name("tier_999"), "Kilo Pass");
    }
}
