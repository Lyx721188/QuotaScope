//! Official provider status pages, ported from Pulse 1.7.2 (b570dd7).
//! Copyright (c) 2026 qunqin24. Apache-2.0.
//! A failed read is never an operational reading. No account credentials,
//! usage figures or inferred uptime enter this separate public-data path.

use crate::model::Provider;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(300);
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMPONENTS: usize = 128;

pub fn watched_pages(settings: &crate::settings::AppSettings) -> HashSet<Page> {
    if !settings.alerts_on_outage {
        return HashSet::new();
    }
    settings
        .enabled_accounts
        .iter()
        .filter_map(|id| {
            crate::model::AccountKey::from_id(id).and_then(|a| Page::for_provider(a.provider))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Page {
    Codex,
    Claude,
    DeepSeek,
}

impl Page {
    pub const ALL: [Self; 3] = [Self::Codex, Self::Claude, Self::DeepSeek];
    pub fn for_provider(provider: Provider) -> Option<Self> {
        match provider {
            Provider::Codex => Some(Self::Codex),
            Provider::ClaudeCode => Some(Self::Claude),
            Provider::DeepSeek => Some(Self::DeepSeek),
            _ => None,
        }
    }
    pub fn provider(self) -> Provider {
        match self {
            Self::Codex => Provider::Codex,
            Self::Claude => Provider::ClaudeCode,
            Self::DeepSeek => Provider::DeepSeek,
        }
    }
    pub fn address(self) -> &'static str {
        match self {
            Self::Codex => "https://status.openai.com",
            Self::Claude => "https://status.claude.com",
            Self::DeepSeek => "https://status.deepseek.com",
        }
    }
    pub fn company(self) -> &'static str {
        match self {
            Self::Codex => "OpenAI",
            Self::Claude => "Anthropic",
            Self::DeepSeek => "DeepSeek",
        }
    }
    fn endpoint(self) -> &'static str {
        match self {
            Self::Codex => "https://status.openai.com/proxy/status.openai.com",
            Self::Claude => "https://status.claude.com/api/v2/summary.json",
            Self::DeepSeek => "https://status.deepseek.com/",
        }
    }
    pub fn notifies_about(self, component: &Component) -> bool {
        match self {
            Self::Codex => true, // The parser already selects the Codex group.
            Self::Claude => matches!(component.id.as_str(), "yyzkbfz2thpt" | "k8w3r06qmzrp"),
            Self::DeepSeek => component.name.contains("API"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    Operational,
    Degraded,
    PartialOutage,
    FullOutage,
    Maintenance,
    Unrecognised,
}

impl State {
    pub fn from_feed(value: &str) -> Self {
        match value {
            "operational" => Self::Operational,
            "degraded_performance" | "degraded" => Self::Degraded,
            "partial_outage" => Self::PartialOutage,
            "major_outage" | "full_outage" => Self::FullOutage,
            "under_maintenance" | "maintenance" => Self::Maintenance,
            _ => Self::Unrecognised,
        }
    }
    pub fn is_outage(self) -> bool {
        matches!(
            self,
            Self::Degraded | Self::PartialOutage | Self::FullOutage
        )
    }
    pub fn severity(self) -> u8 {
        match self {
            Self::Operational => 0,
            Self::Maintenance => 1,
            Self::Unrecognised => 2,
            Self::Degraded => 3,
            Self::PartialOutage => 4,
            Self::FullOutage => 5,
        }
    }
    pub fn title(self) -> &'static str {
        crate::localization::t(match self {
            Self::Operational => "Operational",
            Self::Degraded => "Degraded performance",
            Self::PartialOutage => "Partial outage",
            Self::FullOutage => "Full outage",
            Self::Maintenance => "Under maintenance",
            Self::Unrecognised => "Unrecognised status",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub state: State,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub page: Page,
    pub checked_at: i64,
    pub components: Vec<Component>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    Unreachable,
    Refused,
    TooLarge,
    Unreadable,
}

fn json(bytes: &[u8]) -> Result<Value, ReadError> {
    if bytes.len() > MAX_BYTES {
        return Err(ReadError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(|_| ReadError::Unreadable)
}

fn list(value: &Value) -> Result<&[Value], ReadError> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or(ReadError::Unreadable)
}

fn text(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
}

fn component(value: &Value, id: &str, state: State) -> Option<Component> {
    Some(Component {
        id: text(&value[id])?.into(),
        name: text(&value["name"])?.into(),
        state,
    })
}

fn finish(components: Vec<Component>) -> Result<Vec<Component>, ReadError> {
    let mut ids = HashSet::new();
    if components.is_empty()
        || components.len() > MAX_COMPONENTS
        || components.iter().any(|c| !ids.insert(c.id.as_str()))
    {
        return Err(ReadError::Unreadable);
    }
    Ok(components)
}

/// incident.io's summary: only visible members of the Codex group. Its
/// affected-components list must be readable before absence can mean healthy.
pub fn incident_io(bytes: &[u8]) -> Result<Vec<Component>, ReadError> {
    let value = json(bytes)?;
    let summary = &value["summary"];
    let mut affected = BTreeMap::new();
    for row in list(&summary["affected_components"])? {
        let id = text(&row["component_id"]).ok_or(ReadError::Unreadable)?;
        let state = row["status"]
            .as_str()
            .map(State::from_feed)
            .unwrap_or(State::Unrecognised);
        affected
            .entry(id)
            .and_modify(|old: &mut State| {
                if state.severity() > old.severity() {
                    *old = state;
                }
            })
            .or_insert(state);
    }
    let group = list(&summary["structure"]["items"])?
        .iter()
        .filter_map(|item| item.get("group"))
        .find(|group| group["name"] == "Codex")
        .ok_or(ReadError::Unreadable)?;
    let components = list(&group["components"])?
        .iter()
        .filter(|c| c["hidden"] != true)
        .filter_map(|c| {
            component(
                c,
                "component_id",
                text(&c["component_id"])
                    .and_then(|id| affected.get(id).copied())
                    .unwrap_or(State::Operational),
            )
        })
        .collect();
    finish(components)
}

/// Atlassian Statuspage: groups are headings, and conditional healthy rows
/// are hidden. An unknown or absent state is explicitly unrecognised.
pub fn statuspage(bytes: &[u8]) -> Result<Vec<Component>, ReadError> {
    let value = json(bytes)?;
    let mut rows: Vec<_> = list(&value["components"])?
        .iter()
        .filter(|c| {
            c["group"] != true
                && !(c["only_show_if_degraded"] == true && c["status"] == "operational")
        })
        .collect();
    rows.sort_by_key(|c| c["position"].as_i64().unwrap_or(i64::MAX));
    finish(
        rows.into_iter()
            .filter_map(|c| {
                component(
                    c,
                    "id",
                    c["status"]
                        .as_str()
                        .map(State::from_feed)
                        .unwrap_or(State::Unrecognised),
                )
            })
            .collect(),
    )
}

/// Decode Next.js Flight string literals as JSON, never as JavaScript. The
/// page distributes initialData over multiple chunks; join before reading.
fn flashcat_data(bytes: &[u8]) -> Result<Value, ReadError> {
    if bytes.len() > MAX_BYTES {
        return Err(ReadError::TooLarge);
    }
    let html = std::str::from_utf8(bytes).map_err(|_| ReadError::Unreadable)?;
    let mut remaining = html;
    let mut flight = String::new();
    while let Some(at) = remaining.find("self.__next_f.push([1,") {
        remaining = &remaining[at + "self.__next_f.push([1,".len()..];
        let mut stream = serde_json::Deserializer::from_str(remaining).into_iter::<String>();
        let literal = stream
            .next()
            .and_then(Result::ok)
            .ok_or(ReadError::Unreadable)?;
        let consumed = stream.byte_offset();
        if consumed == 0 || !remaining[consumed..].starts_with("])") {
            return Err(ReadError::Unreadable);
        }
        flight.push_str(&literal);
        remaining = &remaining[consumed + 2..];
    }
    let mut merged = serde_json::Map::new();
    for line in flight
        .lines()
        .filter(|line| line.contains("\"initialData\""))
    {
        let Some((_, payload)) = line.split_once(':') else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        let mut pending = vec![(&record, 0)];
        let mut nodes = 0;
        while let Some((node, depth)) = pending.pop() {
            nodes += 1;
            if depth > 32 || nodes > 16_384 {
                return Err(ReadError::TooLarge);
            }
            match node {
                Value::Object(object) => {
                    if let Some(data) = object.get("initialData").and_then(Value::as_object) {
                        for (key, value) in data {
                            merged.entry(key.clone()).or_insert_with(|| value.clone());
                        }
                    }
                    pending.extend(object.values().map(|v| (v, depth + 1)));
                }
                Value::Array(array) => pending.extend(array.iter().map(|v| (v, depth + 1))),
                _ => {}
            }
        }
    }
    if !merged.contains_key("page") {
        return Err(ReadError::Unreadable);
    }
    Ok(Value::Object(merged))
}

/// Flashcat: flatten visible sections in page order. Explicit component
/// states win; otherwise active impacts are the page's current-state model.
pub fn flashcat(bytes: &[u8], now_ms: i64) -> Result<Vec<Component>, ReadError> {
    let feed = flashcat_data(bytes)?;
    let visible: Vec<_> = list(&feed["page"]["components"])?
        .iter()
        .filter(|c| c["hide_all"] != true)
        .collect();
    // Missing impacts cannot prove that an unstated component is operational.
    let impacts = list(&feed["component_impacts"])?;
    let order = |c: &Value| c["order_id"].as_i64().unwrap_or(i64::MAX);
    let mut groups: Vec<(i64, Vec<&Value>)> = visible
        .iter()
        .copied()
        .filter(|c| c["section_id"].as_str().is_none_or(str::is_empty))
        .map(|c| (order(c), vec![c]))
        .collect();
    if let Some(sections) = feed["page"]["sections"].as_array() {
        for section in sections.iter().filter(|s| s["hide_all"] != true) {
            let Some(id) = text(&section["section_id"]) else {
                continue;
            };
            let mut members: Vec<_> = visible
                .iter()
                .copied()
                .filter(|c| c["section_id"] == id)
                .collect();
            members.sort_by_key(|c| order(c));
            groups.push((order(section), members));
        }
    }
    groups.sort_by_key(|group| group.0);
    let now = now_ms as f64 / 1000.0;
    let mut components = Vec::new();
    for c in groups.into_iter().flat_map(|group| group.1) {
        let Some(id) = text(&c["component_id"]) else {
            continue;
        };
        let mut current = State::Operational;
        for impact in impacts.iter().filter(|impact| impact["component_id"] == id) {
            let start = impact["start_at_seconds"].as_f64();
            let end = if impact["end_at_seconds"].is_null() {
                Some(f64::INFINITY)
            } else {
                impact["end_at_seconds"].as_f64()
            };
            let (Some(start), Some(end)) = (start, end) else {
                current = State::Unrecognised;
                continue;
            };
            if start <= now && end > now {
                let state = impact["status"]
                    .as_str()
                    .map(State::from_feed)
                    .unwrap_or(State::Unrecognised);
                if state.severity() > current.severity() {
                    current = state;
                }
            }
        }
        let state = if c["status"].is_null() || c["status"] == "" {
            current
        } else {
            c["status"]
                .as_str()
                .map(State::from_feed)
                .unwrap_or(State::Unrecognised)
        };
        if let Some(c) = component(c, "component_id", state) {
            components.push(c);
        }
    }
    finish(components)
}

pub fn parse(page: Page, bytes: &[u8], now_ms: i64) -> Result<Reading, ReadError> {
    let components = match page {
        Page::Codex => incident_io(bytes)?,
        Page::Claude => statuspage(bytes)?,
        Page::DeepSeek => flashcat(bytes, now_ms)?,
    };
    Ok(Reading {
        page,
        checked_at: now_ms,
        components,
    })
}

fn fetch(client: &reqwest::blocking::Client, page: Page) -> Result<Vec<u8>, ReadError> {
    let response = client
        .get(page.endpoint())
        .timeout(Duration::from_secs(15))
        .header(
            "Accept",
            if page == Page::DeepSeek {
                "text/html"
            } else {
                "application/json"
            },
        )
        .send()
        .map_err(|_| ReadError::Unreachable)?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(ReadError::Refused);
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err(ReadError::TooLarge);
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadError::Unreachable)?;
    if bytes.len() > MAX_BYTES {
        return Err(ReadError::TooLarge);
    }
    Ok(bytes)
}

#[derive(Default)]
struct Cached {
    attempt: Option<Instant>,
    result: Option<Result<Reading, ReadError>>,
}

impl Cached {
    fn read_with(
        &mut self,
        force: bool,
        now: Instant,
        fetch: impl FnOnce() -> Result<Reading, ReadError>,
    ) -> Result<Reading, ReadError> {
        if !force
            && self
                .attempt
                .is_some_and(|at| now.saturating_duration_since(at) < CHECK_INTERVAL)
        {
            if let Some(result) = &self.result {
                return result.clone();
            }
        }
        let result = fetch();
        self.attempt = Some(now);
        self.result = Some(result.clone());
        result
    }
}

/// Pane and monitor share each page's five-minute cache and single request.
/// Failures also cool down; manual Refresh is the sole forced retry.
pub fn read(page: Page, force: bool) -> Result<Reading, ReadError> {
    static CACHE: OnceLock<[Mutex<Cached>; 3]> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::array::from_fn(|_| Mutex::default()));
    let index = Page::ALL
        .iter()
        .position(|p| *p == page)
        .expect("known page");
    let mut slot = cache[index].lock().unwrap_or_else(|e| e.into_inner());
    slot.read_with(force, Instant::now(), || {
        let client = crate::http::HttpClient::new().client_for_login();
        fetch(&client, page).and_then(|bytes| parse(page, &bytes, crate::timeutil::now_ms()))
    })
}

/// Persisted notification transitions, separate from quota-alert memory.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutageMemory {
    #[serde(default)]
    announced: BTreeMap<String, BTreeMap<String, State>>,
}

#[derive(Default, Clone, Debug, PartialEq)]
pub struct Change {
    pub worse: Vec<Component>,
    pub recovered: Vec<Component>,
}

impl Change {
    pub fn is_empty(&self) -> bool {
        self.worse.is_empty() && self.recovered.is_empty()
    }
    pub fn notification_text(&self, page: Page) -> (String, String) {
        let mut parts = Vec::new();
        if !self.worse.is_empty() {
            let list = self
                .worse
                .iter()
                .map(|c| format!("{} ({})", c.name, c.state.title()))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(crate::localization::t_fmt(
                "{company} reports {components}.",
                &[page.company(), &list],
            ));
        }
        if !self.recovered.is_empty() {
            let list = self
                .recovered
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(crate::localization::t_fmt(
                "Back to normal: {components}.",
                &[&list],
            ));
        }
        (
            format!(
                "{} · {}",
                page.provider().display_name(),
                crate::localization::t("Service status")
            ),
            parts.join(" "),
        )
    }
}

impl OutageMemory {
    pub fn changes(&mut self, page: Page, components: &[Component]) -> Change {
        let key = page.provider().raw();
        let before = self.announced.get(key);
        let mut now = BTreeMap::new();
        let mut change = Change::default();
        for c in components.iter().filter(|c| page.notifies_about(c)) {
            let last = before.and_then(|map| map.get(&c.id)).copied();
            if c.state.is_outage() {
                if last.is_none_or(|old| c.state.severity() > old.severity()) {
                    change.worse.push(c.clone());
                }
                now.insert(c.id.clone(), c.state);
            } else if c.state == State::Operational {
                if last.is_some() {
                    change.recovered.push(c.clone());
                }
            } else if let Some(last) = last {
                now.insert(c.id.clone(), last);
            }
        }
        if now.is_empty() {
            self.announced.remove(key);
        } else {
            self.announced.insert(key.into(), now);
        }
        change
    }
    pub fn keep_only(&mut self, pages: &HashSet<Page>) {
        self.announced
            .retain(|key, _| pages.iter().any(|p| p.provider().raw() == key));
    }
    pub fn load(path: &std::path::Path) -> Self {
        let result = std::fs::File::open(path).ok().and_then(|file| {
            let mut bytes = Vec::new();
            file.take(64 * 1024 + 1).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 64 * 1024 {
                return None;
            }
            serde_json::from_slice::<Self>(&bytes).ok()
        });
        let mut memory = result.unwrap_or_default();
        memory.keep_only(&Page::ALL.into_iter().collect());
        for entries in memory.announced.values_mut() {
            entries.retain(|key, state| !key.is_empty() && key.len() <= 512 && state.is_outage());
            if entries.len() > MAX_COMPONENTS {
                entries.clear();
            }
        }
        memory
    }
    pub fn save(&self, path: &std::path::Path) {
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(self) else {
            return;
        };
        let temp = path.with_extension("json.tmp");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(&temp, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(id: &str, state: State) -> Component {
        Component {
            id: id.into(),
            name: id.into(),
            state,
        }
    }
    #[test]
    fn shared_status_cache_cools_down_success_and_failure_and_allows_manual_retry() {
        let now = Instant::now();
        let mut cache = Cached::default();
        let good = Reading {
            page: Page::Codex,
            checked_at: 10,
            components: vec![c("cli", State::Operational)],
        };
        assert_eq!(
            cache.read_with(false, now, || Ok(good.clone())),
            Ok(good.clone())
        );
        assert_eq!(
            cache.read_with(false, now + Duration::from_secs(299), || panic!(
                "repeated request"
            )),
            Ok(good)
        );
        assert_eq!(
            cache.read_with(false, now + CHECK_INTERVAL, || Err(ReadError::Unreachable)),
            Err(ReadError::Unreachable)
        );
        assert_eq!(
            cache.read_with(false, now + Duration::from_secs(301), || panic!(
                "failure retry loop"
            )),
            Err(ReadError::Unreachable)
        );
        let repaired = Reading {
            page: Page::Codex,
            checked_at: 20,
            components: vec![c("cli", State::Degraded)],
        };
        assert_eq!(
            cache.read_with(
                true,
                now + Duration::from_secs(301),
                || Ok(repaired.clone())
            ),
            Ok(repaired)
        );
    }
    #[test]
    fn upstream_frozen_public_pages_parse_visible_components() {
        let codex = incident_io(include_bytes!(
            "../tests/fixtures/windows-openai-status-operational.json"
        ))
        .unwrap();
        assert_eq!(
            codex.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["Codex Web", "Codex API", "CLI", "VS Code extension"]
        );
        assert!(codex.iter().all(|c| c.state == State::Operational));
        let claude = statuspage(include_bytes!(
            "../tests/fixtures/windows-claude-status-summary.json"
        ))
        .unwrap();
        assert_eq!(claude.len(), 6);
        assert!(claude.iter().any(|c| c.id == "yyzkbfz2thpt"));
        let deepseek = flashcat(
            include_bytes!("../tests/fixtures/windows-deepseek-status-page.html"),
            crate::timeutil::parse_iso8601_ms("2026-10-04T12:00:00+08:00").unwrap(),
        )
        .unwrap();
        assert_eq!(deepseek.len(), 5);
        assert!(deepseek.iter().any(|c| c.name.contains("API")));
    }
    fn flight(value: Value) -> Vec<u8> {
        let line = format!("1:{}\n", json!({"children": {"initialData": value}}));
        // A chunk boundary within the object must not truncate initialData.
        let mid = line.len() / 2;
        format!("<script>self.__next_f.push([1,{}])</script><script>self.__next_f.push([1,{}])</script>",
            serde_json::to_string(&line[..mid]).unwrap(), serde_json::to_string(&line[mid..]).unwrap()).into_bytes()
    }
    #[test]
    fn codex_reads_its_group_only_and_unknown_affected_state_stays_unknown() {
        let bytes = serde_json::to_vec(&json!({"summary": {
            "affected_components":[{"component_id":"cli","status":"major_outage"},
                {"component_id":"web","status":"new-state"},{"component_id":"chat","status":"major_outage"}],
            "structure":{"items":[{"group":{"name":"ChatGPT","components":[{"component_id":"chat","name":"Chat"}]}},
                {"group":{"name":"Codex","components":[{"component_id":"cli","name":"CLI"},
                    {"component_id":"web","name":"Codex Web"},{"component_id":"api","name":"Codex API"},
                    {"component_id":"hidden","name":"Private","hidden":true}, {"name":null}]}}]}
        }})).unwrap();
        let parsed = parse(Page::Codex, &bytes, 12).unwrap();
        assert_eq!(parsed.checked_at, 12);
        assert_eq!(
            parsed
                .components
                .iter()
                .map(|c| c.state)
                .collect::<Vec<_>>(),
            [State::FullOutage, State::Unrecognised, State::Operational]
        );
        let bad = bytes
            .iter()
            .copied()
            .take(bytes.len() / 2)
            .collect::<Vec<_>>();
        assert_eq!(incident_io(&bad), Err(ReadError::Unreadable));
        assert_eq!(
            incident_io(br#"{"summary":{"structure":{"items":[]}}}"#),
            Err(ReadError::Unreadable)
        );
        assert!(incident_io(br#"{"summary":{"affected_components":null,"structure":{"items":[{"group":{"name":"Codex","components":[{"component_id":"x","name":"X"}]}}]}}}"#).is_err());
    }
    #[test]
    fn claude_orders_visible_components_and_missing_status_is_unknown() {
        let bytes = serde_json::to_vec(&json!({"components":[
            {"id":"group","name":"Group","status":"operational","group":true},
            {"id":"quiet","name":"Quiet","status":"operational","only_show_if_degraded":true},
            {"id":"b","name":"B","status":"under_maintenance","position":2},
            {"id":"a","name":"A","status":"degraded_performance","position":1},
            {"id":"unknown","name":"Unknown"}, {"id":"broken","name":null}
        ]}))
        .unwrap();
        let parsed = statuspage(&bytes).unwrap();
        assert_eq!(
            parsed.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["a", "b", "unknown"]
        );
        assert_eq!(parsed[2].state, State::Unrecognised);
    }
    #[test]
    fn flashcat_joins_flight_chunks_flattens_sections_and_reads_active_impacts() {
        let bytes = flight(json!({"page":{"components":[
            {"component_id":"chat","name":"Chat","order_id":0,"status":"operational"},
            {"component_id":"api","name":"Model API","order_id":2,"section_id":"models","status":null},
            {"component_id":"explicit","name":"Explicit API","order_id":1,"section_id":"models","status":"maintenance"},
            {"component_id":"hidden","name":"Hidden","hide_all":true}],
            "sections":[{"section_id":"models","order_id":1}]},
            "component_impacts":[{"component_id":"api","status":"partial_outage","start_at_seconds":1,"end_at_seconds":null},
                {"component_id":"api","status":"full_outage","start_at_seconds":99,"end_at_seconds":null}]}));
        let parsed = flashcat(&bytes, 10_000).unwrap();
        assert_eq!(
            parsed.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["chat", "explicit", "api"]
        );
        assert_eq!(
            parsed.iter().map(|c| c.state).collect::<Vec<_>>(),
            [State::Operational, State::Maintenance, State::PartialOutage]
        );
        assert!(flashcat(b"<html>All systems operational</html>", 10_000).is_err());
    }
    #[test]
    fn malformed_flashcat_impacts_cannot_prove_recovery() {
        let value = json!({"page":{"components":[{"component_id":"api","name":"API"}]},
            "component_impacts":[{"component_id":"api","status":"full_outage","start_at_seconds":"bad"}]});
        assert_eq!(
            flashcat(&flight(value), 12).unwrap()[0].state,
            State::Unrecognised
        );
        let missing = json!({"page":{"components":[{"component_id":"api","name":"API"}]}});
        assert!(flashcat(&flight(missing), 12).is_err());
    }
    #[test]
    fn duplicate_component_ids_and_oversized_feeds_are_refused() {
        assert!(finish(vec![c("a", State::Operational), c("a", State::Operational)]).is_err());
        assert!(finish(
            (0..129)
                .map(|n| c(&n.to_string(), State::Operational))
                .collect()
        )
        .is_err());
        assert_eq!(
            parse(Page::Claude, &vec![b' '; MAX_BYTES + 1], 0),
            Err(ReadError::TooLarge)
        );
    }
    #[test]
    fn outages_announce_once_worsen_rearm_after_improvement_and_recover_once() {
        let mut memory = OutageMemory::default();
        let page = Page::Codex;
        assert!(memory
            .changes(page, &[c("cli", State::Operational)])
            .is_empty());
        assert_eq!(
            memory
                .changes(page, &[c("cli", State::Degraded)])
                .worse
                .len(),
            1
        );
        assert!(memory
            .changes(page, &[c("cli", State::Degraded)])
            .is_empty());
        assert_eq!(
            memory
                .changes(page, &[c("cli", State::FullOutage)])
                .worse
                .len(),
            1
        );
        assert!(memory
            .changes(page, &[c("cli", State::Degraded)])
            .is_empty());
        assert_eq!(
            memory
                .changes(page, &[c("cli", State::FullOutage)])
                .worse
                .len(),
            1
        );
        assert_eq!(
            memory
                .changes(page, &[c("cli", State::Operational)])
                .recovered
                .len(),
            1
        );
        assert!(memory
            .changes(page, &[c("cli", State::Operational)])
            .is_empty());
    }
    #[test]
    fn maintenance_unknown_and_failed_reads_do_not_clear_announced_outages() {
        let mut memory = OutageMemory::default();
        memory.changes(Page::Codex, &[c("cli", State::FullOutage)]);
        let before = memory.clone();
        assert!(parse(Page::Codex, b"{}", 1).is_err()); // No changes call on failed reads.
        assert!(memory
            .changes(Page::Codex, &[c("cli", State::Maintenance)])
            .is_empty());
        assert!(memory
            .changes(Page::Codex, &[c("cli", State::Unrecognised)])
            .is_empty());
        assert_eq!(memory, before);
        assert_eq!(
            memory
                .changes(Page::Codex, &[c("cli", State::Operational)])
                .recovered
                .len(),
            1
        );
    }
    #[test]
    fn removed_components_and_unwatched_pages_are_forgotten_without_recovery() {
        let mut memory = OutageMemory::default();
        memory.changes(Page::Codex, &[c("cli", State::Degraded)]);
        assert!(memory.changes(Page::Codex, &[]).is_empty());
        assert_eq!(
            memory
                .changes(Page::Codex, &[c("cli", State::Degraded)])
                .worse
                .len(),
            1
        );
        memory.changes(Page::DeepSeek, &[c("API", State::FullOutage)]);
        memory.keep_only(&[Page::DeepSeek].into_iter().collect());
        assert!(memory
            .changes(Page::Codex, &[c("cli", State::Operational)])
            .is_empty());
        assert_eq!(
            memory
                .changes(Page::DeepSeek, &[c("API", State::Operational)])
                .recovered
                .len(),
            1
        );
    }
    #[test]
    fn notifications_cover_claude_code_and_api_but_not_chat_and_deepseek_chat() {
        let mut memory = OutageMemory::default();
        let claude = memory.changes(
            Page::Claude,
            &[
                c("yyzkbfz2thpt", State::Degraded),
                c("k8w3r06qmzrp", State::FullOutage),
                c("chat", State::FullOutage),
            ],
        );
        assert_eq!(claude.worse.len(), 2);
        let deepseek = memory.changes(
            Page::DeepSeek,
            &[
                c("V4 Pro API服务", State::Degraded),
                c("网页对话", State::FullOutage),
            ],
        );
        assert_eq!(deepseek.worse.len(), 1);
    }
    #[test]
    fn outage_memory_round_trip_preserves_dedup_and_ignores_foreign_pages() {
        let mut memory = OutageMemory::default();
        memory.changes(Page::Codex, &[c("cli", State::Degraded)]);
        let bytes = serde_json::to_vec(&memory).unwrap();
        let mut loaded: OutageMemory = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(loaded, memory);
        assert!(loaded
            .changes(Page::Codex, &[c("cli", State::Degraded)])
            .is_empty());
        loaded.announced.insert("foreign".into(), BTreeMap::new());
        loaded.keep_only(&Page::ALL.into_iter().collect());
        assert_eq!(loaded, memory);
    }
    #[test]
    fn outage_switch_is_off_by_default_and_separate_from_quota_alerts() {
        let old: crate::settings::AppSettings =
            serde_json::from_str(r#"{"language":"zh"}"#).unwrap();
        assert!(!old.alerts_on_outage);
        let new = crate::settings::AppSettings {
            alerts_on_outage: true,
            ..old
        };
        assert!(!new.wants_alerts);
        assert!(!new.alerts_on_failure);
    }
    #[test]
    fn only_enabled_providers_are_watched_once_regardless_of_account_count() {
        let mut settings = crate::settings::AppSettings {
            alerts_on_outage: true,
            enabled_accounts: ["codex", "codex#1", "deepSeek#2", "cursor", "invalid"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            ..Default::default()
        };
        assert_eq!(
            watched_pages(&settings),
            [Page::Codex, Page::DeepSeek].into_iter().collect()
        );
        settings.alerts_on_outage = false;
        assert!(watched_pages(&settings).is_empty());
        settings.alerts_on_outage = true;
        settings.enabled_accounts.clear();
        assert!(watched_pages(&settings).is_empty());
    }
    #[test]
    fn outage_memory_file_can_be_replaced_and_reopened_without_repeating_alerts() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "qs-status-memory-{}-{stamp}.json",
            std::process::id()
        ));
        let mut memory = OutageMemory::default();
        memory.changes(Page::Codex, &[c("cli", State::Degraded)]);
        memory.save(&path);
        let mut reopened = OutageMemory::load(&path);
        assert_eq!(reopened, memory);
        assert!(reopened
            .changes(Page::Codex, &[c("cli", State::Degraded)])
            .is_empty());
        reopened.changes(Page::Codex, &[c("cli", State::FullOutage)]);
        reopened.save(&path);
        assert_eq!(OutageMemory::load(&path), reopened);
        std::fs::remove_file(path).unwrap();
    }
}
