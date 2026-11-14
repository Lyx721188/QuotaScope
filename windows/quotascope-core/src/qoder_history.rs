//! Whole-account credit records, not tokens or money. Pulse 338c796 protocol.
use crate::model::{AccountKey, Unavailability};
use chrono::{Days, Local, TimeZone};
use serde_json::Value;
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: chrono::NaiveDate,
    pub used: f64,
    pub charged: f64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Credits {
    pub days: Vec<Day>,
}
impl Credits {
    pub fn total(&self, days: usize) -> (f64, f64) {
        self.days
            .iter()
            .rev()
            .take(days)
            .fold((0.0, 0.0), |(u, c), d| (u + d.used, c + d.charged))
    }
}
pub fn number(value: f64) -> String {
    if value > 0.0 && value < 0.01 {
        "< 0.01".into()
    } else if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}
pub fn read(account: &AccountKey) -> crate::history::HistoryRead {
    let settings = crate::settings::with(Clone::clone);
    let slot = crate::provider_sites::slot_for(account, &settings);
    let Some(cookie) =
        crate::secrets::key_for(&slot).and_then(|c| crate::providers::qoder::normalize(&c))
    else {
        return crate::history::HistoryRead::NotConfigured;
    };
    let site = crate::provider_sites::site(account, &settings);
    let origin = if site == "china" {
        "https://qoder.com.cn"
    } else {
        "https://qoder.com"
    };
    let http = crate::http::HttpClient::new();
    let answer = read_with(crate::timeutil::now_ms(), |page, start, end| {
        if !crate::scan::checkpoint() {
            return Err(Unavailability::Unreachable);
        }
        let mut url = url::Url::parse(&format!(
            "{origin}/api/v1/me/usages/big_model_credits/histories"
        ))
        .map_err(|_| Unavailability::UnreadableReply)?;
        url.query_pairs_mut()
            .append_pair("page", &page.to_string())
            .append_pair("page_size", "500")
            .append_pair("start_time", &start.to_string())
            .append_pair("end_time", &end.to_string())
            .append_pair("order_by", "begin_at")
            .append_pair("order", "-1");
        let referer = format!("{origin}/account/usage");
        http.fetch_json(
            crate::http::Method::Get,
            url.as_str(),
            &[
                ("Cookie", &cookie),
                ("Accept", "application/json, text/plain, */*"),
                ("Origin", origin),
                ("Referer", &referer),
                ("X-Requested-With", "XMLHttpRequest"),
                ("Bx-V", "2.5.35"),
            ],
            None,
        )
        .map_err(|r| {
            if r == Unavailability::ApiKeyRefused {
                Unavailability::SessionExpired
            } else {
                r
            }
        })
    });
    match answer {
        Ok(credits) => crate::history::HistoryRead::Credits(credits),
        Err(reason) => crate::history::HistoryRead::Failed(reason),
    }
}
fn read_with(
    now: i64,
    mut get: impl FnMut(usize, i64, i64) -> Result<Value, Unavailability>,
) -> Result<Credits, Unavailability> {
    let invalid = Unavailability::UnreadableReply;
    let today = chrono::DateTime::from_timestamp_millis(now)
        .ok_or(invalid)?
        .with_timezone(&Local)
        .date_naive();
    let first = today.checked_sub_days(Days::new(30)).ok_or(invalid)?;
    let start = Local
        .from_local_datetime(&first.and_hms_opt(0, 0, 0).ok_or(invalid)?)
        .earliest()
        .ok_or(invalid)?
        .timestamp_millis();
    let mut days: Vec<Day> = (0..31)
        .map(|offset| Day {
            date: first + Days::new(offset),
            used: 0.0,
            charged: 0.0,
        })
        .collect();
    let mut page = 1;
    for _ in 0..6 {
        let root = get(page, start, now)?;
        let rows = root["data"].as_array().ok_or(invalid)?;
        if rows.len() > 500 {
            return Err(invalid);
        }
        let pagination = root
            .get("pageResult")
            .or_else(|| root.get("page_result"))
            .filter(|p| p.is_object())
            .ok_or(invalid)?;
        for row in rows {
            let at = row
                .get("beginAt")
                .or_else(|| row.get("begin_at"))
                .and_then(crate::providers::qoder::date)
                .ok_or(invalid)?;
            let charged = row["credits"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or(invalid)?;
            let used = row
                .get("originalCredits")
                .or_else(|| row.get("original_credits"))
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or(invalid)?;
            if at < start || at > now {
                continue;
            }
            let day = chrono::DateTime::from_timestamp_millis(at)
                .ok_or(invalid)?
                .with_timezone(&Local)
                .date_naive();
            if let Some(day) = days.iter_mut().find(|d| d.date == day) {
                day.used += used;
                day.charged += charged;
                if !day.used.is_finite() || !day.charged.is_finite() {
                    return Err(invalid);
                }
            }
        }
        let next = pagination
            .get("nextPage")
            .or_else(|| pagination.get("next_page"))
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or(invalid)
            })
            .transpose()?;
        let last = pagination
            .get("lastPage")
            .or_else(|| pagination.get("last_page"))
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or(invalid)
            })
            .transpose()?;
        if next.is_none_or(|next| next == 0) || last.is_some_and(|last| page >= last) {
            return Ok(Credits { days });
        }
        let next = next.unwrap();
        if next <= page || last.is_some_and(|last| next > last) {
            return Err(invalid);
        }
        page = next;
    }
    Err(invalid)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn records_keep_before_discount_and_deducted_credits_separate() {
        let now = 1794470400000;
        let credits=read_with(now,|_,_,_|Ok(json!({"data":[{"begin_at":now-1000,"original_credits":100.5,"credits":4.0},{"begin_at":now+1000,"original_credits":999.0,"credits":999.0}],"page_result":{}}))).unwrap();
        assert_eq!(credits.days.len(), 31);
        assert_eq!(credits.total(31), (100.5, 4.0));
    }
    #[test]
    fn failed_or_unclosed_paging_never_publishes_a_partial_month() {
        let now = 1794470400000;
        assert!(read_with(now, |page, _, _| Ok(
            json!({"data":[],"page_result":{"next_page":page+1}})
        ))
        .is_err());
        assert!(read_with(now, |_, _, _| Ok(
            json!({"data":[{"begin_at":now,"credits":1}],"page_result":{}})
        ))
        .is_err());
        assert!(read_with(now, |_, _, _| Ok(json!({"data":[]}))).is_err());
        assert!(read_with(now, |_, _, _| Ok(
            json!({"data":[],"page_result":{"next_page":1,"last_page":2}})
        ))
        .is_err());
    }
}
