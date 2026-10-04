//! The official pages' daily bars and published uptime, adapted from Pulse
//! 1.7.2 ServiceStatus.swift, Copyright (c) 2026 qunqin24, Apache-2.0.
use crate::service_status::{Component, Day, Page, ReadError, Reading, State};
use chrono::{Days, Local, NaiveDate, TimeZone};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug)]
struct Interval {
    date: NaiveDate,
    start: i64,
    end: i64,
}

fn intervals<T: TimeZone>(now: i64, count: u64, zone: &T) -> Result<Vec<Interval>, ReadError> {
    let today = zone
        .timestamp_millis_opt(now)
        .single()
        .ok_or(ReadError::Unreadable)?
        .date_naive();
    (0..count)
        .rev()
        .map(|back| {
            let date = today
                .checked_sub_days(Days::new(back))
                .ok_or(ReadError::Unreadable)?;
            let next = date.succ_opt().ok_or(ReadError::Unreadable)?;
            let start = zone
                .from_local_datetime(&date.and_hms_opt(0, 0, 0).ok_or(ReadError::Unreadable)?)
                .earliest()
                .ok_or(ReadError::Unreadable)?
                .timestamp_millis();
            let end = zone
                .from_local_datetime(&next.and_hms_opt(0, 0, 0).ok_or(ReadError::Unreadable)?)
                .earliest()
                .ok_or(ReadError::Unreadable)?
                .timestamp_millis();
            Ok(Interval { date, start, end })
        })
        .collect()
}

pub(crate) fn endpoint(reading: &Reading) -> Result<String, ReadError> {
    match reading.page {
        Page::Codex => {
            let days = intervals(reading.checked_at, 91, &Local)?;
            let mut url = url::Url::parse(
                "https://status.openai.com/proxy/status.openai.com/component_impacts",
            )
            .map_err(|_| ReadError::Unreadable)?;
            url.query_pairs_mut()
                .append_pair("start_at", &crate::timeutil::iso8601_utc(days[0].start))
                .append_pair("end_at", &crate::timeutil::iso8601_utc(days[90].end));
            Ok(url.into())
        }
        Page::Claude => {
            let mut url = url::Url::parse("https://status.claude.com/uptime_showcase")
                .map_err(|_| ReadError::Unreadable)?;
            url.query_pairs_mut().append_pair(
                "components",
                &reading
                    .components
                    .iter()
                    .map(|c| c.id.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            Ok(url.into())
        }
        Page::DeepSeek => Ok(reading.page.address().into()),
    }
}

fn feed(bytes: &[u8]) -> Result<Value, ReadError> {
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(ReadError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ReadError::Unreadable)
}
fn array(value: &Value, cap: usize) -> Result<&[Value], ReadError> {
    let values = value.as_array().ok_or(ReadError::Unreadable)?;
    if values.len() > cap {
        return Err(ReadError::TooLarge);
    }
    Ok(values)
}
fn uptime(value: &Value) -> Option<f64> {
    let number = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())?;
    (number.is_finite() && (0.0..=100.0).contains(&number)).then_some(number)
}
fn stamp(value: &Value, seconds: bool) -> Option<i64> {
    if seconds {
        let n = value.as_f64()?;
        // Explicit range avoids saturating casts masquerading as a date.
        (n.is_finite() && (-62_135_596_800.0..=253_402_300_799.0).contains(&n))
            .then_some((n * 1000.0) as i64)
    } else {
        crate::timeutil::parse_iso8601_ms(value.as_str()?)
    }
}

/// Validate relevant impacts before drawing any healthy day. Group-only
/// uptime entries and malformed unrelated rows cannot discard a good history.
fn impact_history(
    value: &Value,
    components: &mut [Component],
    now: i64,
    days: &[Interval],
    seconds: bool,
) -> Result<(), ReadError> {
    let mut by_id: HashMap<&str, Vec<(i64, i64, State)>> = HashMap::new();
    for row in array(&value["component_impacts"], 20_000)? {
        let Some(id) = row["component_id"].as_str() else {
            return Err(ReadError::Unreadable);
        };
        if !components.iter().any(|c| c.id == id) {
            continue;
        }
        let start = stamp(
            &row[if seconds {
                "start_at_seconds"
            } else {
                "start_at"
            }],
            seconds,
        )
        .ok_or(ReadError::Unreadable)?;
        let end_value = &row[if seconds { "end_at_seconds" } else { "end_at" }];
        let end = if end_value.is_null() {
            now
        } else {
            stamp(end_value, seconds)
                .ok_or(ReadError::Unreadable)?
                .min(now)
        };
        if end < start && start <= now {
            return Err(ReadError::Unreadable);
        }
        if end <= start {
            continue;
        }
        let state = row["status"]
            .as_str()
            .map(State::from_feed)
            .unwrap_or(State::Unrecognised);
        by_id.entry(id).or_default().push((start, end, state));
    }
    let uptime_rows = array(&value["component_uptimes"], 2048)?;
    for component in components {
        let row = uptime_rows
            .iter()
            .find(|r| r["component_id"] == component.id);
        let since = row.and_then(|r| {
            stamp(
                &r[if seconds {
                    "available_since_seconds"
                } else {
                    "data_available_since"
                }],
                seconds,
            )
        });
        if row.is_some_and(|r| {
            !r[if seconds {
                "available_since_seconds"
            } else {
                "data_available_since"
            }]
            .is_null()
        }) && since.is_none()
        {
            return Err(ReadError::Unreadable);
        }
        component.uptime = row.and_then(|r| uptime(&r["uptime"]));
        component.days = days
            .iter()
            .map(|day| {
                let state = if since.is_some_and(|start| day.end <= start) {
                    None
                } else {
                    Some(
                        by_id
                            .get(component.id.as_str())
                            .into_iter()
                            .flatten()
                            .filter(|(start, end, _)| *start < day.end && *end > day.start)
                            .map(|(_, _, state)| *state)
                            .max_by_key(|s| s.severity())
                            .unwrap_or(State::Operational),
                    )
                };
                Day {
                    date: day.date,
                    state,
                    rgb: None,
                }
            })
            .collect();
    }
    Ok(())
}

fn bar_colours(html: &str) -> Vec<u32> {
    html.split("<rect")
        .skip(1)
        .filter_map(|part| {
            let tag = part.split_once('>')?.0;
            if !tag.contains("uptime-day") {
                return None;
            }
            let fill = tag.split_once("fill=\"#")?.1;
            if fill.len() < 7
                || fill.as_bytes()[6] != b'"'
                || !fill.as_bytes()[..6].iter().all(u8::is_ascii_hexdigit)
            {
                return None;
            }
            u32::from_str_radix(&fill[..6], 16).ok()
        })
        .collect()
}

fn statuspage_history(
    value: &Value,
    components: &mut [Component],
    today: NaiveDate,
) -> Result<(), ReadError> {
    let timelines = value["timelines"]
        .as_object()
        .ok_or(ReadError::Unreadable)?;
    let oldest = today
        .checked_sub_days(Days::new(89))
        .ok_or(ReadError::Unreadable)?;
    let values = array(&value["values"], 2048)?;
    let mut partial = false;
    for component in components {
        let Some(timeline) = timelines.get(&component.id) else {
            partial = true;
            continue;
        };
        let raw = array(&timeline["days"], 366)?;
        let since = timeline["component"]["startDate"]
            .as_str()
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        let mut parsed = Vec::new();
        let mut aligned = true;
        for day in raw {
            let Some(date) = day["date"]
                .as_str()
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            else {
                aligned = false;
                partial = true;
                continue;
            };
            if date < oldest || date > today {
                aligned = false;
                continue;
            }
            let outages = &day["outages"];
            let seconds = |key: &str| -> Option<i64> {
                if !outages.is_null() && !outages.is_object() {
                    return None;
                }
                if outages[key].is_null() {
                    Some(0)
                } else {
                    outages[key].as_i64().filter(|n| *n >= 0)
                }
            };
            let state = if since.is_some_and(|start| date < start) {
                None
            } else {
                Some(match (seconds("m"), seconds("p")) {
                    (Some(m), Some(p)) => {
                        if m > 0 {
                            State::FullOutage
                        } else if p > 0 {
                            State::PartialOutage
                        } else {
                            State::Operational
                        }
                    }
                    _ => State::Unrecognised,
                })
            };
            parsed.push(Day {
                date,
                state,
                rgb: None,
            });
        }
        if parsed.windows(2).any(|rows| rows[0].date >= rows[1].date) {
            partial = true;
            continue; // Conflicting dates cannot be positioned or coloured.
        }
        if aligned && parsed.len() == raw.len() {
            let colours = value["components"][&component.id]
                .as_str()
                .map(bar_colours)
                .unwrap_or_default();
            if colours.len() == parsed.len() {
                for (day, colour) in parsed.iter_mut().zip(colours) {
                    day.rgb = Some(colour);
                }
            }
        }
        component.days = parsed;
        component.uptime = values
            .iter()
            .find(|v| v["component"] == component.id)
            .and_then(|v| uptime(&v["ninety"]));
        if component.days.is_empty() {
            partial = true;
        }
    }
    if partial {
        Err(ReadError::Unreadable)
    } else {
        Ok(())
    }
}

pub(crate) fn apply(reading: &mut Reading, bytes: &[u8]) -> Result<(), ReadError> {
    apply_in_zone(reading, bytes, &Local)
}

fn apply_in_zone<T: TimeZone>(
    reading: &mut Reading,
    bytes: &[u8],
    zone: &T,
) -> Result<(), ReadError> {
    match reading.page {
        Page::Codex => impact_history(
            &feed(bytes)?,
            &mut reading.components,
            reading.checked_at,
            &intervals(reading.checked_at, 91, zone)?,
            false,
        ),
        Page::DeepSeek => impact_history(
            &crate::service_status::flashcat_data(bytes)?,
            &mut reading.components,
            reading.checked_at,
            &intervals(reading.checked_at, 90, zone)?,
            true,
        ),
        Page::Claude => {
            let today = zone
                .timestamp_millis_opt(reading.checked_at)
                .single()
                .ok_or(ReadError::Unreadable)?
                .date_naive();
            statuspage_history(&feed(bytes)?, &mut reading.components, today)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use serde_json::json;

    fn reading(page: Page) -> Reading {
        Reading {
            page,
            checked_at: crate::timeutil::parse_iso8601_ms("2026-10-04T16:00:00+08:00").unwrap(),
            components: vec![Component {
                id: "a".into(),
                name: "A".into(),
                state: State::Degraded,
                days: vec![],
                uptime: None,
            }],
            history_failed: false,
        }
    }
    fn zone() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }
    fn bars(c: &Component) -> String {
        c.days
            .iter()
            .map(|d| match d.state {
                None => '-',
                Some(State::Operational) => '0',
                Some(State::Degraded) => '1',
                Some(State::PartialOutage) => '2',
                Some(State::FullOutage) => '3',
                Some(State::Maintenance) => 'm',
                Some(State::Unrecognised) => '?',
            })
            .collect()
    }
    #[test]
    fn history_uses_local_calendar_overlap_open_impacts_and_published_uptime() {
        let mut r = reading(Page::Codex);
        let data = json!({"component_impacts":[
            {"component_id":"a","start_at":"2026-10-02T23:59:00+08:00","end_at":"2026-10-03T00:01:00+08:00","status":"degraded"},
            {"component_id":"a","start_at":"2026-10-03T23:59:00+08:00","status":"partial_outage"},
            {"component_id":"a","start_at":"2026-10-04T18:00:00+08:00","status":"full_outage"}],
            "component_uptimes":[{"status_page_component_group_id":"group","uptime":"99.99"},
                {"component_id":"a","uptime":"98.76","data_available_since":"2026-10-02T10:00:00+08:00"}]});
        apply_in_zone(&mut r, &serde_json::to_vec(&data).unwrap(), &zone()).unwrap();
        assert_eq!(r.components[0].days.len(), 91);
        assert!(bars(&r.components[0]).ends_with("-122"));
        assert_eq!(r.components[0].uptime, Some(98.76));
        assert_eq!(r.components[0].state, State::Degraded);
    }
    #[test]
    fn unreadable_history_never_overwrites_current_state_or_draws_healthy_days() {
        let mut r = reading(Page::Codex);
        assert!(apply_in_zone(&mut r, b"{}", &zone()).is_err());
        assert_eq!(r.components[0].state, State::Degraded);
        assert!(r.components[0].days.is_empty() && r.components[0].uptime.is_none());
        let invalid = json!({"component_impacts":[{"component_id":"a","start_at":"bad","status":"full_outage"}],"component_uptimes":[]});
        assert!(apply_in_zone(&mut r, &serde_json::to_vec(&invalid).unwrap(), &zone()).is_err());
        assert!(r.components[0].days.is_empty());
    }
    #[test]
    fn invalid_uptime_stays_missing_and_unknown_impact_is_not_healthy() {
        let mut r = reading(Page::Codex);
        let data = json!({"component_impacts":[{"component_id":"a","start_at":"2026-10-04T12:00:00+08:00","status":"new-state"},
            {"component_id":"unrelated","start_at":"bad"}],"component_uptimes":[{"component_id":"a","uptime":"NaN"}]});
        apply_in_zone(&mut r, &serde_json::to_vec(&data).unwrap(), &zone()).unwrap();
        assert_eq!(
            r.components[0].days.last().unwrap().state,
            Some(State::Unrecognised)
        );
        assert_eq!(r.components[0].uptime, None);
        for v in [json!(-1), json!(101), json!("Infinity"), Value::Null] {
            assert_eq!(uptime(&v), None);
        }
    }
    #[test]
    fn claude_preserves_page_dates_and_svg_colours_without_computing_uptime() {
        let mut r = reading(Page::Claude);
        let data = json!({"timelines":{"a":{"component":{"startDate":"2026-10-03"},"days":[
            {"date":"2026-10-02","outages":{}},{"date":"2026-10-03","outages":{"p":20}},
            {"date":"2026-10-04","outages":{"m":30}}]}},
            "components":{"a":"<rect class=\"uptime-day\" fill=\"#AAAAAA\"/><rect class=\"uptime-day\" fill=\"#123456\"/><rect class=\"uptime-day\" fill=\"#E04343\"/>"},
            "values":[{"component":"a","ninety":99.42}]});
        apply_in_zone(&mut r, &serde_json::to_vec(&data).unwrap(), &zone()).unwrap();
        assert_eq!(bars(&r.components[0]), "-23");
        assert_eq!(
            r.components[0]
                .days
                .iter()
                .map(|d| d.rgb)
                .collect::<Vec<_>>(),
            [Some(0xAAAAAA), Some(0x123456), Some(0xE04343)]
        );
        assert_eq!(r.components[0].uptime, Some(99.42));
    }
    #[test]
    fn missing_and_duplicate_dates_do_not_receive_misaligned_colours() {
        let mut r = reading(Page::Claude);
        let data = json!({"timelines":{"a":{"component":{},"days":[{"date":"bad"},{"date":"2026-10-04","outages":{}}]}},
            "components":{"a":"<rect class=\"uptime-day\" fill=\"#123456\"/><rect class=\"uptime-day\" fill=\"#E04343\"/>"},"values":[]});
        assert!(apply_in_zone(&mut r, &serde_json::to_vec(&data).unwrap(), &zone()).is_err());
        assert_eq!(r.components[0].days.len(), 1);
        assert_eq!(r.components[0].days[0].rgb, None);
        let duplicate = json!({"timelines":{"a":{"component":{},"days":[{"date":"2026-10-04"},{"date":"2026-10-04"}]}},"values":[]});
        let mut r = reading(Page::Claude);
        assert!(apply_in_zone(&mut r, &serde_json::to_vec(&duplicate).unwrap(), &zone()).is_err());
        assert!(r.components[0].days.is_empty());
    }
    #[test]
    fn openai_frozen_history_matches_all_four_independently_checked_rows() {
        let mut r = crate::service_status::parse(
            Page::Codex,
            include_bytes!("../tests/fixtures/windows-openai-status-operational.json"),
            reading(Page::Codex).checked_at,
        )
        .unwrap();
        apply_in_zone(
            &mut r,
            include_bytes!("../tests/fixtures/windows-openai-status-impacts.json"),
            &zone(),
        )
        .unwrap();
        let expected = [
            "0000000000000010011100000000000000000000000000000000000000011000000000000000000000300010000",
            "0000000000000000011100000000000000000000000000100000000000011000000000000000000000300010000",
            "0000000000001010011100000000000000000000000000000000000000011000000000000000000000300010000",
            "0000000000000010011100000000000000000000000000000000000000011000000000000000000000300010000"];
        assert_eq!(r.components.iter().map(bars).collect::<Vec<_>>(), expected);
        assert!(r.components.iter().all(|c| c.uptime == Some(99.95)));
    }
    #[test]
    fn claude_frozen_history_retains_all_official_colours_dates_and_uptimes() {
        let mut r = crate::service_status::parse(
            Page::Claude,
            include_bytes!("../tests/fixtures/windows-claude-status-summary.json"),
            reading(Page::Claude).checked_at,
        )
        .unwrap();
        apply_in_zone(
            &mut r,
            include_bytes!("../tests/fixtures/windows-claude-status-uptime.json"),
            &zone(),
        )
        .unwrap();
        assert_eq!(
            r.components.iter().map(|c| c.uptime).collect::<Vec<_>>(),
            [
                Some(99.44),
                Some(99.95),
                Some(99.52),
                Some(99.44),
                Some(99.44),
                Some(100.0)
            ]
        );
        assert!(r.components.iter().all(|c| c.days.len() == 90));
        let code = r
            .components
            .iter()
            .find(|c| c.id == "yyzkbfz2thpt")
            .unwrap();
        assert_eq!(code.days[0].date.to_string(), "2026-07-07");
        assert_eq!(code.days[89].date.to_string(), "2026-10-04");
        assert_eq!(code.days[0].state, Some(State::PartialOutage));
        assert_eq!(code.days[0].rgb, Some(0xE75F36));
        for c in &r.components {
            for day in &c.days {
                assert_eq!(
                    day.rgb == Some(0x76AD2A),
                    day.state == Some(State::Operational)
                );
            }
        }
    }
    #[test]
    fn bad_availability_and_malformed_outage_seconds_never_become_healthy_bars() {
        let mut r = reading(Page::Codex);
        let bad_since = json!({"component_impacts":[],"component_uptimes":[{"component_id":"a","data_available_since":"bad","uptime":99.9}]});
        assert!(apply_in_zone(&mut r, &serde_json::to_vec(&bad_since).unwrap(), &zone()).is_err());
        assert!(r.components[0].days.is_empty());
        let bad_seconds = json!({"timelines":{"a":{"component":{},"days":[{"date":"2026-10-04","outages":"bad"}]}},"values":[]});
        let mut r = reading(Page::Claude);
        apply_in_zone(&mut r, &serde_json::to_vec(&bad_seconds).unwrap(), &zone()).unwrap();
        assert_eq!(r.components[0].days[0].state, Some(State::Unrecognised));
    }
    #[test]
    fn invalid_unicode_svg_fill_and_record_overflow_are_bounded() {
        assert!(bar_colours("<rect class=\"uptime-day\" fill=\"#abc😀\"/>").is_empty());
        assert_eq!(array(&json!([null]), 0), Err(ReadError::TooLarge));
        assert_eq!(
            feed(&vec![b' '; 4 * 1024 * 1024 + 1]),
            Err(ReadError::TooLarge)
        );
    }
    #[test]
    fn deepseek_frozen_public_history_matches_the_upstream_checked_bars() {
        let bytes = include_bytes!("../tests/fixtures/windows-deepseek-status-page.html");
        let mut r =
            crate::service_status::parse(Page::DeepSeek, bytes, reading(Page::DeepSeek).checked_at)
                .unwrap();
        apply_in_zone(&mut r, bytes, &zone()).unwrap();
        let expected = [
            "000000000000000001000101100120020000001000010000000000000300000000000010100000220000000100",
            "000000000000000001000100200020020010000000010120000000000000000000000020000100220000000100",
            "000000000000000200000000100000000000000000000000000000000000000000000020000100220000000100",
            "---------------000000000000000000000000000000000000000000000000000000000000000000000000000",
            "---------------000000020000000000000003000200000000000000000000000000000000000000000000000"];
        assert_eq!(r.components.iter().map(bars).collect::<Vec<_>>(), expected);
        assert_eq!(
            r.components.iter().map(|c| c.uptime).collect::<Vec<_>>(),
            [
                Some(99.92),
                Some(99.69),
                Some(99.68),
                Some(100.0),
                Some(99.91)
            ]
        );
    }
}
