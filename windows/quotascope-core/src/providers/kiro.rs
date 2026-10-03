//! Kiro subscription credit pools, through the CLI-owned ACP login.
//! Field names and semantics follow Pulse's KiroUsageService at 3696a65.
//! No Kiro token, database, terminal usage output or price is read here.

use super::{KeyRing, ProviderService};
use crate::kiro_acp::{self, Failure};
use crate::model::{AccountKey, Kind, Provider, ProviderUsage, Unavailability, UsageWindow};
use serde::Deserialize;
use std::collections::HashMap;

pub struct KiroService;

impl ProviderService for KiroService {
    fn provider(&self) -> Provider {
        Provider::Kiro
    }

    fn origin_token(&self) -> &'static str {
        "kiroACP"
    }

    fn fetch(&self, _keys: &KeyRing) -> ProviderUsage {
        match kiro_acp::usage() {
            Ok(reply) => reading(&reply),
            Err(failure) => unavailable(reason_for_failure(&failure)),
        }
    }
}

#[derive(Deserialize)]
struct Reply {
    success: bool,
    message: Option<String>,
    data: Option<Payload>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    plan_name: Option<String>,
    billing_cycle_reset: Option<String>,
    usage_breakdowns: Vec<Breakdown>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Breakdown {
    resource_type: Option<String>,
    display_name: Option<String>,
    used: Option<f64>,
    limit: Option<f64>,
    percentage: Option<f64>,
    has_limit: Option<bool>,
}

fn unavailable(reason: Unavailability) -> ProviderUsage {
    let mut usage = ProviderUsage::unavailable(AccountKey::primary(Provider::Kiro), reason);
    usage.origin = Some("kiroACP".into());
    usage
}

pub fn reading(reply: &serde_json::Value) -> ProviderUsage {
    reading_at(reply, crate::timeutil::now_ms())
}

/// A fixed observation time makes parsing fixtures independent of the clock.
pub fn reading_at(reply: &serde_json::Value, now_ms: i64) -> ProviderUsage {
    // Swift decodes the entire envelope. Type drift is not a missing figure.
    let Ok(reply) = serde_json::from_value::<Reply>(reply.clone()) else {
        return unavailable(Unavailability::UnreadableReply);
    };
    if !reply.success {
        return unavailable(reason_for_message(reply.message.as_deref()));
    }
    let Some(payload) = reply.data else {
        return unavailable(Unavailability::NoLimitsReported);
    };
    let reset = payload.billing_cycle_reset.as_deref().and_then(reset_date);
    let mut occurrences = HashMap::<String, usize>::new();
    let mut windows = Vec::new();
    for item in payload.usage_breakdowns {
        let Some(limit) = item.limit.filter(|v| v.is_finite() && *v > 0.0) else {
            continue;
        };
        if item.has_limit == Some(false) {
            continue;
        }
        let used = item.used.filter(|v| v.is_finite()).or_else(|| {
            item.percentage
                .filter(|v| v.is_finite())
                .map(|percent| percent / 100.0 * limit)
        });
        let Some(used) = used.filter(|v| v.is_finite()) else {
            continue;
        };
        let resource = stable_id(item.resource_type.as_deref())
            .or_else(|| stable_id(item.display_name.as_deref()))
            .unwrap_or_else(|| "usage".into());
        let occurrence = occurrences.entry(resource.clone()).or_default();
        *occurrence += 1;
        let id = if *occurrence == 1 {
            resource
        } else {
            format!("{resource}.{occurrence}")
        };
        let mut window = UsageWindow::new(
            &id,
            Kind::Monthly,
            item.display_name.or(item.resource_type),
            (used / limit).clamp(0.0, 1.0),
            30 * 86_400,
            reset,
        );
        // A date-only reset never proves a fixed 30-day duration.
        window.reports_length = false;
        window.is_exhausted = used >= limit;
        windows.push(window);
    }
    if windows.is_empty() {
        return unavailable(Unavailability::NoLimitsReported);
    }
    let mut usage = ProviderUsage::live_now(AccountKey::primary(Provider::Kiro), windows);
    usage.observed_at = Some(now_ms);
    usage.plan = payload.plan_name;
    usage.origin = Some("kiroACP".into());
    usage
}

fn stable_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_lowercase)
}

fn reset_date(text: &str) -> Option<i64> {
    // Accept the upstream yyyy-MM-dd UTC form, never a guessed timestamp unit.
    let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
    (date.format("%Y-%m-%d").to_string() == text)
        .then(|| {
            date.and_hms_opt(0, 0, 0)
                .map(|d| d.and_utc().timestamp_millis())
        })
        .flatten()
}

pub fn reason_for_message(message: Option<&str>) -> Unavailability {
    let text = message.unwrap_or_default().to_lowercase();
    if ["sign in", "not authenticated", "login"]
        .iter()
        .any(|v| text.contains(v))
    {
        Unavailability::KiroSignInRequired
    } else if ["method not found", "unsupported", "agent-engine"]
        .iter()
        .any(|v| text.contains(v))
    {
        Unavailability::KiroVersionUnsupported
    } else {
        Unavailability::UnreadableReply
    }
}

fn reason_for_failure(failure: &Failure) -> Unavailability {
    match failure {
        Failure::Missing => Unavailability::KiroNotInstalled,
        Failure::Server(message) => reason_for_message(Some(message)),
        Failure::Unsupported => Unavailability::KiroVersionUnsupported,
        Failure::Unreadable => Unavailability::UnreadableReply,
        Failure::Start | Failure::TimedOut | Failure::Closed => Unavailability::Unreachable,
    }
}
