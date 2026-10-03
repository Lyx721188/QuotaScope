//! DeepSeek console protocol, independent of browser, secrets and network.
//! Based on Pulse 3696a65's console shapes. This history is account-wide,
//! day-level billing, never input for local transcript API-value estimates.
use crate::history::HistoryRead;
use crate::ledger::{LedgerDay, TokenTally, UsageLedger};
use crate::model::Unavailability;
use chrono::{DateTime, Days, NaiveDate, Offset, TimeZone};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const ORIGIN: &str = "https://platform.deepseek.com";
pub const STORAGE_KEY: &str = "userToken";

pub fn storage_token(values: &BTreeMap<String, String>) -> Option<String> {
    let wrapper: Value = serde_json::from_str(values.get(STORAGE_KEY)?).ok()?;
    let token = wrapper.get("value")?.as_str()?.trim();
    (!token.is_empty() && token.len() <= 16_384 && token.bytes().all(|c| c.is_ascii_graphic()))
        .then(|| token.to_string())
}

fn code_problem(code: i64) -> Unavailability {
    if (40_000..40_100).contains(&code) {
        Unavailability::SessionExpired
    } else {
        Unavailability::ServerError
    }
}

/// Both envelope layers must affirm success. HTTP 200 alone is insufficient.
pub fn body(reply: &Value) -> Result<&Value, Unavailability> {
    let code = reply["code"]
        .as_i64()
        .ok_or(Unavailability::UnreadableReply)?;
    if code != 0 {
        return Err(code_problem(code));
    }
    let code = reply["data"]["biz_code"]
        .as_i64()
        .ok_or(Unavailability::UnreadableReply)?;
    if code != 0 {
        return Err(code_problem(code));
    }
    reply["data"]["biz_data"]
        .as_object()
        .map(|_| &reply["data"]["biz_data"])
        .ok_or(Unavailability::UnreadableReply)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Range {
    pub first: NaiveDate,
    pub today: NaiveDate,
    pub start_seconds: i64,
    pub end_seconds: i64,
    /// Whole-hour offset, expressed in seconds (e.g. +08 -> 28800).
    pub timezone_seconds: i32,
}

impl Range {
    pub fn new<T: TimeZone>(now: &DateTime<T>) -> Option<Self> {
        let today = now.date_naive();
        let first = today.checked_sub_days(Days::new(29))?;
        let tomorrow = today.checked_add_days(Days::new(1))?;
        let zone = now.timezone();
        let start = zone
            .from_local_datetime(&first.and_hms_opt(0, 0, 0)?)
            .earliest()?;
        let end = zone
            .from_local_datetime(&tomorrow.and_hms_opt(0, 0, 0)?)
            .earliest()?;
        let offset = now.offset().fix().local_minus_utc();
        let timezone_seconds = offset.div_euclid(3600) * 3600;
        let remainder = i64::from(offset - timezone_seconds);
        Some(Self {
            first,
            today,
            start_seconds: start.timestamp().checked_add(remainder)?,
            end_seconds: end.timestamp().checked_add(remainder)?,
            timezone_seconds,
        })
    }

    pub fn query(&self) -> [(String, String); 3] {
        [
            ("start".into(), self.start_seconds.to_string()),
            ("end".into(), self.end_seconds.to_string()),
            ("tz".into(), self.timezone_seconds.to_string()),
        ]
    }

    fn contains(&self, date: NaiveDate) -> bool {
        date >= self.first && date <= self.today
    }
}

fn figure(value: &Value) -> Option<f64> {
    crate::http::number(value).filter(|n| n.is_finite() && n.abs() < 1e15)
}

fn count(value: &Value) -> Option<i64> {
    figure(value)
        .filter(|n| *n >= 0.0 && n.fract() == 0.0)
        .map(|n| n as i64)
}

fn currency(value: &Value) -> Option<String> {
    let code = value.as_str()?.trim();
    (!code.is_empty() && code.len() <= 8 && code.bytes().all(|c| c.is_ascii_alphanumeric()))
        .then(|| code.to_ascii_uppercase())
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value], Unavailability> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or(Unavailability::UnreadableReply)
}

fn add_tally(a: TokenTally, b: TokenTally) -> Result<TokenTally, Unavailability> {
    let fail = Unavailability::UnreadableReply;
    Ok(TokenTally {
        input: a.input.checked_add(b.input).ok_or(fail)?,
        cache_read: a.cache_read.checked_add(b.cache_read).ok_or(fail)?,
        output: a.output.checked_add(b.output).ok_or(fail)?,
        cache_write: 0,
    })
}

/// Use the bucket's middle across a DST change; a shifted midnight can
/// otherwise place an entire day's work in the previous local date.
fn bucket_day<T: TimeZone>(value: &Value, half: f64, zone: &T) -> Option<NaiveDate> {
    let time = figure(value)? + half;
    let timestamp = DateTime::from_timestamp_millis((time * 1000.0) as i64)?;
    Some(timestamp.with_timezone(zone).date_naive())
}

struct Purse {
    currency: String,
    charges: Vec<(NaiveDate, f64)>,
    incomplete: bool,
}

pub fn parse_history<T: TimeZone>(
    amount_reply: &Value,
    cost_reply: &Value,
    preferred: Option<&str>,
    now: DateTime<T>,
) -> Result<HistoryRead, Unavailability> {
    let amount = body(amount_reply)?;
    let cost = body(cost_reply)?;
    let range = Range::new(&now).ok_or(Unavailability::UnreadableReply)?;
    let zone = now.timezone();
    let half = figure(&amount["bucket"])
        .filter(|n| *n > 0.0 && *n <= 172800.0)
        .unwrap_or(86400.0)
        / 2.0;
    let mut partial = false;
    let mut tallies = BTreeMap::<NaiveDate, BTreeMap<String, TokenTally>>::new();
    for series in array(amount, "series")? {
        if !crate::scan::checkpoint() {
            return Err(Unavailability::Loading);
        }
        let model = series["model"].as_str().unwrap_or("").to_string();
        for bucket in array(series, "buckets")? {
            if !crate::scan::checkpoint() {
                return Err(Unavailability::Loading);
            }
            let Some(date) = bucket_day(&bucket["time"], half, &zone) else {
                partial = true;
                continue;
            };
            if !range.contains(date) {
                continue;
            }
            let Some(usage) = bucket["usage"].as_object() else {
                partial = true;
                continue;
            };
            let mut read = |key: &str| {
                let value = usage.get(key).and_then(count);
                partial |= value.is_none();
                value.unwrap_or(0)
            };
            let tally = TokenTally {
                input: read("PROMPT_CACHE_MISS_TOKEN"),
                cache_read: read("PROMPT_CACHE_HIT_TOKEN"),
                output: read("RESPONSE_TOKEN"),
                cache_write: 0,
            };
            if tally.total() > 0 {
                let entry = tallies
                    .entry(date)
                    .or_default()
                    .entry(model.clone())
                    .or_default();
                *entry = add_tally(*entry, tally)?;
            }
        }
    }
    let mut purses = Vec::new();
    for purse in array(cost, "data")? {
        if !crate::scan::checkpoint() {
            return Err(Unavailability::Loading);
        }
        let Some(code) = currency(&purse["currency"]) else {
            partial = true;
            continue;
        };
        let mut charges = Vec::new();
        let mut incomplete = false;
        for series in array(purse, "series")? {
            for bucket in array(series, "buckets")? {
                if !crate::scan::checkpoint() {
                    return Err(Unavailability::Loading);
                }
                let Some(date) = bucket_day(&bucket["time"], half, &zone) else {
                    partial = true;
                    incomplete = true;
                    continue;
                };
                let Some(value) = figure(&bucket["cost"]).filter(|n| *n >= 0.0) else {
                    partial = true;
                    incomplete = true;
                    continue;
                };
                if range.contains(date) {
                    charges.push((date, value));
                }
            }
        }
        // A named wallet with no reported charge is still missing money;
        // at least one explicit zero is needed to assert a zero bill.
        if let Some(existing) = purses.iter_mut().find(|p: &&mut Purse| p.currency == code) {
            existing.charges.extend(charges);
            existing.incomplete |= incomplete;
        } else {
            purses.push(Purse {
                currency: code,
                charges,
                incomplete,
            });
        }
    }
    let has_money = |p: &&Purse| p.charges.iter().any(|(_, value)| *value > 0.0);
    let requested = purses
        .iter()
        .find(|p| !p.charges.is_empty() && preferred.is_some_and(|c| c == p.currency));
    let chosen = requested
        .filter(|p| has_money(p))
        .or_else(|| purses.iter().find(has_money))
        .or(requested)
        .or_else(|| purses.iter().find(|p| !p.charges.is_empty()))
        // Do not turn a missing or unreadable charged bucket into $0.00.
        .filter(|p| !p.incomplete);
    let mut costs = BTreeMap::<NaiveDate, f64>::new();
    if let Some(chosen) = chosen {
        for (day, charge) in &chosen.charges {
            let total = costs.entry(*day).or_default();
            *total += charge;
            if !total.is_finite() {
                return Err(Unavailability::UnreadableReply);
            }
        }
    }
    let mut ledger = UsageLedger::empty();
    ledger.has_partial_records = partial;
    for n in 0..30 {
        let date = range
            .first
            .checked_add_days(Days::new(n))
            .ok_or(Unavailability::UnreadableReply)?;
        let models = tallies.remove(&date).unwrap_or_default();
        let tally = models
            .values()
            .try_fold(TokenTally::default(), |a, b| add_tally(a, *b))?;
        let tokens = tally
            .input
            .checked_add(tally.cache_read)
            .and_then(|n| n.checked_add(tally.output))
            .ok_or(Unavailability::UnreadableReply)?;
        let cost = costs.remove(&date).unwrap_or(0.0);
        if ledger.earliest.is_none() && (tokens > 0 || cost > 0.0) {
            ledger.earliest = Some(date);
        }
        ledger.days.push(LedgerDay {
            date,
            tokens,
            cost,
            unpriced_tokens: 0,
            models: models
                .iter()
                .filter(|(id, _)| !id.is_empty())
                .map(|(id, tally)| (id.clone(), tally.total()))
                .collect(),
            tally,
            model_tallies: models
                .into_iter()
                .filter(|(id, _)| !id.is_empty())
                .collect(),
            model_costs: BTreeMap::new(),
        });
    }
    Ok(HistoryRead::Answered {
        ledger,
        account_wide: true,
        actual_costs: chosen.is_some(),
        currency: chosen.map(|p| p.currency.clone()),
    })
}

/// Convert readable wallets into the API-key balance shape, without an
/// availability verdict or the console's token-estimation guesses.
pub fn parse_summary(reply: &Value) -> Result<Value, Unavailability> {
    let summary = body(reply)?;
    let mut order = Vec::<String>::new();
    let mut wallets = BTreeMap::<String, [Option<f64>; 2]>::new();
    for (index, key) in ["normal_wallets", "bonus_wallets"].iter().enumerate() {
        for wallet in array(summary, key)? {
            if !crate::scan::checkpoint() {
                return Err(Unavailability::Loading);
            }
            let (Some(code), Some(balance)) =
                (currency(&wallet["currency"]), figure(&wallet["balance"]))
            else {
                continue;
            };
            if balance < 0.0 {
                continue;
            }
            if !wallets.contains_key(&code) {
                order.push(code.clone());
            }
            let entries = wallets.entry(code).or_default();
            let amount = entries[index].get_or_insert(0.0);
            *amount += balance;
            if !amount.is_finite() {
                return Err(Unavailability::UnreadableReply);
            }
        }
    }
    let infos: Vec<_> = order
        .into_iter()
        .map(|code| {
            let pair = wallets[&code];
            json!({
                "currency":code,
                "total_balance":format!("{:.2}",pair[0].unwrap_or(0.0)+pair[1].unwrap_or(0.0)),
                "topped_up_balance":pair[0].map(|n|format!("{n:.2}")),
                "granted_balance":pair[1].map(|n|format!("{n:.2}")),
            })
        })
        .collect();
    Ok(json!({"balance_infos":infos}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, LocalResult, NaiveDateTime, Timelike};

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(28800)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 3, 15, 0, 0)
            .unwrap()
    }
    fn reply(body: Value) -> Value {
        json!({"code":0,"data":{"biz_code":0,"biz_data":body}})
    }
    fn time(day: i64) -> i64 {
        let zone = now().timezone();
        zone.from_local_datetime(
            &(now().date_naive() + chrono::Duration::days(day))
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .unwrap()
        .timestamp()
    }
    fn amount() -> Value {
        reply(json!({"bucket":86400,"series":[
            {"model":"deepseek-chat","api_key":{"tracking_id":"synthetic-a"},"buckets":[{"time":time(0),"usage":{"PROMPT_CACHE_HIT_TOKEN":800,"PROMPT_CACHE_MISS_TOKEN":150,"RESPONSE_TOKEN":50,"REQUEST":3}}]},
            {"model":"deepseek-chat","api_key":{"tracking_id":"synthetic-b"},"buckets":[{"time":time(0),"usage":{"PROMPT_CACHE_HIT_TOKEN":"100","PROMPT_CACHE_MISS_TOKEN":"0","RESPONSE_TOKEN":"0"}}]},
            {"model":"deepseek-reasoner","buckets":[{"time":time(-1),"usage":{"PROMPT_CACHE_HIT_TOKEN":0,"PROMPT_CACHE_MISS_TOKEN":400,"RESPONSE_TOKEN":600}}]}
        ]}))
    }
    fn costs(usd: Value, cny: Value) -> Value {
        reply(json!({"data":[
            {"currency":"USD","series":[{"buckets":[{"time":time(0),"cost":usd}]}]},
            {"currency":"CNY","series":[{"buckets":[{"time":time(0),"cost":cny}]}]}
        ]}))
    }
    fn answered(value: HistoryRead) -> (UsageLedger, bool, Option<String>) {
        match value {
            HistoryRead::Answered {
                ledger,
                actual_costs,
                currency,
                account_wide,
            } => {
                assert!(account_wide);
                (ledger, actual_costs, currency)
            }
            _ => panic!("Expected parsed history"),
        }
    }

    #[test]
    fn storage_wrapper_is_separate_from_api_keys_and_signed_out_is_absent() {
        let token =
            |value: &str| storage_token(&BTreeMap::from([(STORAGE_KEY.into(), value.into())]));
        assert_eq!(
            token(r#"{"value":" abc123 ","__version":"0"}"#).as_deref(),
            Some("abc123")
        );
        for value in [
            r#"{"value":null}"#,
            r#"{"value":" "}"#,
            r#"{"value":"line\nbreak"}"#,
            "abc123",
            r#"{"accessToken":"other"}"#,
        ] {
            assert!(token(value).is_none());
        }
        assert!(storage_token(&BTreeMap::new()).is_none());
    }

    #[test]
    fn envelope_checks_both_codes_and_requires_explicit_success() {
        for code in [40000, 40002, 40003, 40099] {
            assert_eq!(
                body(&json!({"code":code})),
                Err(Unavailability::SessionExpired)
            );
            assert_eq!(
                body(&json!({"code":0,"data":{"biz_code":code}})),
                Err(Unavailability::SessionExpired)
            );
        }
        for code in [1, 39999, 40100, 50000] {
            assert_eq!(
                body(&json!({"code":code})),
                Err(Unavailability::ServerError)
            );
        }
        for value in [
            json!({}),
            json!({"code":0}),
            json!({"code":0,"data":{"biz_data":{}}}),
            json!({"code":"0"}),
            reply(Value::Null),
        ] {
            assert_eq!(body(&value), Err(Unavailability::UnreadableReply));
        }
        assert_eq!(
            body(&reply(json!({"series":[]}))).unwrap(),
            &json!({"series":[]})
        );
    }

    #[test]
    fn thirty_day_range_uses_seconds_and_floors_fractional_offsets() {
        let range = Range::new(&now()).unwrap();
        assert_eq!(range.start_seconds, time(-29));
        assert_eq!(range.end_seconds, time(1));
        assert_eq!(range.timezone_seconds, 28800);
        assert_eq!(range.end_seconds - range.start_seconds, 30 * 86400);
        for offset in [19800, -12600] {
            let n = now().with_timezone(&FixedOffset::east_opt(offset).unwrap());
            let r = Range::new(&n).unwrap();
            let remainder = i64::from(offset - offset.div_euclid(3600) * 3600);
            let local_start = n
                .timezone()
                .from_local_datetime(&r.first.and_hms_opt(0, 0, 0).unwrap())
                .unwrap()
                .timestamp();
            assert_eq!(r.start_seconds, local_start + remainder);
            assert_eq!(r.timezone_seconds, offset.div_euclid(3600) * 3600);
        }
    }

    #[test]
    fn account_keys_sum_tokens_in_calendar_days_without_hourly_invention() {
        let cost = reply(json!({"data":[{"currency":"CNY","series":[
            {"buckets":[{"time":time(0),"cost":"0.0125"}]},
            {"buckets":[{"time":time(-1),"cost":"1.5"}]}
        ]}]}));
        let (ledger, actual, currency) =
            answered(parse_history(&amount(), &cost, None, now()).unwrap());
        assert!(actual && !ledger.has_partial_records);
        assert_eq!(currency.as_deref(), Some("CNY"));
        assert_eq!(ledger.days.len(), 30);
        assert!(ledger.slots.is_empty() && ledger.unpriced_models.is_empty());
        assert_eq!(
            ledger.days[29].tally,
            TokenTally {
                input: 150,
                cache_read: 900,
                output: 50,
                cache_write: 0
            }
        );
        assert_eq!(ledger.days[29].models["deepseek-chat"], 1100);
        assert_eq!(ledger.days[28].tokens, 1000);
        assert!((ledger.days[29].cost - 0.0125).abs() < 1e-9);
        assert_eq!(ledger.days[28].cost, 1.5);
        assert_eq!(ledger.earliest, Some(now().date_naive() - Days::new(1)));
        assert!(
            (ledger
                .cache_hit_rate_calendar(30, now().date_naive())
                .unwrap()
                - 900.0 / 1450.0)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn currencies_never_mix_and_explicit_zero_is_distinct_from_missing() {
        for (usd, cny, preferred, wanted, total) in [
            (json!(0), json!(2), Some("USD"), "CNY", 2.0),
            (json!(1), json!(2), Some("USD"), "USD", 1.0),
            (json!(1), json!(2), Some("EUR"), "USD", 1.0),
            (json!(0), Value::Null, Some("USD"), "USD", 0.0),
        ] {
            let (ledger, actual, currency) =
                answered(parse_history(&amount(), &costs(usd, cny), preferred, now()).unwrap());
            assert!(actual);
            assert_eq!(currency.as_deref(), Some(wanted));
            assert_eq!(ledger.days[29].cost, total);
        }
        for cost in [
            reply(json!({"data":[]})),
            costs(Value::Null, Value::Null),
            reply(json!({"data":[{"currency":"USD","series":[]}]})),
        ] {
            let (ledger, actual, currency) =
                answered(parse_history(&amount(), &cost, None, now()).unwrap());
            assert!(!actual && currency.is_none());
            assert_eq!(ledger.days[29].tokens, 1100);
        }
    }

    #[test]
    fn malformed_numbers_are_partial_and_unidentified_tokens_stay_unassigned() {
        let bad = reply(json!({"bucket":86400,"series":[{"buckets":[
            {"time":time(0),"usage":{"PROMPT_CACHE_HIT_TOKEN":"nan","PROMPT_CACHE_MISS_TOKEN":"1e30","RESPONSE_TOKEN":"inf"}},
            {"time":time(-1),"usage":{"PROMPT_CACHE_HIT_TOKEN":-1,"PROMPT_CACHE_MISS_TOKEN":10,"RESPONSE_TOKEN":5}},
            {"time":time(-2),"usage":{"PROMPT_CACHE_HIT_TOKEN":0,"PROMPT_CACHE_MISS_TOKEN":0.5,"RESPONSE_TOKEN":0}},
            {"time":time(-30),"usage":{"PROMPT_CACHE_HIT_TOKEN":0,"PROMPT_CACHE_MISS_TOKEN":999,"RESPONSE_TOKEN":0}},
            {"time":"nan","usage":{}}
        ]}]}));
        let (ledger, actual, currency) =
            answered(parse_history(&bad, &reply(json!({"data":[]})), None, now()).unwrap());
        assert!(ledger.has_partial_records && !actual && currency.is_none());
        assert_eq!(ledger.days.iter().map(|d| d.tokens).sum::<i64>(), 15);
        assert!(ledger.days[28].models.is_empty());
        assert_eq!(ledger.days[28].tally.input, 10);
        assert!(parse_history(&reply(json!({})), &reply(json!({"data":[]})), None, now()).is_err());
        assert!(parse_history(&amount(), &reply(json!({})), None, now()).is_err());
    }

    #[test]
    fn normal_and_bonus_wallets_join_only_within_their_currency() {
        let summary = reply(json!({
            "normal_wallets":[{"balance":"8.15","currency":"CNY","token_estimation":"1"}],
            "bonus_wallets":[{"balance":"5.97","currency":"CNY"},{"balance":"1","currency":"USD"},{"balance":null,"currency":"EUR"},{"balance":-1,"currency":"JPY"}],
            "total_costs":[{"currency":"CNY","amount":"132.97"}]
        }));
        let result = parse_summary(&summary).unwrap();
        assert!(result.get("is_available").is_none());
        assert_eq!(result["balance_infos"].as_array().unwrap().len(), 2);
        assert_eq!(result["balance_infos"][0]["total_balance"], "14.12");
        assert_eq!(result["balance_infos"][1]["total_balance"], "1.00");
        assert!(result["balance_infos"][1]["topped_up_balance"].is_null());
        assert!(parse_summary(&reply(json!({}))).is_err());
        let empty = parse_summary(&reply(json!({"normal_wallets":[],"bonus_wallets":[]}))).unwrap();
        assert_eq!(empty["balance_infos"], json!([]));
    }

    #[test]
    fn duplicate_currency_rows_join_and_missing_buckets_never_become_zero_bills() {
        let first =
            json!({"currency":"CNY","series":[{"buckets":[{"time":time(0),"cost":"1.25"}]}]});
        let second =
            json!({"currency":"CNY","series":[{"buckets":[{"time":time(-1),"cost":"2.50"}]}]});
        let (ledger, actual, currency) = answered(
            parse_history(
                &amount(),
                &reply(json!({"data":[first,second]})),
                None,
                now(),
            )
            .unwrap(),
        );
        assert!(actual && currency.as_deref() == Some("CNY"));
        assert_eq!(ledger.days.iter().map(|d| d.cost).sum::<f64>(), 3.75);
        let incomplete = reply(json!({"data":[{"currency":"CNY","series":[{"buckets":[
            {"time":time(0),"cost":"1.25"},{"time":time(-1),"cost":null}
        ]}]}]}));
        let (ledger, actual, currency) =
            answered(parse_history(&amount(), &incomplete, None, now()).unwrap());
        assert!(ledger.has_partial_records && !actual && currency.is_none());
        assert_eq!(ledger.days[28].tokens, 1000);
    }

    // A small test zone with the European fall-back date. Production uses
    // chrono's actual OS zone; no substitute timezone is shipped.
    #[derive(Clone, Copy)]
    struct FallBack;
    impl TimeZone for FallBack {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Self
        }
        fn offset_from_local_date(&self, date: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, date: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let boundary = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
            let offset = if date.date() > boundary || (date.date() == boundary && date.hour() >= 3)
            {
                3600
            } else {
                7200
            };
            LocalResult::Single(FixedOffset::east_opt(offset).unwrap())
        }
        fn offset_from_utc_date(&self, date: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, date: &NaiveDateTime) -> FixedOffset {
            let boundary = NaiveDate::from_ymd_opt(2026, 10, 25)
                .unwrap()
                .and_hms_opt(1, 0, 0)
                .unwrap();
            FixedOffset::east_opt(if *date >= boundary { 3600 } else { 7200 }).unwrap()
        }
    }

    #[test]
    fn dst_range_retains_thirty_dates_and_shifted_midnight_uses_its_middle() {
        let n = FallBack.with_ymd_and_hms(2026, 10, 26, 15, 0, 0).unwrap();
        let range = Range::new(&n).unwrap();
        assert_eq!(range.end_seconds - range.start_seconds, 30 * 86400 + 3600);
        assert_eq!(range.timezone_seconds, 3600);
        let day = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let shifted = chrono::Utc
            .with_ymd_and_hms(2026, 10, 25, 22, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(bucket_day(&json!(shifted), 43200.0, &FallBack), Some(day));
        let (ledger, _, _) = answered(
            parse_history(
                &reply(json!({"bucket":86400,"series":[]})),
                &reply(json!({"data":[]})),
                None,
                n,
            )
            .unwrap(),
        );
        assert_eq!(ledger.days.len(), 30);
        assert_eq!(ledger.days[29].date, day);
    }
}
