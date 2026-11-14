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
    Cursor,
    GrokBot,
    Copilot,
    Devin,
    Windsurf,
    Augment,
    Factory,
    KiloCode,
    Minimax,
    MinimaxCn,
    KimiCode,
    Moonshot,
    Warp,
    Manus,
    Venice,
    ZoomMate,
    Replicate,
    OpenAiPlatform,
    JetBrainsAi,
    Amp,
    Aixy,
    ElevenLabs,
    NotionAi,
    Perplexity,
    Poe,
    V0,
    HuggingFace,
    Chutes,
    Synthetic,
    DevPass,
    RaycastAi,
    TypeSafe,
    Zed,
    GitKraken,
    AtlasCloud,
    CommandCode,
    ClinePass,
    Hyper,
    OpenCodeGo,
    DeepInfra,
    IbmBob,
    Gemini,
}

impl Page {
    pub const ALL: [Self; 45] = [
        Self::Codex,
        Self::Claude,
        Self::DeepSeek,
        Self::Cursor,
        Self::GrokBot,
        Self::Copilot,
        Self::Devin,
        Self::Windsurf,
        Self::Augment,
        Self::Factory,
        Self::KiloCode,
        Self::Minimax,
        Self::MinimaxCn,
        Self::KimiCode,
        Self::Moonshot,
        Self::Warp,
        Self::Manus,
        Self::Venice,
        Self::ZoomMate,
        Self::Replicate,
        Self::OpenAiPlatform,
        Self::JetBrainsAi,
        Self::Amp,
        Self::Aixy,
        Self::ElevenLabs,
        Self::NotionAi,
        Self::Perplexity,
        Self::Poe,
        Self::V0,
        Self::HuggingFace,
        Self::Chutes,
        Self::Synthetic,
        Self::DevPass,
        Self::RaycastAi,
        Self::TypeSafe,
        Self::Zed,
        Self::GitKraken,
        Self::AtlasCloud,
        Self::CommandCode,
        Self::ClinePass,
        Self::Hyper,
        Self::OpenCodeGo,
        Self::DeepInfra,
        Self::IbmBob,
        Self::Gemini,
    ];
    pub fn for_provider(provider: Provider) -> Option<Self> {
        match provider {
            Provider::Codex => Some(Self::Codex),
            Provider::ClaudeCode => Some(Self::Claude),
            Provider::DeepSeek => Some(Self::DeepSeek),
            Provider::OpenAiPlatform => Some(Self::OpenAiPlatform),
            Provider::JetBrainsAi => Some(Self::JetBrainsAi),
            Provider::Amp => Some(Self::Amp),
            Provider::Aixy => Some(Self::Aixy),
            Provider::ElevenLabs => Some(Self::ElevenLabs),
            Provider::NotionAi => Some(Self::NotionAi),
            Provider::Perplexity => Some(Self::Perplexity),
            Provider::Poe => Some(Self::Poe),
            Provider::V0 => Some(Self::V0),
            Provider::HuggingFace => Some(Self::HuggingFace),
            Provider::Chutes => Some(Self::Chutes),
            Provider::Synthetic => Some(Self::Synthetic),
            Provider::DevPass => Some(Self::DevPass),
            Provider::RaycastAi => Some(Self::RaycastAi),
            Provider::TypeSafe => Some(Self::TypeSafe),
            Provider::Zed => Some(Self::Zed),
            Provider::GitKraken => Some(Self::GitKraken),
            Provider::AtlasCloud => Some(Self::AtlasCloud),
            Provider::CommandCode => Some(Self::CommandCode),
            Provider::ClinePass => Some(Self::ClinePass),
            Provider::Hyper => Some(Self::Hyper),
            Provider::OpenCodeGo => Some(Self::OpenCodeGo),
            Provider::DeepInfra => Some(Self::DeepInfra),
            Provider::IbmBob => Some(Self::IbmBob),
            Provider::Gemini => Some(Self::Gemini),

            Provider::Cursor => Some(Self::Cursor),
            Provider::GrokBot => Some(Self::GrokBot),
            Provider::Copilot => Some(Self::Copilot),
            Provider::Devin => Some(Self::Devin),
            Provider::Windsurf => Some(Self::Windsurf),
            Provider::Augment => Some(Self::Augment),
            Provider::Factory => Some(Self::Factory),
            Provider::KiloCode => Some(Self::KiloCode),
            Provider::Minimax => Some(if crate::settings::with(|s| s.minimax_site == "china") {
                Self::MinimaxCn
            } else {
                Self::Minimax
            }),
            Provider::MinimaxCn => Some(Self::MinimaxCn),
            Provider::KimiCode => Some(Self::KimiCode),
            Provider::Moonshot => Some(Self::Moonshot),
            Provider::Warp => Some(Self::Warp),
            Provider::Manus => Some(Self::Manus),
            Provider::Venice => Some(Self::Venice),
            Provider::ZoomMate => Some(Self::ZoomMate),
            Provider::Replicate => Some(Self::Replicate),

            _ => None,
        }
    }
    pub fn provider(self) -> Provider {
        match self {
            Self::Codex => Provider::Codex,
            Self::Claude => Provider::ClaudeCode,
            Self::DeepSeek => Provider::DeepSeek,
            Self::OpenAiPlatform => Provider::OpenAiPlatform,
            Self::JetBrainsAi => Provider::JetBrainsAi,
            Self::Amp => Provider::Amp,
            Self::Aixy => Provider::Aixy,
            Self::ElevenLabs => Provider::ElevenLabs,
            Self::NotionAi => Provider::NotionAi,
            Self::Perplexity => Provider::Perplexity,
            Self::Poe => Provider::Poe,
            Self::V0 => Provider::V0,
            Self::HuggingFace => Provider::HuggingFace,
            Self::Chutes => Provider::Chutes,
            Self::Synthetic => Provider::Synthetic,
            Self::DevPass => Provider::DevPass,
            Self::RaycastAi => Provider::RaycastAi,
            Self::TypeSafe => Provider::TypeSafe,
            Self::Zed => Provider::Zed,
            Self::GitKraken => Provider::GitKraken,
            Self::AtlasCloud => Provider::AtlasCloud,
            Self::CommandCode => Provider::CommandCode,
            Self::ClinePass => Provider::ClinePass,
            Self::Hyper => Provider::Hyper,
            Self::OpenCodeGo => Provider::OpenCodeGo,
            Self::DeepInfra => Provider::DeepInfra,
            Self::IbmBob => Provider::IbmBob,
            Self::Gemini => Provider::Gemini,

            Self::Cursor => Provider::Cursor,
            Self::GrokBot => Provider::GrokBot,
            Self::Copilot => Provider::Copilot,
            Self::Devin => Provider::Devin,
            Self::Windsurf => Provider::Windsurf,
            Self::Augment => Provider::Augment,
            Self::Factory => Provider::Factory,
            Self::KiloCode => Provider::KiloCode,
            Self::Minimax => Provider::Minimax,
            Self::MinimaxCn => Provider::Minimax,
            Self::KimiCode => Provider::KimiCode,
            Self::Moonshot => Provider::Moonshot,
            Self::Warp => Provider::Warp,
            Self::Manus => Provider::Manus,
            Self::Venice => Provider::Venice,
            Self::ZoomMate => Provider::ZoomMate,
            Self::Replicate => Provider::Replicate,
        }
    }
    pub fn address(self) -> &'static str {
        match self {
            Self::Codex => "https://status.openai.com",
            Self::Claude => "https://status.claude.com",
            Self::DeepSeek => "https://status.deepseek.com",
            Self::OpenAiPlatform => "https://status.openai.com",
            Self::JetBrainsAi => "https://status.jetbrains.cloud",
            Self::Amp => "https://ampcodestatus.com",
            Self::Aixy => "https://status.aixy-gateway.com",
            Self::ElevenLabs => "https://status.elevenlabs.io",
            Self::NotionAi => "https://www.notion-status.com",
            Self::Perplexity => "https://status.perplexity.com",
            Self::Poe => "https://status.poe.com",
            Self::V0 => "https://v0-status.com",
            Self::HuggingFace => "https://status.huggingface.co",
            Self::Chutes => "https://status.chutes.ai",
            Self::Synthetic => "https://status.synthetic.new",
            Self::DevPass => "https://status.llmgateway.io",
            Self::RaycastAi => "https://status.raycast.com",
            Self::TypeSafe => "https://status.typesafe.ai",
            Self::Zed => "https://status.zed.dev",
            Self::GitKraken => "https://cloud.gitkrakenstatus.com",
            Self::AtlasCloud => "https://atlascloud.instatus.com",
            Self::CommandCode => "https://status.commandcode.ai",
            Self::ClinePass => "https://status.cline.bot",
            Self::Hyper => "https://status.charm.sh",
            Self::OpenCodeGo => "https://status.opencode.ai",
            Self::DeepInfra => "https://status.deepinfra.com",
            Self::IbmBob => "https://status.bob.ibm.com",
            Self::Gemini => "https://status.cloud.google.com",

            Self::Cursor => "https://status.cursor.com",
            Self::GrokBot => "https://status.cursor.com",
            Self::Copilot => "https://www.githubstatus.com",
            Self::Devin => "https://www.devinstatus.com",
            Self::Windsurf => "https://status.windsurf.com",
            Self::Augment => "https://status.augmentcode.com",
            Self::Factory => "https://status.factory.ai",
            Self::KiloCode => "https://status.kilo.ai",
            Self::Minimax => "https://status.minimax.io",
            Self::MinimaxCn => "https://status.minimaxi.com",
            Self::KimiCode => "https://status.moonshot.cn",
            Self::Moonshot => "https://status.moonshot.cn",
            Self::Warp => "https://status.warp.dev",
            Self::Manus => "https://status.manus.im",
            Self::Venice => "https://veniceai-status.com",
            Self::ZoomMate => "https://www.zoomstatus.com",
            Self::Replicate => "https://www.cloudflarestatus.com",
        }
    }
    pub fn company(self) -> &'static str {
        match self {
            Self::Codex => "OpenAI",
            Self::Claude => "Anthropic",
            Self::DeepSeek => "DeepSeek",
            Self::OpenAiPlatform => "OpenAI",
            Self::JetBrainsAi => "JetBrains",
            Self::Amp => "Amp",
            Self::Aixy => "AIXY",
            Self::ElevenLabs => "ElevenLabs",
            Self::NotionAi => "Notion",
            Self::Perplexity => "Perplexity",
            Self::Poe => "Poe",
            Self::V0 => "Vercel",
            Self::HuggingFace => "Hugging Face",
            Self::Chutes => "Chutes",
            Self::Synthetic => "Synthetic",
            Self::DevPass => "LLM Gateway",
            Self::RaycastAi => "Raycast",
            Self::TypeSafe => "TypeSafe",
            Self::Zed => "Zed",
            Self::GitKraken => "GitKraken",
            Self::AtlasCloud => "Atlas Cloud",
            Self::CommandCode => "Command Code",
            Self::ClinePass => "Cline",
            Self::Hyper => "Charm",
            Self::OpenCodeGo => "OpenCode",
            Self::DeepInfra => "DeepInfra",
            Self::IbmBob => "IBM",
            Self::Gemini => "Google Cloud",

            Self::Cursor => "Cursor",
            Self::GrokBot => "Cursor",
            Self::Copilot => "GitHub",
            Self::Devin => "Cognition",
            Self::Windsurf => "Windsurf",
            Self::Augment => "Augment Code",
            Self::Factory => "Factory",
            Self::KiloCode => "Kilo",
            Self::Minimax => "MiniMax",
            Self::MinimaxCn => "MiniMax",
            Self::KimiCode => "Moonshot AI",
            Self::Moonshot => "Moonshot AI",
            Self::Warp => "Warp",
            Self::Manus => "Manus",
            Self::Venice => "Venice",
            Self::ZoomMate => "Zoom",
            Self::Replicate => "Cloudflare",
        }
    }
    pub(crate) fn is_incident(self) -> bool {
        self == Self::Codex || extra::format(self) == Some(extra::Format::Incident)
    }
    pub(crate) fn endpoint(self) -> String {
        if let Some(url) = extra::endpoint(self) {
            return url;
        }
        match self {
            Self::Codex => "https://status.openai.com/proxy/status.openai.com".into(),
            Self::DeepSeek => "https://status.deepseek.com/".into(),
            _ => format!("{}/api/v2/summary.json", self.address()),
        }
    }
    pub fn notifies_about(self, component: &Component) -> bool {
        if let Some(result) = extra::notifies(self, component) {
            return result;
        }
        match self {
            Self::Codex => true, // The parser already selects the Codex group.
            Self::Claude => matches!(component.id.as_str(), "yyzkbfz2thpt" | "k8w3r06qmzrp"),
            Self::DeepSeek => component.name.contains("API"),
            Self::Cursor => {
                ["rflc60xp5jp2", "vsny1qv7v86c", "mwv1g9sc7kdh"].contains(&component.id.as_str())
            }
            Self::Devin => ["q72cy1kjpk4r", "d87cp5jknh1c"].contains(&component.id.as_str()),
            Self::Augment => component.id == "1k3gvg4j889s",
            Self::Factory => {
                ["kfqc4mfdkrqx", "jfcmwr3grbyc", "z1frly8fm1l5"].contains(&component.id.as_str())
            }
            Self::KiloCode => ["kdk5t7ljzmm1", "rkvq7kvz2zlh"].contains(&component.id.as_str()),
            Self::Minimax => component.id == "pr0d8qr59svt",
            Self::MinimaxCn => component.id == "vwp8mgy34fck",
            Self::Moonshot => ["8psr5dfdld0s", "pjrph68g94yr"].contains(&component.id.as_str()),
            Self::Warp => {
                ["z7hlxwbqx7ry", "z90zfbttkx9f", "7z5yq9wznb4f"].contains(&component.id.as_str())
            }
            Self::Venice => component.id == "6knqw5vybqzk",
            _ => true,
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
    pub days: Vec<Day>,
    pub uptime: Option<f64>,
    pub group: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: chrono::NaiveDate,
    pub state: Option<State>,
    pub rgb: Option<u32>,
    pub start_ms: Option<i64>,
    pub hours: Option<u8>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub page: Page,
    pub checked_at: i64,
    pub components: Vec<Component>,
    /// A failed or incomplete history leaves the current state standing.
    pub history_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    Http(u16),
    Unreachable,
    Refused,
    TooLarge,
    Unreadable,
}

impl ReadError {
    pub fn message(self) -> String {
        use crate::localization::{t, t_fmt};
        match self {
            Self::Http(code) => t_fmt("Status page returned HTTP {code}.", &[&code.to_string()]),
            Self::Unreachable => t("The status page request failed to connect.").into(),
            Self::Refused => t("The status page refused the request.").into(),
            Self::TooLarge => t("The status page response exceeded the read limit.").into(),
            Self::Unreadable => {
                t("The page contained no readable components for this provider.").into()
            }
        }
    }
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
        days: Vec::new(),
        uptime: None,
        group: None,
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

/// Product rows only. Supplier outages and unrelated products must not
/// become notifications for this account. Rules follow Pulse 338c796.
fn statuspage_for(page: Page, bytes: &[u8]) -> Result<Vec<Component>, ReadError> {
    let mut root = json(bytes)?;
    let rows = list(&root["components"])?;
    let selected: Vec<_> = rows
        .iter()
        .filter(|row| {
            let id = row["id"].as_str().unwrap_or("");
            let group = row["group_id"].as_str().unwrap_or("");
            match page {
                Page::Cursor => id != "sm5wkcnqkvr9",
                Page::GrokBot => id == "sm5wkcnqkvr9",
                Page::Copilot => ["pjmpxvq2cmr2", "cnnb39dkkk82"].contains(&id),
                Page::Devin => !["92j5h0ttqxfb", "9ydbk2b55pk1"].contains(&group),
                Page::Windsurf => ["r5wf1ykd7y1m", "8q19cygxvshj"].contains(&id),
                Page::Factory => group != "c9sjq8nsz2kq",
                Page::KiloCode => group != "frgc645htq00",
                Page::KimiCode => group == "cyrtlddz0q8c",
                Page::Moonshot => group == "87jpctq5cp71",
                Page::ZoomMate => id == "n1llzwc1cxtr",
                Page::Replicate => id == "fvgfcmy66tdr",
                _ => true,
            }
        })
        .cloned()
        .collect();
    root["components"] = Value::Array(selected);
    statuspage(&serde_json::to_vec(&root).map_err(|_| ReadError::Unreadable)?)
}

/// Decode Next.js Flight string literals as JSON, never as JavaScript. The
/// page distributes initialData over multiple chunks; join before reading.
pub(crate) fn flashcat_data(bytes: &[u8]) -> Result<Value, ReadError> {
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
                            if matches!(
                                key.as_str(),
                                "page" | "component_impacts" | "component_uptimes"
                            ) {
                                merged.entry(key.clone()).or_insert_with(|| value.clone());
                            }
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
        _ if extra::format(page).is_some() => extra::parse(page, bytes, now_ms)?,
        _ => statuspage_for(page, bytes)?,
    };
    Ok(Reading {
        page,
        checked_at: now_ms,
        components,
        history_failed: false,
    })
}

fn fetch_url(
    client: &reqwest::blocking::Client,
    address: &str,
    html: bool,
) -> Result<Vec<u8>, ReadError> {
    // Public status pages carry no account headers. Follow a small number of
    // HTTPS moves, keeping the entire redirect chain inside one deadline.
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut address = address.to_string();
    let mut redirects = 0;
    let response = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ReadError::Unreachable);
        }
        let response = client
            .get(&address)
            .timeout(remaining)
            .header(
                "Accept",
                if html {
                    "text/html"
                } else {
                    "application/json"
                },
            )
            .send()
            .map_err(|_| ReadError::Unreachable)?;
        if !response.status().is_redirection() || redirects >= 3 {
            break response;
        }
        let target = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| public_redirect(&address, location))
            .ok_or(ReadError::Http(response.status().as_u16()))?;
        address = target;
        redirects += 1;
    };
    if response.status() != reqwest::StatusCode::OK {
        return Err(ReadError::Http(response.status().as_u16()));
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

fn public_redirect(address: &str, location: &str) -> Option<String> {
    let target = url::Url::parse(address).ok()?.join(location).ok()?;
    (target.scheme() == "https" && target.username().is_empty() && target.password().is_none())
        .then(|| target.into())
}

#[derive(Default)]
struct Cached {
    attempt: Option<Instant>,
    result: Option<Result<Reading, ReadError>>,
    raw: Option<Vec<u8>>,
    history_attempt: Option<Instant>,
    chart_only: bool,
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
    read_cached(page, force, false)
}

/// Also asks for the history the official page draws. A current-only monitor
/// and a pane share the summary; DeepSeek reuses the same already-read HTML.
pub fn read_details(page: Page, force: bool) -> Result<Reading, ReadError> {
    read_cached(page, force, true)
}

fn read_cached(page: Page, force: bool, details: bool) -> Result<Reading, ReadError> {
    static CACHE: OnceLock<[Mutex<Cached>; 45]> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::array::from_fn(|_| Mutex::default()));
    let index = Page::ALL
        .iter()
        .position(|p| *p == page)
        .expect("known page");
    let mut slot = cache[index].lock().unwrap_or_else(|e| e.into_inner());
    let mut fetched = None;
    let result = slot.read_with(force, Instant::now(), || {
        let client = crate::http::HttpClient::new().client_for_login();
        let bytes = fetch_shared(&client, &page.endpoint(), page == Page::DeepSeek, force)?;
        let result = parse(page, &bytes, crate::timeutil::now_ms());
        fetched = Some(bytes);
        result
    });
    if let Some(bytes) = fetched {
        slot.chart_only = page.is_incident()
            && json(&bytes)
                .ok()
                .is_some_and(|root| root["summary"]["display_uptime_mode"] == "chart_only");
        slot.raw = (page == Page::DeepSeek).then_some(bytes);
        slot.history_attempt = None;
    }
    let mut reading = result?;
    if details
        && extra::format(page).is_none_or(|format| format == extra::Format::Incident)
        && (force
            || slot
                .history_attempt
                .is_none_or(|at| at.elapsed() >= CHECK_INTERVAL))
    {
        let history = if page == Page::DeepSeek {
            slot.raw.clone().ok_or(ReadError::Unreadable)
        } else {
            let client = crate::http::HttpClient::new().client_for_login();
            crate::service_status_history::endpoint(&reading)
                .and_then(|url| fetch_url(&client, &url, false))
        };
        reading.history_failed = history
            .and_then(|bytes| crate::service_status_history::apply(&mut reading, &bytes))
            .is_err();
        if slot.chart_only {
            for component in &mut reading.components {
                component.uptime = None;
            }
        }
        slot.history_attempt = Some(Instant::now());
        slot.result = Some(Ok(reading.clone()));
    }
    Ok(reading)
}

// Multiple products on a company's feed share one public summary request.
// Locks are per URL, so a slow company cannot block another company's read.
type SharedRaw = std::sync::Arc<Mutex<Option<(Instant, Result<Vec<u8>, ReadError>)>>>;
fn fetch_shared(
    client: &reqwest::blocking::Client,
    address: &str,
    html: bool,
    force: bool,
) -> Result<Vec<u8>, ReadError> {
    static FEEDS: OnceLock<Mutex<std::collections::HashMap<String, SharedRaw>>> = OnceLock::new();
    let entry = {
        let mut feeds = FEEDS
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        feeds.entry(address.into()).or_default().clone()
    };
    let mut cached = entry.lock().unwrap_or_else(|e| e.into_inner());
    if !force {
        if let Some((at, result)) = &*cached {
            if at.elapsed() < CHECK_INTERVAL {
                return result.clone();
            }
        }
    }
    let result = fetch_url(client, address, html);
    *cached = Some((Instant::now(), result.clone()));
    result
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
    #[test]
    fn public_status_moves_accept_relative_and_https_targets() {
        assert_eq!(
            super::public_redirect(
                "https://status.cursor.com/api/v2/summary.json",
                "https://status.x.ai/api/compat/summary.json"
            )
            .as_deref(),
            Some("https://status.x.ai/api/compat/summary.json")
        );
        assert_eq!(
            super::public_redirect("https://status.example/api/summary", "../current").as_deref(),
            Some("https://status.example/current")
        );
        assert!(
            super::public_redirect("https://status.example", "http://status.example").is_none()
        );
        assert!(super::public_redirect(
            "https://status.example",
            "https://user:secret@status.example"
        )
        .is_none());
    }
    use super::*;
    use serde_json::json;

    fn c(id: &str, state: State) -> Component {
        Component {
            id: id.into(),
            name: id.into(),
            state,
            days: vec![],
            uptime: None,
            group: None,
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
            history_failed: false,
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
            history_failed: false,
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
            [Page::Codex, Page::DeepSeek, Page::Cursor]
                .into_iter()
                .collect()
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

#[path = "service_status_extra.rs"]
mod extra;
