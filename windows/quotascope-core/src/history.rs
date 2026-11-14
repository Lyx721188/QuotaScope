//! Account history. Provider statistics stay separate from local API-value
//! estimates. Actual provider charges carry their own currency explicitly.

use crate::ledger::{LedgerDay, TokenTally, UsageLedger};
use crate::model::{Provider, Unavailability};
use chrono::{DateTime, Days, Local, NaiveDate, TimeZone};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum HistoryRead {
    Credits(crate::qoder_history::Credits),
    NotConfigured,
    Failed(Unavailability),
    Answered {
        ledger: UsageLedger,
        account_wide: bool,
        actual_costs: bool,
        /// None for local USD API estimates or statistics without bills.
        /// Actual costs may be displayed only with an explicit currency.
        currency: Option<String>,
    },
}

pub fn read(provider: Provider) -> HistoryRead {
    match provider {
        Provider::ClaudeCode | Provider::Codex | Provider::Antigravity => HistoryRead::Answered {
            ledger: crate::ledger::ledger(provider),
            account_wide: false,
            actual_costs: false,
            currency: None,
        },
        Provider::OpenCodeGo => crate::opencode_console::history(),
        Provider::Qoder => crate::qoder_history::read(&crate::model::AccountKey::primary(provider)),
        Provider::DeepSeek => read_account(&crate::model::AccountKey::primary(provider)),
        Provider::Zai | Provider::GlmCoding => {
            let key = crate::secrets::key_for(provider.raw())
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    (provider == Provider::GlmCoding)
                        .then(crate::model::glm_stored_key)
                        .flatten()
                });
            let Some(key) = key else {
                return HistoryRead::NotConfigured;
            };
            let Some(url) = statistics_url(provider, Local::now()) else {
                return HistoryRead::Failed(Unavailability::ServerError);
            };
            let auth = format!("Bearer {}", key.trim());
            match crate::http::HttpClient::new().fetch_json(
                crate::http::Method::Get,
                url.as_str(),
                &[("Authorization", &auth), ("Accept", "application/json")],
                None,
            ) {
                Ok(reply) => parse_reply(&reply),
                Err(reason) => HistoryRead::Failed(reason),
            }
        }
        _ if local_source(provider).is_some() => {
            if !crate::settings::with(|s| s.reads_token_spend) {
                return HistoryRead::NotConfigured;
            }
            let id = local_source(provider).unwrap();
            let root = crate::additional_spend::CATALOG
                .iter()
                .find(|r| r.0 == id)
                .map(|r| r.2)
                .unwrap_or("");
            let mut ledger =
                crate::additional_spend::read(id, &crate::additional_spend::roots(id, root));
            if provider == Provider::Devin {
                ledger = merge_local(vec![
                    ledger,
                    crate::ledger::pi_ledger_for_provider(crate::ledger::PiClient::Omp, "devin"),
                ]);
            }
            HistoryRead::Answered {
                ledger,
                account_wide: false,
                actual_costs: false,
                currency: None,
            }
        }
        _ => HistoryRead::NotConfigured,
    }
}

pub fn local_source(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Minimax | Provider::MinimaxCn => Some("mcode"),
        Provider::KimiCode => Some("kimi"),
        Provider::CommandCode => Some("commandcode"),
        Provider::Grok => Some("grok"),
        Provider::Devin => Some("devin-cli"),
        Provider::Cursor => Some("cursor"),
        Provider::Copilot => Some("copilot"),
        _ => None,
    }
}
/// Combine disjoint clients without repricing the records or changing currencies.
fn merge_local(ledgers: Vec<UsageLedger>) -> UsageLedger {
    let mut out = UsageLedger::empty();
    let mut days = BTreeMap::new();
    let mut slots = BTreeMap::new();
    for ledger in ledgers {
        out.has_partial_records |= ledger.has_partial_records;
        out.unpriced_models.extend(ledger.unpriced_models);
        out.model_names.extend(ledger.model_names);
        for day in ledger.days {
            let old = days
                .entry(day.date)
                .or_insert_with(|| crate::ledger::LedgerDay {
                    date: day.date,
                    tokens: 0,
                    cost: 0.0,
                    unpriced_tokens: 0,
                    models: BTreeMap::new(),
                    tally: TokenTally::default(),
                    model_tallies: BTreeMap::new(),
                    model_costs: BTreeMap::new(),
                });
            old.tokens += day.tokens;
            old.cost += day.cost;
            old.unpriced_tokens += day.unpriced_tokens;
            old.tally += day.tally;
            for (k, v) in day.models {
                *old.models.entry(k).or_default() += v;
            }
            for (k, v) in day.model_tallies {
                *old.model_tallies.entry(k).or_default() += v;
            }
            for (k, v) in day.model_costs {
                *old.model_costs.entry(k).or_default() += v;
            }
        }
        for slot in ledger.slots {
            let old = slots
                .entry(slot.start_ms)
                .or_insert_with(|| crate::ledger::Slot {
                    start_ms: slot.start_ms,
                    tokens: 0,
                    cost: 0.0,
                    unpriced_tokens: 0,
                    models: BTreeMap::new(),
                    unclassified_models: BTreeMap::new(),
                    costs: BTreeMap::new(),
                });
            old.tokens += slot.tokens;
            old.cost += slot.cost;
            old.unpriced_tokens += slot.unpriced_tokens;
            for (k, v) in slot.models {
                *old.models.entry(k).or_default() += v;
            }
            for (k, v) in slot.unclassified_models {
                *old.unclassified_models.entry(k).or_default() += v;
            }
            for (k, v) in slot.costs {
                *old.costs.entry(k).or_default() += v;
            }
        }
    }
    out.days = days.into_values().collect();
    out.slots = slots.into_values().collect();
    out.earliest = out.days.first().map(|d| d.date);
    out.unpriced_models.sort();
    out.unpriced_models.dedup();
    out
}

pub fn read_account(account: &crate::model::AccountKey) -> HistoryRead {
    if account.provider == Provider::Qoder {
        return crate::qoder_history::read(account);
    }
    if account.provider == Provider::DeepSeek {
        let currency = crate::settings::with(|s| s.deepseek_currency.clone());
        return crate::deepseek_history::read(account, currency.as_deref());
    }
    if account.is_primary() {
        read(account.provider)
    } else {
        HistoryRead::NotConfigured
    }
}

/// Local wall-clock dates, percent encoded, with the storefront chosen
/// explicitly so a mainland credential never reaches the international host.
pub fn statistics_url(provider: Provider, now: DateTime<Local>) -> Option<url::Url> {
    if !matches!(provider, Provider::Zai | Provider::GlmCoding) {
        return None;
    }
    let date = now.date_naive().checked_sub_days(Days::new(30))?;
    let start = Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()?;
    let mut url = url::Url::parse(&format!(
        "{}/api/monitor/usage/model-usage",
        crate::providers::zai::host_for(provider)
    ))
    .ok()?;
    url.query_pairs_mut()
        .append_pair("startTime", &start.format("%Y-%m-%d %H:%M:%S").to_string())
        .append_pair("endTime", &now.format("%Y-%m-%d %H:%M:%S").to_string());
    Some(url)
}

pub fn parse_reply(reply: &serde_json::Value) -> HistoryRead {
    if reply.get("success").and_then(|v| v.as_bool()) != Some(true)
        || reply.get("code").and_then(|v| v.as_i64()) != Some(200)
    {
        return HistoryRead::Failed(crate::providers::zai::envelope_problem(reply));
    }
    let Some(payload) = reply.get("data").filter(|v| v.is_object()) else {
        return HistoryRead::Failed(Unavailability::ServerError);
    };
    match parse_statistics(payload) {
        Some(ledger) => HistoryRead::Answered {
            ledger,
            account_wide: true,
            actual_costs: false,
            currency: None,
        },
        None => HistoryRead::Failed(Unavailability::ServerError),
    }
}

fn count(value: &serde_json::Value) -> Option<i64> {
    crate::http::number(value)
        .filter(|n| n.is_finite() && *n >= 0.0 && *n < i64::MAX as f64)
        .map(|n| n.round() as i64)
}

/// Hourly buckets aggregate to days. The total wins when present, otherwise
/// model counts supply it; no input/output/cache split or money is invented.
pub fn parse_statistics(payload: &serde_json::Value) -> Option<UsageLedger> {
    let labels = match payload.get("x_time") {
        None | Some(serde_json::Value::Null) => return Some(UsageLedger::empty()),
        Some(value) => value.as_array()?,
    };
    let totals = payload.get("tokensUsage").and_then(|v| v.as_array());
    let series = crate::http::array_field(payload, "modelDataList");
    let mut tokens: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    let mut models: BTreeMap<NaiveDate, BTreeMap<String, i64>> = BTreeMap::new();
    for (index, label) in labels.iter().enumerate() {
        let Some(day) = label
            .as_str()
            .and_then(|s| s.get(..10))
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        else {
            continue;
        };
        let mut model_total: i64 = 0;
        for item in series {
            let Some(name) = crate::http::string_field(item, "modelName").filter(|s| !s.is_empty())
            else {
                continue;
            };
            let Some(n) = item
                .get("tokensUsage")
                .and_then(|v| v.as_array())
                .and_then(|v| v.get(index))
                .and_then(count)
                .filter(|n| *n > 0)
            else {
                continue;
            };
            let slot = models
                .entry(day)
                .or_default()
                .entry(name.to_string())
                .or_default();
            *slot = slot.saturating_add(n);
            model_total = model_total.saturating_add(n);
        }
        let total = totals
            .and_then(|v| v.get(index))
            .and_then(count)
            .unwrap_or(model_total);
        if total > 0 {
            let slot = tokens.entry(day).or_default();
            *slot = slot.saturating_add(total);
        }
    }
    let (Some(first), Some(last)) = (
        tokens.keys().next().copied(),
        tokens.keys().next_back().copied(),
    ) else {
        return Some(UsageLedger::empty());
    };
    // The request is thirty days; malformed centuries of buckets cannot
    // turn a detail card into an unbounded allocation.
    if last.signed_duration_since(first).num_days() > 366 {
        return None;
    }
    let mut days = Vec::new();
    let mut day = first;
    loop {
        let n = tokens.get(&day).copied().unwrap_or(0);
        days.push(LedgerDay {
            date: day,
            tokens: n,
            cost: 0.0,
            unpriced_tokens: n,
            models: models.remove(&day).unwrap_or_default(),
            tally: TokenTally::default(),
            model_tallies: BTreeMap::new(),
            model_costs: BTreeMap::new(),
        });
        if day == last {
            break;
        }
        day = day.succ_opt()?;
    }
    Some(UsageLedger {
        days,
        earliest: Some(first),
        ..UsageLedger::empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hourly_counts_merge_and_gaps_are_calendar_days() {
        let ledger = parse_statistics(&json!({"x_time":["2026-09-01 01:00","2026-09-01 02:00","2026-09-03"],"tokensUsage":[2.5,4,10],"modelDataList":[{"modelName":"glm","tokensUsage":[1,3,9]}]})).unwrap();
        assert_eq!(
            ledger.days.iter().map(|d| d.tokens).collect::<Vec<_>>(),
            vec![7, 0, 10]
        );
        assert_eq!(ledger.days[0].models["glm"], 4);
        assert!(ledger
            .days
            .iter()
            .all(|d| d.cost == 0.0 && d.unpriced_tokens == d.tokens));
        assert!(ledger.slots.is_empty());
    }

    #[test]
    fn short_total_series_falls_back_to_models_but_zero_is_a_real_total() {
        let ledger = parse_statistics(&json!({"x_time":["2026-09-01","2026-09-02","2026-09-03"],"tokensUsage":[0,4],"modelDataList":[{"modelName":"glm","tokensUsage":[5,6,7]}]})).unwrap();
        assert_eq!(
            ledger.days.iter().map(|d| d.tokens).collect::<Vec<_>>(),
            vec![4, 7]
        );
    }

    #[test]
    fn empty_success_is_distinct_from_missing_key_or_failed_reply() {
        assert!(
            matches!(parse_reply(&json!({"success":true,"code":200,"data":{"x_time":[]}})), HistoryRead::Answered { ledger, .. } if ledger.days.is_empty())
        );
        assert!(matches!(
            parse_reply(&json!({"success":false,"code":401})),
            HistoryRead::Failed(Unavailability::ApiKeyRefused)
        ));
        assert!(matches!(
            parse_reply(&json!({"success":true,"code":200,"data":"broken"})),
            HistoryRead::Failed(_)
        ));
    }

    #[test]
    fn invalid_dates_counts_and_unbounded_spans_are_rejected() {
        assert!(
            parse_statistics(&json!({"x_time":["乱码测试","bad"],"tokensUsage":[1,-3]}))
                .unwrap()
                .days
                .is_empty()
        );
        assert!(parse_statistics(
            &json!({"x_time":["1900-01-01","2026-01-01"],"tokensUsage":[1,1]})
        )
        .is_none());
    }

    #[test]
    fn history_requests_keep_the_two_storefronts_separate() {
        let now = Local.with_ymd_and_hms(2026, 10, 1, 14, 30, 0).unwrap();
        for (provider, host) in [
            (Provider::Zai, "api.z.ai"),
            (Provider::GlmCoding, "open.bigmodel.cn"),
        ] {
            let url = statistics_url(provider, now).unwrap();
            assert_eq!(url.host_str(), Some(host));
            let pairs: BTreeMap<_, _> = url.query_pairs().collect();
            assert_eq!(pairs["startTime"], "2026-09-01 00:00:00");
            assert_eq!(pairs["endTime"], "2026-10-01 14:30:00");
        }
    }
}
