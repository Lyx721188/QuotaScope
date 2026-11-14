//! Additional official feeds, adapted from Pulse 338c796 (Apache-2.0).
use super::{Component, Day, Page, ReadError, State};
use chrono::{Days, Local, TimeZone};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Format {
    Incident,
    BetterStack,
    Instatus,
    Upptime,
    Gatus,
    Robot,
    OpenCode,
    DeepInfra,
    Bob,
    Google,
}
pub(super) fn format(page: Page) -> Option<Format> {
    Some(match page {
        Page::OpenAiPlatform => Format::Incident,
        Page::JetBrainsAi => Format::Incident,
        Page::Amp => Format::Incident,
        Page::Aixy => Format::Incident,
        Page::ElevenLabs => Format::Incident,
        Page::NotionAi => Format::Incident,
        Page::Perplexity => Format::Incident,
        Page::Poe => Format::Incident,
        Page::V0 => Format::Incident,
        Page::HuggingFace => Format::BetterStack,
        Page::Chutes => Format::BetterStack,
        Page::Synthetic => Format::BetterStack,
        Page::DevPass => Format::BetterStack,
        Page::RaycastAi => Format::BetterStack,
        Page::TypeSafe => Format::BetterStack,
        Page::Zed => Format::Instatus,
        Page::GitKraken => Format::Instatus,
        Page::AtlasCloud => Format::Instatus,
        Page::CommandCode => Format::Upptime,
        Page::ClinePass => Format::Gatus,
        Page::Hyper => Format::Robot,
        Page::OpenCodeGo => Format::OpenCode,
        Page::DeepInfra => Format::DeepInfra,
        Page::IbmBob => Format::Bob,
        Page::Gemini => Format::Google,
        _ => return None,
    })
}

pub(super) fn endpoint(page: Page) -> Option<String> {
    let root = page.address();
    Some(match format(page)? {
        Format::Incident => format!("{root}/proxy/{}",url::Url::parse(root).ok()?.host_str()?),
        Format::BetterStack => format!("{root}/index.json"),
        Format::Instatus => format!("{root}/v2/components.json"),
        Format::Upptime => "https://raw.githubusercontent.com/CommandCodeAI/commandcode-status/master/history/summary.json".into(),
        Format::Gatus => format!("{root}/api/v1/endpoints/statuses"),
        Format::Robot => format!("{root}/api/getMonitorList/JrppyTBzE3?page=1"),
        Format::OpenCode => format!("{root}/api/status"),
        Format::DeepInfra | Format::Bob => format!("{root}/api/v1/status"),
        Format::Google => format!("{root}/incidents.json"),
    })
}

pub(super) fn notifies(page: Page, component: &Component) -> Option<bool> {
    let ids: &[&str] = match page {
        Page::JetBrainsAi => &["01KX3EN535A0SKSZK3S84949V1"],
        Page::Aixy => &["01M08789J59XQG5AR373SVQZ6Z"],
        Page::Perplexity => &["01M2R6E5FPASTEW3A3V8TSYHJ0"],
        Page::V0 => &["01KHYYTCKK0NS8FN857PCJB63W", "01J5EED92VTMVYNWYKYS9B1G1F"],
        Page::HuggingFace => &["306621", "84641"],
        Page::Chutes => &["8599285"],
        Page::DevPass => &["8768001"],
        Page::RaycastAi => &["5341215"],
        Page::TypeSafe => &["8735312"],
        Page::Zed => &["cmaw974wo0001xh7v9vfihlul", "cm75bq8t9000rsdcb9bfls8wt"],
        Page::GitKraken => &["clvkz1ojq2917bgo9peyj3bha", "clvkz1ok12924bgo956mkaa0r"],
        Page::OpenCodeGo => &["go"],
        Page::DeepInfra => &["api"],
        Page::IbmBob => &["Chat/Code Completion", "Login/SSO"],
        Page::Synthetic => {
            return Some(
                component.group.as_deref() == Some("Synthetic - API (Synthetic hosted Models)"),
            )
        }
        _ => return format(page).map(|_| true),
    };
    Some(ids.contains(&component.id.as_str()))
}

fn row(
    id: impl Into<String>,
    name: impl Into<String>,
    state: State,
    group: Option<String>,
) -> Component {
    Component {
        id: id.into(),
        name: name.into(),
        state,
        group,
        days: Vec::new(),
        uptime: None,
    }
}
fn text(value: &Value) -> Option<String> {
    super::text(value).map(str::to_owned)
}
fn word(value: &Value) -> State {
    value
        .as_str()
        .map(State::from_feed)
        .unwrap_or(State::Unrecognised)
}
fn numeric(value: &Value) -> Option<f64> {
    crate::http::number(value).filter(|n| n.is_finite())
}
fn date(value: &Value) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(value.as_str()?.get(..10)?, "%Y-%m-%d").ok()
}
fn bar(date: chrono::NaiveDate, state: Option<State>) -> Day {
    Day {
        date,
        state,
        rgb: None,
        start_ms: None,
        hours: None,
    }
}
fn better_state(value: &Value) -> State {
    match value.as_str() {
        Some("operational") => State::Operational,
        Some("degraded" | "recovered_degraded") => State::Degraded,
        Some("downtime") => State::FullOutage,
        Some("maintenance") => State::Maintenance,
        _ => State::Unrecognised,
    }
}

pub(super) fn parse(page: Page, bytes: &[u8], now: i64) -> Result<Vec<Component>, ReadError> {
    let root = super::json(bytes)?;
    let mut out = Vec::new();
    match format(page).ok_or(ReadError::Unreadable)? {
        Format::Incident => return incident(page, root),
        Format::BetterStack => {
            let included = super::list(&root["included"])?;
            let sections: HashMap<String, (&Value, i64)> = included
                .iter()
                .filter(|r| r["type"] == "status_page_section")
                .filter_map(|r| {
                    Some((
                        text(&r["id"])?,
                        (
                            &r["attributes"],
                            r["attributes"]["position"].as_i64().unwrap_or(-1),
                        ),
                    ))
                })
                .collect();
            let mut resources: Vec<_> = included
                .iter()
                .filter(|r| r["type"] == "status_page_resource")
                .collect();
            resources.sort_by_key(|r| {
                let section = r["attributes"]["status_page_section_id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| {
                        r["attributes"]["status_page_section_id"]
                            .as_i64()
                            .map(|n| n.to_string())
                    })
                    .unwrap_or_default();
                (
                    sections.get(&section).map(|r| r.1).unwrap_or(-1),
                    r["attributes"]["position"].as_i64().unwrap_or(i64::MAX),
                )
            });
            for r in resources {
                let a = &r["attributes"];
                let (Some(id), Some(name)) = (text(&r["id"]), text(&a["public_name"])) else {
                    continue;
                };
                let section = a["status_page_section_id"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| a["status_page_section_id"].as_i64().map(|n| n.to_string()))
                    .unwrap_or_default();
                let group = sections.get(&section).and_then(|s| text(&s.0["name"]));
                let mut c = row(id, name, better_state(&a["status"]), group);
                c.uptime = numeric(&a["availability"])
                    .filter(|n| (0.0..=1.0).contains(n))
                    .map(|n| n * 100.0);
                c.days = a["status_history"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(366)
                    .filter_map(|d| {
                        Some(bar(
                            date(&d["day"])?,
                            if d["status"] == "not_monitored" {
                                None
                            } else {
                                Some(better_state(&d["status"]))
                            },
                        ))
                    })
                    .collect();
                out.push(c);
            }
        }
        Format::Instatus => {
            let rows = super::list(&root["components"])?;
            let headers: HashSet<_> = rows
                .iter()
                .filter_map(|r| r.pointer("/group/id").and_then(Value::as_str))
                .collect();
            for r in rows {
                let (Some(id), Some(name)) = (text(&r["id"]), text(&r["name"])) else {
                    continue;
                };
                if headers.contains(id.as_str())
                    || page == Page::Zed
                        && r.pointer("/group/name").and_then(Value::as_str) == Some("Delta")
                {
                    continue;
                }
                out.push(row(
                    id,
                    name,
                    word(&r["status"]),
                    r.pointer("/group/name").and_then(text),
                ));
            }
        }
        Format::Upptime => {
            for r in super::list(&root)? {
                let (Some(id), Some(name)) = (text(&r["slug"]), text(&r["name"])) else {
                    continue;
                };
                out.push(row(
                    id,
                    name,
                    match r["status"].as_str() {
                        Some("up") => State::Operational,
                        Some("down") => State::FullOutage,
                        Some("degraded") => State::Degraded,
                        _ => State::Unrecognised,
                    },
                    None,
                ));
            }
        }
        Format::Gatus => {
            for r in super::list(&root)? {
                let (Some(id), Some(name)) = (text(&r["key"]), text(&r["name"])) else {
                    continue;
                };
                let last = r["results"].as_array().and_then(|a| a.last());
                let state = match last {
                    Some(r) if r["success"] == true => State::Operational,
                    Some(r)
                        if r["success"] == false
                            && r["status"]
                                .as_u64()
                                .is_some_and(|n| (200..300).contains(&n)) =>
                    {
                        State::Degraded
                    }
                    Some(r) if r["success"] == false => State::FullOutage,
                    _ => State::Unrecognised,
                };
                out.push(row(id, name, state, text(&r["group"])));
            }
        }
        Format::Robot => {
            for r in super::list(&root["data"])? {
                let Some(id) = r["monitorId"].as_i64().map(|n| n.to_string()) else {
                    continue;
                };
                if id != "803945156" {
                    continue;
                }
                let Some(name) = text(&r["name"]) else {
                    continue;
                };
                let mut c = row(
                    id,
                    name,
                    match r["statusClass"].as_str() {
                        Some("success") => State::Operational,
                        Some("warning") => State::Degraded,
                        Some("danger") => State::FullOutage,
                        _ => State::Unrecognised,
                    },
                    None,
                );
                c.uptime = numeric(&r["90dRatio"]["ratio"]).filter(|n| (0.0..=100.0).contains(n));
                c.days = r["dailyRatios"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(366)
                    .filter_map(|d| {
                        let state = if matches!(d["color"].as_str(), Some("grey" | "black")) {
                            None
                        } else {
                            numeric(&d["ratio"]).map(|n| {
                                if n >= 100.0 {
                                    State::Operational
                                } else if n <= 0.0 {
                                    State::FullOutage
                                } else {
                                    State::PartialOutage
                                }
                            })
                        };
                        Some(bar(date(&d["date"])?, state))
                    })
                    .collect();
                out.push(c);
            }
        }
        Format::OpenCode => {
            let slot = 3 * 3600 * 1000i64;
            let current = now.div_euclid(slot) * slot;
            let history = super::list(&root["history"])?;
            for r in super::list(&root["components"])? {
                let Some(id) = text(&r["service"]) else {
                    continue;
                };
                let name = match id.as_str() {
                    "console" => "Console API",
                    "logs" => "Request logs",
                    "inference" => "Inference API",
                    "go" => "Go",
                    "payg" => "Pay-as-you-go",
                    "free" => "Free inference",
                    s => s,
                };
                let mut c = row(id.clone(), name, word(&r["status"]), None);
                c.days = (0..56)
                    .rev()
                    .filter_map(|back| {
                        let start = current - back * slot;
                        let value = history
                            .iter()
                            .find(|r| numeric(&r["start"]) == Some(start as f64))
                            .and_then(|r| r["services"].get(&id));
                        let state = value.filter(|v| v.as_str() != Some("unknown")).map(word);
                        Some(Day {
                            date: chrono::DateTime::from_timestamp_millis(start)?
                                .with_timezone(&Local)
                                .date_naive(),
                            state,
                            rgb: None,
                            start_ms: Some(start),
                            hours: Some(3),
                        })
                    })
                    .collect();
                out.push(c);
            }
        }
        Format::DeepInfra => {
            for r in super::list(&root["services"])? {
                let (Some(id), Some(name)) = (text(&r["publicId"]), text(&r["displayName"])) else {
                    continue;
                };
                out.push(row(id, name, word(&r["status"]), None));
            }
        }
        Format::Bob => {
            for region in super::list(&root["regions"])? {
                for r in super::list(&region["services"])? {
                    let Some(name) = text(&r["name"]) else {
                        continue;
                    };
                    let state = word(&r["status"]);
                    if let Some(old) = out.iter_mut().find(|c| c.id == name) {
                        if state.severity() > old.state.severity() {
                            old.state = state;
                        }
                    } else {
                        out.push(row(name.clone(), name, state, None));
                    }
                }
            }
        }
        Format::Google => {
            let rows = super::list(&root)?;
            let product = "deUeOEPYanfJ9w8cpyBJ";
            let mut c = row(product, "Gemini Code Assist", State::Operational, None);
            let mut impacts = Vec::new();
            for r in rows {
                if !r["affected_products"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|p| p["id"] == product))
                {
                    continue;
                }
                let Some(start) = r["begin"]
                    .as_str()
                    .and_then(crate::timeutil::parse_iso8601_ms)
                else {
                    continue;
                };
                let end = if r["end"].is_null() {
                    now
                } else {
                    let Some(end) = r["end"]
                        .as_str()
                        .and_then(crate::timeutil::parse_iso8601_ms)
                    else {
                        continue;
                    };
                    end
                };
                let state = match r["status_impact"].as_str() {
                    Some("SERVICE_OUTAGE") => State::FullOutage,
                    Some("SERVICE_DISRUPTION") => State::PartialOutage,
                    Some("SERVICE_INFORMATION") => State::Degraded,
                    _ => State::Unrecognised,
                };
                if start <= now && end >= now && state.severity() > c.state.severity() {
                    c.state = state;
                }
                impacts.push((start, end, state));
            }
            let today = chrono::DateTime::from_timestamp_millis(now)
                .ok_or(ReadError::Unreadable)?
                .with_timezone(&Local)
                .date_naive();
            c.days = (0..90)
                .rev()
                .filter_map(|back| {
                    let day = today.checked_sub_days(Days::new(back))?;
                    let start = Local
                        .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
                        .earliest()?
                        .timestamp_millis();
                    let end = Local
                        .from_local_datetime(&day.succ_opt()?.and_hms_opt(0, 0, 0)?)
                        .earliest()?
                        .timestamp_millis();
                    Some(bar(
                        day,
                        Some(
                            impacts
                                .iter()
                                .filter(|(s, e, _)| *s < end && *e > start)
                                .map(|(_, _, state)| *state)
                                .max_by_key(|s| s.severity())
                                .unwrap_or(State::Operational),
                        ),
                    ))
                })
                .collect();
            out.push(c);
        }
    }
    super::finish(out)
}

fn incident(page: Page, mut root: Value) -> Result<Vec<Component>, ReadError> {
    let summary = &root["summary"];
    let mut members = Vec::new();
    for item in super::list(&summary["structure"]["items"])? {
        let (rows, group): (Vec<&Value>, Option<&str>) = if let Some(group) = item.get("group") {
            if group["hidden"] == true {
                continue;
            }
            (
                super::list(&group["components"])?.iter().collect(),
                group["name"].as_str(),
            )
        } else {
            (item.get("component").into_iter().collect(), None)
        };
        for r in rows {
            if r["hidden"] == true {
                continue;
            }
            let id = r["component_id"].as_str().unwrap_or("");
            let shows = match page {
                Page::OpenAiPlatform => group == Some("APIs"),
                Page::JetBrainsAi => group != Some("AI Providers"),
                Page::Amp => {
                    ["01KBQ3ZK9NMBWKR7DY9H7NQ17D", "01M0WN9KPF89TDEHSPQ918XHTH"].contains(&id)
                }
                Page::ElevenLabs => group == Some("ElevenAPI"),
                Page::NotionAi => group == Some("Notion AI"),
                _ => true,
            };
            if shows {
                members.push(r.clone());
            }
        }
    }
    root["summary"]["structure"]["items"] =
        json!([{"group":{"name":"Codex","components":members}}]);
    if root["summary"]["affected_components"].is_null() {
        root["summary"]["affected_components"] = json!([]);
    }
    super::incident_io(&serde_json::to_vec(&root).map_err(|_| ReadError::Unreadable)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(page: Page, value: Value) -> Vec<Component> {
        parse(page, &serde_json::to_vec(&value).unwrap(), 1794470400000).unwrap()
    }
    #[test]
    fn incident_products_do_not_show_another_products_outage() {
        let rows = read(
            Page::OpenAiPlatform,
            json!({"summary":{"affected_components":[{"component_id":"codex","status":"major_outage"}],"structure":{"items":[{"group":{"name":"APIs","components":[{"component_id":"api","name":"API"}]}},{"group":{"name":"Codex","components":[{"component_id":"codex","name":"Codex"}]}}]}}}),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "api");
        assert_eq!(rows[0].state, State::Operational);
    }
    #[test]
    fn better_stack_preserves_unmonitored_days_and_official_availability() {
        let rows = read(
            Page::Synthetic,
            json!({"included":[{"type":"status_page_section","id":"1","attributes":{"name":"Synthetic - API (Synthetic hosted Models)","position":0}},{"type":"status_page_resource","id":"api","attributes":{"public_name":"Inference","status_page_section_id":1,"status":"not_monitored","availability":"0.99","status_history":[{"day":"2026-11-11","status":"not_monitored"},{"day":"2026-11-12","status":"downtime"}]}}]}),
        );
        assert_eq!(rows[0].state, State::Unrecognised);
        assert_eq!(rows[0].days[0].state, None);
        assert_eq!(rows[0].days[1].state, Some(State::FullOutage));
        assert_eq!(rows[0].uptime, Some(99.0));
        assert_eq!(notifies(Page::Synthetic, &rows[0]), Some(true));
    }
    #[test]
    fn monitor_feeds_use_the_last_real_check_and_do_not_invent_history() {
        let rows = read(
            Page::ClinePass,
            json!([{"key":"api","name":"API","results":[{"success":true,"status":200},{"success":false,"status":200}]}]),
        );
        assert_eq!(rows[0].state, State::Degraded);
        assert!(rows[0].days.is_empty());
        assert_eq!(
            read(
                Page::CommandCode,
                json!([{"slug":"api","name":"API","status":"down"}])
            )[0]
            .state,
            State::FullOutage
        );
        let rows = read(
            Page::Zed,
            json!({"components":[{"id":"main","name":"API","status":"operational"},{"id":"other","name":"Delta","status":"major_outage","group":{"id":"g","name":"Delta"}}]}),
        );
        assert_eq!(rows.len(), 1);
    }
    #[test]
    fn company_feeds_keep_three_hour_gaps_and_worst_regional_state() {
        let rows = read(
            Page::OpenCodeGo,
            json!({"components":[{"service":"go","status":"operational"}],"history":[]}),
        );
        assert_eq!(rows[0].days.len(), 56);
        assert!(rows[0]
            .days
            .iter()
            .all(|d| d.state.is_none() && d.hours == Some(3)));
        let rows = read(
            Page::IbmBob,
            json!({"regions":[{"services":[{"name":"Login/SSO","status":"operational"}]},{"services":[{"name":"Login/SSO","status":"major_outage"}]}]}),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, State::FullOutage);
        assert_eq!(read(Page::Gemini, json!([]))[0].days.len(), 90);
        assert!(parse(Page::Gemini, b"{}", 1794470400000).is_err());
    }
    #[test]
    fn paused_uptime_robot_days_have_no_state_and_other_monitors_are_excluded() {
        let rows = read(
            Page::Hyper,
            json!({"data":[{"monitorId":803945156i64,"name":"Hyper","statusClass":"success","dailyRatios":[{"date":"2026-11-12","ratio":"100","color":"grey"}],"90dRatio":{"ratio":"99.9"}},{"monitorId":1,"name":"Other","statusClass":"danger","dailyRatios":[]}]}),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].days[0].state, None);
        assert_eq!(rows[0].uptime, Some(99.9));
        let rows = read(
            Page::DeepInfra,
            json!({"services":[{"publicId":"api","displayName":"Inference API","status":"operational"}],"models":[{"publicId":"model","displayName":"Other","status":"major_outage"}]}),
        );
        assert_eq!(rows.len(), 1);
    }
}
