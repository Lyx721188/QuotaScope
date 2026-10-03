//! The data model, ported from `UsageProvider.swift` and `ProviderUsage.swift`.
//!
//! The one rule this module exists to keep: **QuotaScope does not invent usage
//! percentages.** Everything a `UsageWindow` carries came from the provider,
//! and the two labelled exceptions are the only places a denominator was
//! inferred — each carries its `Estimate` so the UI can say so.

use serde::{Deserialize, Serialize};

/// The coding agents QuotaScope tracks.
///
/// Sixteen of these are ported to Windows; `ollamaCloud`, `grokBot` and
/// `volcengine` keep their place in the model but are not fetchable here
/// yet — see `is_ported_to_windows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    ClaudeCode,
    Codex,
    Kiro,
    Antigravity,
    Cursor,
    OpenCodeGo,
    KimiCode,
    OllamaCloud,
    Zai,
    #[serde(rename = "glmCoding")]
    GlmCoding,
    Minimax,
    #[serde(rename = "minimaxCN")]
    MinimaxCn,
    Copilot,
    Grok,
    GrokBot,
    Volcengine,
    CommandCode,
    DeepSeek,
    Devin,
    XiaomiMiMo,
    Sub2api,
    #[serde(rename = "newAPI")]
    NewApi,
    V2ex,
    Qoder,
    StepFun,
    ClinePass,
    AlibabaCodingPlan,
    AlibabaTokenPlan,
    QwenCloud,
    Factory,
    Gemini,
    KiloCode,
    Augment,
    #[serde(rename = "jetBrainsAI")]
    JetBrainsAi,
    T3Chat,
    Synthetic,
    ElevenLabs,
    Warp,
    Windsurf,
    Bifrost,
    Chutes,
    LongCat,
    ZoomMate,
    #[serde(rename = "notionAI")]
    NotionAi,
    #[serde(rename = "ibmBob")]
    IbmBob,
    NousPortal,
    #[serde(rename = "raycastAI")]
    RaycastAi,
    GitKraken,
    #[serde(rename = "xKiro")]
    XKiro,
    Abacus,
    Moonshot,
    Hyper,
    AtlasCloud,
    Poe,
    Venice,
    #[serde(rename = "openAIPlatform")]
    OpenAiPlatform,
    Amp,
    Zed,
    Sakana,
    Mistral,
    Codebuff,
    #[serde(rename = "llmProxy")]
    LlmProxy,
    #[serde(rename = "liteLLM")]
    LiteLlm,
    Aixy,
    Neuralwatt,
    ClawRouter,
    ZenMux,
    V0,
    DevPass,
    Perplexity,
    Manus,
    HuggingFace,
    DeepInfra,
    #[serde(rename = "xaiAPI")]
    XaiApi,
    Replicate,
    TypeSafe,
    #[serde(rename = "vercelAIGateway")]
    VercelAiGateway,
    /// Not a product but a kind of account: each extension in the
    /// Extensions folder is one account of this provider. It stays out of
    /// `ALL_PROVIDERS`, which is the built-in list.
    Extension,
}

pub const ALL_PROVIDERS: [Provider; 77] = [
    Provider::ClaudeCode,
    Provider::Codex,
    Provider::Kiro,
    Provider::Antigravity,
    Provider::Cursor,
    Provider::OpenCodeGo,
    Provider::KimiCode,
    Provider::OllamaCloud,
    Provider::Zai,
    Provider::GlmCoding,
    Provider::Minimax,
    Provider::MinimaxCn,
    Provider::Copilot,
    Provider::Grok,
    Provider::GrokBot,
    Provider::Volcengine,
    Provider::CommandCode,
    Provider::DeepSeek,
    Provider::Devin,
    Provider::XiaomiMiMo,
    Provider::Sub2api,
    Provider::NewApi,
    Provider::V2ex,
    Provider::Qoder,
    Provider::StepFun,
    Provider::ClinePass,
    Provider::AlibabaCodingPlan,
    Provider::AlibabaTokenPlan,
    Provider::QwenCloud,
    Provider::Factory,
    Provider::Gemini,
    Provider::KiloCode,
    Provider::Augment,
    Provider::JetBrainsAi,
    Provider::T3Chat,
    Provider::Synthetic,
    Provider::ElevenLabs,
    Provider::Warp,
    Provider::Windsurf,
    Provider::Bifrost,
    Provider::Chutes,
    Provider::LongCat,
    Provider::ZoomMate,
    Provider::NotionAi,
    Provider::IbmBob,
    Provider::NousPortal,
    Provider::RaycastAi,
    Provider::GitKraken,
    Provider::XKiro,
    Provider::Abacus,
    Provider::Moonshot,
    Provider::Hyper,
    Provider::AtlasCloud,
    Provider::Poe,
    Provider::Venice,
    Provider::OpenAiPlatform,
    Provider::Amp,
    Provider::Zed,
    Provider::Sakana,
    Provider::Mistral,
    Provider::Codebuff,
    Provider::LlmProxy,
    Provider::LiteLlm,
    Provider::Aixy,
    Provider::Neuralwatt,
    Provider::ClawRouter,
    Provider::ZenMux,
    Provider::V0,
    Provider::DevPass,
    Provider::Perplexity,
    Provider::Manus,
    Provider::HuggingFace,
    Provider::DeepInfra,
    Provider::XaiApi,
    Provider::Replicate,
    Provider::TypeSafe,
    Provider::VercelAiGateway,
];

impl Provider {
    pub fn raw(&self) -> &'static str {
        crate::localization::provider_raw(*self)
    }

    pub fn from_raw(raw: &str) -> Option<Provider> {
        ALL_PROVIDERS
            .into_iter()
            .find(|p| p.raw() == raw)
            .or_else(|| (raw == "extension").then_some(Provider::Extension))
    }

    /// Product names, left untranslated.
    pub fn display_name(&self) -> &'static str {
        match self {
            Provider::ClaudeCode => "Claude Code",
            Provider::Codex => "Codex",
            Provider::Kiro => "Kiro",
            Provider::Antigravity => "Antigravity",
            Provider::Cursor => "Cursor",
            Provider::OpenCodeGo => "OpenCode Go",
            Provider::KimiCode => "Kimi Code",
            Provider::OllamaCloud => "Ollama Cloud",
            // Two entries rather than one with a region switch: two accounts
            // on two services. Named for the two shops, not for the product.
            Provider::Zai => "z.ai",
            Provider::GlmCoding => "Zhipu",
            Provider::Minimax => "MiniMax",
            Provider::MinimaxCn => "MiniMax CN",
            Provider::Copilot => "GitHub Copilot",
            Provider::Grok => "Grok",
            Provider::GrokBot => "Grok Bot",
            Provider::Volcengine => "Volcengine",
            Provider::CommandCode => "Command Code",
            Provider::DeepSeek => "DeepSeek",
            Provider::Devin => "Devin",
            Provider::XiaomiMiMo => "Xiaomi Coding Plan",
            Provider::Sub2api => "sub2api",
            Provider::NewApi => "New API",
            Provider::V2ex => "V2EX",
            Provider::Qoder => "Qoder",
            Provider::StepFun => "StepFun",
            Provider::ClinePass => "ClinePass",
            Provider::AlibabaCodingPlan => "Alibaba Coding Plan",
            Provider::AlibabaTokenPlan => "Alibaba Token Plan",
            Provider::QwenCloud => "Qwen Cloud",
            Provider::Factory => "Factory",
            Provider::Gemini => "Gemini",
            Provider::KiloCode => "Kilo Code",
            Provider::Augment => "Augment Code",
            Provider::JetBrainsAi => "JetBrains AI",
            Provider::T3Chat => "T3 Chat",
            Provider::Synthetic => "Synthetic",
            Provider::ElevenLabs => "ElevenLabs",
            Provider::Warp => "Warp",
            Provider::Windsurf => "Windsurf",
            Provider::Bifrost => "Bifrost",
            Provider::Chutes => "Chutes",
            Provider::LongCat => "LongCat",
            Provider::ZoomMate => "ZoomMate",
            Provider::NotionAi => "Notion AI",
            Provider::IbmBob => "IBM Bob",
            Provider::NousPortal => "Nous Portal",
            Provider::RaycastAi => "Raycast AI",
            Provider::GitKraken => "GitKraken AI",
            Provider::XKiro => "xKiro",
            Provider::Abacus => "Abacus AI",
            Provider::Moonshot => "Moonshot",
            Provider::Hyper => "Hyper",
            Provider::AtlasCloud => "Atlas Cloud",
            Provider::Poe => "Poe",
            Provider::Venice => "Venice",
            Provider::OpenAiPlatform => "OpenAI API",
            Provider::Amp => "Amp",
            Provider::Zed => "Zed",
            Provider::Sakana => "Sakana AI",
            Provider::Mistral => "Mistral",
            Provider::Codebuff => "Codebuff",
            Provider::LlmProxy => "LLM API Key Proxy",
            Provider::LiteLlm => "LiteLLM",
            Provider::Aixy => "Aixy",
            Provider::Neuralwatt => "Neuralwatt",
            Provider::ClawRouter => "ClawRouter",
            Provider::ZenMux => "ZenMux",
            Provider::V0 => "v0",
            Provider::DevPass => "DevPass",
            Provider::Perplexity => "Perplexity",
            Provider::Manus => "Manus",
            Provider::HuggingFace => "Hugging Face",
            Provider::DeepInfra => "DeepInfra",
            Provider::XaiApi => "xAI API",
            Provider::Replicate => "Replicate",
            Provider::TypeSafe => "TypeSafe",
            Provider::VercelAiGateway => "Vercel AI Gateway",
            // A placeholder: an extension account's real name comes from its
            // manifest, carried in the settings' extension-name map.
            Provider::Extension => "Extension",
        }
    }

    /// A short monogram for the ring's centre disc. The macOS app draws the
    /// product's own mark; the Windows port ships monograms until each mark
    /// is redrawn as vector art. Two letters where one would collide.
    pub fn monogram(&self) -> &'static str {
        match self {
            Provider::ClaudeCode => "Cl",
            Provider::Codex => "Cd",
            Provider::Kiro => "Ki",
            Provider::Antigravity => "A",
            Provider::Cursor => "Cu",
            Provider::OpenCodeGo => "Oc",
            Provider::KimiCode => "K",
            Provider::OllamaCloud => "Ol",
            Provider::Zai => "z",
            Provider::GlmCoding => "Gl",
            Provider::Minimax => "M",
            Provider::MinimaxCn => "M",
            Provider::Copilot => "Cp",
            Provider::Grok => "Gk",
            Provider::GrokBot => "X",
            Provider::Volcengine => "V",
            Provider::CommandCode => "Co",
            Provider::DeepSeek => "D",
            Provider::Devin => "Dv",
            Provider::XiaomiMiMo => "Xm",
            Provider::Sub2api => "S2",
            Provider::NewApi => "NA",
            Provider::V2ex => "V2",
            Provider::Qoder => "Qo",
            Provider::StepFun => "St",
            Provider::ClinePass => "Cl",
            Provider::AlibabaCodingPlan => "AC",
            Provider::AlibabaTokenPlan => "AT",
            Provider::QwenCloud => "Qw",
            Provider::Factory => "Fa",
            Provider::Gemini => "Ge",
            Provider::KiloCode => "Kc",
            Provider::Augment => "Au",
            Provider::JetBrainsAi => "Jb",
            Provider::T3Chat => "T3",
            Provider::Synthetic => "Sy",
            Provider::ElevenLabs => "El",
            Provider::Warp => "Wa",
            Provider::Windsurf => "Wi",
            Provider::Bifrost => "Bi",
            Provider::Chutes => "Ch",
            Provider::LongCat => "Lc",
            Provider::ZoomMate => "Zm",
            Provider::NotionAi => "No",
            Provider::IbmBob => "Ib",
            Provider::NousPortal => "Np",
            Provider::RaycastAi => "Ra",
            Provider::GitKraken => "Gk",
            Provider::XKiro => "Xk",
            Provider::Abacus => "Ab",
            Provider::Moonshot => "Mo",
            Provider::Hyper => "Hy",
            Provider::AtlasCloud => "At",
            Provider::Poe => "Po",
            Provider::Venice => "Ve",
            Provider::OpenAiPlatform => "OA",
            Provider::Amp => "Am",
            Provider::Zed => "Ze",
            Provider::Sakana => "Sa",
            Provider::Mistral => "Mi",
            Provider::Codebuff => "Cb",
            Provider::LlmProxy => "LP",
            Provider::LiteLlm => "LL",
            Provider::Aixy => "Ax",
            Provider::Neuralwatt => "Ne",
            Provider::ClawRouter => "CR",
            Provider::ZenMux => "ZM",
            Provider::V0 => "v0",
            Provider::DevPass => "DP",
            Provider::Perplexity => "Px",
            Provider::Manus => "Ma",
            Provider::HuggingFace => "HF",
            Provider::DeepInfra => "DI",
            Provider::XaiApi => "xA",
            Provider::Replicate => "Re",
            Provider::TypeSafe => "TS",
            Provider::VercelAiGateway => "VA",
            Provider::Extension => "E",
        }
    }

    /// Whether this provider's route has actually been ported to Windows.
    /// The others stay in the model (settings, JSON, docs) but never fetch,
    /// and Settings says so in as many words.
    pub fn is_ported_to_windows(&self) -> bool {
        if crate::providers::remaining::PROVIDERS.contains(self) {
            return true;
        }
        matches!(
            self,
            Provider::ClaudeCode
                | Provider::Codex
                | Provider::Kiro
                | Provider::Antigravity
                | Provider::Cursor
                | Provider::Copilot
                | Provider::Grok
                | Provider::OpenCodeGo
                | Provider::KimiCode
                | Provider::Zai
                | Provider::GlmCoding
                | Provider::Minimax
                | Provider::MinimaxCn
                | Provider::CommandCode
                | Provider::DeepSeek
                | Provider::Sub2api
                | Provider::NewApi
                | Provider::V2ex
                | Provider::Moonshot
                | Provider::HuggingFace
                | Provider::Venice
                | Provider::DeepInfra
                | Provider::Poe
                | Provider::Chutes
                | Provider::V0
                | Provider::Aixy
                | Provider::AtlasCloud
                | Provider::Factory
                | Provider::GitKraken
                | Provider::Hyper
                | Provider::Neuralwatt
                | Provider::ZenMux
                | Provider::Amp
                | Provider::Codebuff
                | Provider::ClawRouter
                | Provider::DevPass
                | Provider::ClinePass
                | Provider::KiloCode
                | Provider::Synthetic
                | Provider::ElevenLabs
                | Provider::IbmBob
                | Provider::VercelAiGateway
                | Provider::XaiApi
                | Provider::XKiro
                | Provider::OpenAiPlatform
                | Provider::Warp
                | Provider::AlibabaCodingPlan
                | Provider::Bifrost
                | Provider::LlmProxy
                | Provider::LiteLlm
                | Provider::Abacus
                | Provider::Augment
                | Provider::LongCat
                | Provider::Manus
                | Provider::Mistral
                | Provider::NotionAi
                | Provider::Perplexity
                | Provider::QwenCloud
                | Provider::RaycastAi
                | Provider::Replicate
                | Provider::Sakana
                | Provider::T3Chat
                | Provider::TypeSafe
                | Provider::Zed
                | Provider::ZoomMate
                | Provider::Qoder
                | Provider::StepFun
                | Provider::XiaomiMiMo
                | Provider::Extension
        )
    }

    /// Why the provider is not available on Windows, when it is not.
    pub fn windows_gap(&self) -> Option<&'static str> {
        if crate::providers::remaining::PROVIDERS.contains(self) {
            return None;
        }
        match self {
            Provider::OllamaCloud => {
                Some("Reads a browser session cookie — browser access has not been ported yet.")
            }
            Provider::GrokBot => {
                Some("Reads the login Cursor saved. Enable Cursor's Grok Bot through Cursor until this route is ported.")
            }
            Provider::Volcengine => {
                Some("Signs Volcengine's usage API with access keys — the signer has not been ported yet.")
            }
            Provider::Devin => Some("Reads Devin's local or API quota — the Windows route has not been ported yet."),
            Provider::Sub2api | Provider::NewApi => Some("Enter the gateway address and API key to enable this account."),
            Provider::Windsurf => Some("Reads browser local storage — that Windows route has not been ported yet."),
            _ if self.is_profiled_unported() => Some("This provider is catalogued, but its Windows usage route has not been ported yet."),
            _ => None,
        }
    }

    fn is_profiled_unported(&self) -> bool {
        matches!(
            self,
            Provider::AlibabaTokenPlan
                | Provider::Gemini
                | Provider::JetBrainsAi
                | Provider::Windsurf
                | Provider::NousPortal
                | Provider::OllamaCloud
        )
    }

    pub fn needs_server_address(&self) -> bool {
        matches!(
            self,
            Provider::Bifrost
                | Provider::LlmProxy
                | Provider::LiteLlm
                | Provider::Sub2api
                | Provider::NewApi
        )
    }

    /// The provider's own page for an account's usage or billing, where one
    /// is known — what the tray's "Open usage page" goes to.
    ///
    /// **Only pages somebody opens.** Several providers' services send a
    /// `Referer` or call an endpoint whose address looks like a page; those
    /// are not listed unless they are also where a person looks at their
    /// usage. A provider missing here gets no menu item rather than a
    /// guessed link.
    pub fn usage_page(&self) -> Option<&'static str> {
        match self {
            Provider::ClaudeCode => Some("https://claude.ai/settings/usage"),
            Provider::Codex => Some("https://chatgpt.com/codex/settings/usage"),
            Provider::Cursor => Some("https://cursor.com/dashboard?tab=usage"),
            Provider::Copilot => Some("https://github.com/settings/copilot"),
            Provider::DeepSeek => Some("https://platform.deepseek.com/usage"),
            Provider::OpenAiPlatform => Some("https://platform.openai.com/usage"),
            Provider::KimiCode => Some("https://www.kimi.com/code/console"),
            Provider::OllamaCloud => Some("https://ollama.com/settings"),
            Provider::XiaomiMiMo => Some("https://platform.xiaomimimo.com"),
            Provider::Replicate => Some("https://replicate.com/account/billing"),
            Provider::QwenCloud => {
                Some("https://home.qwencloud.com/billing/subscription/token-plan-individual")
            }
            Provider::Perplexity => Some("https://www.perplexity.ai/account/usage"),
            Provider::GitKraken => Some("https://gitkraken.dev/account#ai-usage"),
            Provider::Neuralwatt => Some("https://portal.neuralwatt.com/dashboard"),
            Provider::Amp => Some("https://ampcode.com/settings"),
            Provider::TypeSafe => Some("https://console.typesafe.ai/settings/billing"),
            Provider::Mistral => Some("https://admin.mistral.ai/organization/usage"),
            Provider::XaiApi => Some("https://console.x.ai"),
            _ => None,
        }
    }

    /// Whether a spending history can be shown for this provider at all.
    pub fn provides_history(&self) -> bool {
        matches!(
            self,
            Provider::ClaudeCode
                | Provider::Codex
                | Provider::Antigravity
                | Provider::Zai
                | Provider::GlmCoding
                | Provider::OpenCodeGo
        )
    }

    /// Billing classification from upstream, independent of credential type.
    pub fn is_api_billing(&self) -> bool {
        matches!(
            self,
            Provider::DeepSeek
                | Provider::Sub2api
                | Provider::NewApi
                | Provider::Aixy
                | Provider::AtlasCloud
                | Provider::Bifrost
                | Provider::ClawRouter
                | Provider::DeepInfra
                | Provider::Hyper
                | Provider::LlmProxy
                | Provider::LiteLlm
                | Provider::Moonshot
                | Provider::OpenAiPlatform
                | Provider::Perplexity
                | Provider::Poe
                | Provider::Replicate
                | Provider::TypeSafe
                | Provider::Venice
                | Provider::VercelAiGateway
                | Provider::XaiApi
        )
    }

    /// Providers whose spending is invisible to this machine sit on the
    /// adaptive ceiling for ever; prepaid credit is capped shorter instead.
    pub fn reports_spendable_balance(&self) -> bool {
        matches!(
            self,
            Provider::DeepSeek
                | Provider::CommandCode
                | Provider::Moonshot
                | Provider::Venice
                | Provider::DeepInfra
                | Provider::Sub2api
                | Provider::NewApi
                | Provider::AtlasCloud
                | Provider::Neuralwatt
                | Provider::ZenMux
                | Provider::Amp
                | Provider::KiloCode
                | Provider::VercelAiGateway
                | Provider::XaiApi
                | Provider::XKiro
                | Provider::OpenAiPlatform
                | Provider::Replicate
                | Provider::TypeSafe
        )
    }

    pub fn spending_is_watched_locally(&self) -> bool {
        !self.reports_spendable_balance()
    }

    /// Whether this provider keeps a credential of its own in `keys.dat`.
    pub fn keeps_own_credential(&self) -> bool {
        self.uses_api_key() || *self == Provider::Copilot
    }

    /// Providers Settings offers a paste field for. Ollama's would be a
    /// session cookie, and it is not ported, so it is not listed.
    pub fn uses_api_key(&self) -> bool {
        if matches!(self, Provider::OllamaCloud | Provider::Windsurf) {
            return true;
        }
        matches!(
            self,
            Provider::OpenCodeGo
                | Provider::KimiCode
                | Provider::Zai
                | Provider::GlmCoding
                | Provider::Minimax
                | Provider::MinimaxCn
                | Provider::Volcengine
                | Provider::CommandCode
                | Provider::DeepSeek
                | Provider::V2ex
                | Provider::Moonshot
                | Provider::HuggingFace
                | Provider::Venice
                | Provider::DeepInfra
                | Provider::ClinePass
                | Provider::AlibabaCodingPlan
                | Provider::IbmBob
                | Provider::KiloCode
                | Provider::Bifrost
                | Provider::Sub2api
                | Provider::NewApi
                | Provider::Aixy
                | Provider::Amp
                | Provider::AtlasCloud
                | Provider::Chutes
                | Provider::ClawRouter
                | Provider::Codebuff
                | Provider::DevPass
                | Provider::ElevenLabs
                | Provider::Factory
                | Provider::GitKraken
                | Provider::Hyper
                | Provider::LlmProxy
                | Provider::LiteLlm
                | Provider::Neuralwatt
                | Provider::OpenAiPlatform
                | Provider::Poe
                | Provider::Replicate
                | Provider::Synthetic
                | Provider::TypeSafe
                | Provider::V0
                | Provider::VercelAiGateway
                | Provider::Warp
                | Provider::XaiApi
                | Provider::XKiro
                | Provider::ZenMux
        )
    }

    /// Whether the provider can report anything at all without being set up
    /// on this machine — evidence that another tool already ran here.
    pub fn can_report_without_setup(&self) -> bool {
        match self {
            Provider::ClaudeCode => home_path(".claude").exists(),
            Provider::Codex => home_path(".codex").exists(),
            Provider::Grok => home_path(".grok").exists(),
            Provider::OpenCodeGo => opencode_stored_key().is_some(),
            Provider::GlmCoding => glm_stored_key().is_some(),
            Provider::CommandCode => commandcode_stored_key().is_some(),
            // The editor's install is the evidence: its language server only
            // exists inside one.
            Provider::Antigravity => antigravity_install().is_some(),
            // The login database is both the evidence and the credential.
            Provider::Cursor => cursor_database().exists(),
            _ => false,
        }
    }
}

/// Where an Antigravity install keeps its language server, if one is here.
pub fn antigravity_install() -> Option<std::path::PathBuf> {
    let base = std::env::var("LOCALAPPDATA").ok()?;
    let programs = std::path::PathBuf::from(&base)
        .join("Programs")
        .join("Antigravity");
    if programs.exists() {
        return Some(programs);
    }
    let direct = std::path::PathBuf::from(base).join("Antigravity");
    direct.exists().then_some(direct)
}

/// The SQLite database VS Code-derived editors keep global state in — for
/// Cursor, also where its login lives.
pub fn cursor_database() -> std::path::PathBuf {
    let base = crate::data_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    base.join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

pub fn home_path(rel: &str) -> std::path::PathBuf {
    crate::home_dir().join(rel)
}

pub fn opencode_stored_key() -> Option<String> {
    let text = std::fs::read_to_string(home_path(".local/share/opencode/auth.json")).ok()?;
    let root: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = root.get("opencode-go")?.get("key")?.as_str()?;
    Some(key.to_string()).filter(|k| !k.is_empty())
}

pub fn glm_stored_key() -> Option<String> {
    for path in [
        home_path(".coding-relay/glm-api-key"),
        home_path(".config/bigmodel/api_key"),
        home_path(".config/zhipu/api_key"),
    ] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let key = text.lines().next().unwrap_or("").trim().to_string();
            if !key.is_empty() {
                return Some(key);
            }
        }
    }
    None
}

pub fn commandcode_stored_key() -> Option<String> {
    let text = std::fs::read_to_string(home_path(".commandcode/auth.json")).ok()?;
    let root: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = root.get("apiKey")?.as_str()?;
    Some(key.to_string()).filter(|k| !k.is_empty())
}

/// One account of one provider. A provider's first account id is the
/// provider's own raw value; added accounts carry a `#slot`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountKey {
    pub provider: Provider,
    #[serde(default)]
    pub slot: String,
}

impl AccountKey {
    pub fn primary(provider: Provider) -> Self {
        AccountKey {
            provider,
            slot: String::new(),
        }
    }

    pub fn id(&self) -> String {
        if self.slot.is_empty() {
            self.provider.raw().to_string()
        } else {
            format!("{}#{}", self.provider.raw(), self.slot)
        }
    }

    pub fn is_primary(&self) -> bool {
        self.slot.is_empty()
    }

    pub fn from_id(id: &str) -> Option<AccountKey> {
        let (head, slot) = match id.split_once('#') {
            Some((h, s)) => (h, s),
            None => (id, ""),
        };
        Some(AccountKey {
            provider: Provider::from_raw(head)?,
            slot: slot.to_string(),
        })
    }
}

/// What kind of window a limit is, kept as meaning rather than as text.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    FiveHour,
    Weekly,
    Spend,
    /// Prepaid credit, which is **not a limit**: no ceiling, no window.
    Balance,
    Daily,
    Messages,
    Monthly,
    TopUp,
    Credits,
    SharedCredits,
    #[serde(rename = "other")]
    Other(#[serde(default)] i64),
}

impl Kind {
    /// The flat token `--json` promises: a script matches on this, so it is
    /// the same in every language.
    pub fn token(&self) -> String {
        match self {
            Kind::FiveHour => "fiveHour".into(),
            Kind::Weekly => "weekly".into(),
            Kind::Spend => "spend".into(),
            Kind::Balance => "balance".into(),
            Kind::Daily => "daily".into(),
            Kind::Messages => "messages".into(),
            Kind::Monthly => "monthly".into(),
            Kind::TopUp => "topUp".into(),
            Kind::Credits => "credits".into(),
            Kind::SharedCredits => "sharedCredits".into(),
            Kind::Other(s) => format!("other:{s}"),
        }
    }

    pub fn localized_name(&self, seconds: i64) -> String {
        let key = match self {
            Kind::FiveHour => "5-hour limit",
            Kind::Weekly => "Weekly limit",
            Kind::Spend => "Spend limit",
            Kind::Balance => "Balance",
            Kind::Daily => "Daily limit",
            Kind::Messages => "Messages",
            Kind::Monthly => "Monthly limit",
            Kind::TopUp => "Top-up allowance",
            Kind::Credits => "Credits",
            Kind::SharedCredits => "Shared credits",
            Kind::Other(_) => {
                return if seconds >= 86_400 {
                    crate::localization::t_fmt(
                        "{n}-day limit",
                        &[(&(seconds / 86_400).to_string())],
                    )
                } else {
                    crate::localization::t_fmt(
                        "{n}-hour limit",
                        &[(&((seconds + 3599) / 3600).to_string())],
                    )
                };
            }
        };
        crate::localization::t(key).to_string()
    }
}

/// Where a window's denominator came from, when it did not come from the
/// provider. The one labelled inference in the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Estimate {
    /// Command Code: the monthly grant, from its published plan price.
    PlanPrice,
    /// DeepSeek: the highest balance QuotaScope has watched.
    SinceTopUp,
    /// DeepSeek: a figure the reader typed.
    YourBudget,
}

impl Estimate {
    pub fn token(&self) -> &'static str {
        match self {
            Estimate::PlanPrice => "planPrice",
            Estimate::SinceTopUp => "sinceTopUp",
            Estimate::YourBudget => "yourBudget",
        }
    }
}

/// One rate-limit window exactly as a provider reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    pub id: String,
    pub kind: Kind,
    /// The model this limit is scoped to, when the provider scopes it to one.
    /// A product name, so it is never translated.
    #[serde(default)]
    pub scope: Option<String>,
    /// 0...1 under normal conditions, but a provider may report over 100%
    /// once a limit is exceeded.
    pub used_fraction: f64,
    #[serde(default)]
    pub window_seconds: i64,
    /// Epoch milliseconds.
    #[serde(default)]
    pub resets_at: Option<i64>,
    /// Whether `window_seconds` is a length the provider actually **stated**,
    /// or one chosen so the row sorts. They are not the same thing, and only
    /// one of them can be divided by.
    #[serde(default = "yes")]
    pub reports_length: bool,
    #[serde(default)]
    pub estimate: Option<Estimate>,
    /// Whether the provider says this limit is spent — its judgement, not
    /// `used_fraction >= 1`. A spend limit can run past 100%.
    #[serde(default)]
    pub is_exhausted: bool,
    /// When a bought pack on this limit lapses — Grok's and Qoder's bonus
    /// credits arrive with an expiry, and a balance that will shrink is
    /// worth knowing about. Epoch milliseconds.
    #[serde(default)]
    pub next_expiry_ms: Option<i64>,
    /// Amount expiring at next_expiry_ms, in the provider's allowance unit.
    /// Absent in legacy caches and where the provider states only a date.
    #[serde(default)]
    pub next_expiry_amount: Option<f64>,
    /// The row's own name, when the source gives one — an extension
    /// programme names its limits in its own words. Shown instead of the
    /// kind's name; never translated.
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AllowanceExpiry {
    pub at: i64,
    pub amount: f64,
}

impl AllowanceExpiry {
    /// Sum all remaining parts expiring on the first local calendar day,
    /// rather than understating a day containing several expiring packs.
    pub fn soonest(parts: impl Iterator<Item = (f64, i64)>, now_ms: i64) -> Option<Self> {
        let ahead: Vec<_> = parts
            .filter(|(amount, at)| amount.is_finite() && *amount > 0.0 && *at > now_ms)
            .collect();
        let at = ahead.iter().map(|(_, at)| *at).min()?;
        let day = chrono::DateTime::from_timestamp_millis(at)?
            .with_timezone(&chrono::Local)
            .date_naive();
        let amount = ahead
            .iter()
            .filter(|(_, time)| {
                chrono::DateTime::from_timestamp_millis(*time)
                    .is_some_and(|time| time.with_timezone(&chrono::Local).date_naive() == day)
            })
            .map(|(amount, _)| amount)
            .sum();
        Some(Self { at, amount })
    }
}

fn yes() -> bool {
    true
}

impl UsageWindow {
    pub fn new(
        id: &str,
        kind: Kind,
        scope: Option<String>,
        used_fraction: f64,
        window_seconds: i64,
        resets_at: Option<i64>,
    ) -> Self {
        UsageWindow {
            id: id.to_string(),
            kind,
            scope,
            used_fraction,
            window_seconds,
            resets_at,
            reports_length: true,
            estimate: None,
            is_exhausted: false,
            next_expiry_ms: None,
            next_expiry_amount: None,
            label: None,
        }
    }

    pub fn is_estimated(&self) -> bool {
        self.estimate.is_some()
    }

    pub fn set_expiring_parts(&mut self, parts: impl Iterator<Item = (f64, i64)>, now_ms: i64) {
        let expiry = AllowanceExpiry::soonest(parts, now_ms);
        self.next_expiry_ms = expiry.map(|e| e.at);
        self.next_expiry_amount = expiry.map(|e| e.amount);
    }

    /// Buying more credit is not a reset. Credit pools only turn over when
    /// the provider moves the reset time; a large fall can identify other
    /// windows turning over, but never a continuously topped-up balance.
    pub fn has_turned_over(&self, previous: &UsageWindow) -> bool {
        if matches!(self.kind, Kind::Balance | Kind::TopUp) {
            return false;
        }
        let moved_on = self
            .resets_at
            .zip(previous.resets_at)
            .is_some_and(|(new, old)| new.saturating_sub(old) > 60_000);
        if matches!(self.kind, Kind::Credits | Kind::SharedCredits) {
            moved_on
        } else {
            moved_on || previous.used_fraction - self.used_fraction >= 0.4
        }
    }

    /// How much of this window has gone by, 0...1 — nil unless the provider
    /// gave a reset time **and** stated a length. A sort key is not a length,
    /// and dividing by one draws a fraction the provider never gave.
    pub fn elapsed_fraction(&self, now_ms: i64) -> Option<f64> {
        if !self.reports_length {
            return None;
        }
        let resets = self.resets_at?;
        if self.window_seconds <= 0 {
            return None;
        }
        let remaining_ms = resets - now_ms;
        let window_ms = self.window_seconds as f64 * 1000.0;
        Some((1.0 - remaining_ms as f64 / window_ms).clamp(0.0, 1.0))
    }

    /// The clock arc's fraction in the direction the reader chose: how much
    /// of the window has gone by, or how much is still to come. Both need
    /// what `elapsed_fraction` needs — a stated length and a reset time.
    pub fn window_clock_fraction(&self, remaining: bool, now_ms: i64) -> Option<f64> {
        let elapsed = self.elapsed_fraction(now_ms)?;
        Some(if remaining { 1.0 - elapsed } else { elapsed })
    }

    /// The row's full name: its own label when the source gave one, else
    /// the kind, the scope, and which inference applies.
    pub fn display_name(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        let base = self.kind.localized_name(self.window_seconds);
        let scoped = match &self.scope {
            Some(s) => format!("{base} · {s}"),
            None => base,
        };
        match self.estimate {
            Some(e) => {
                let title = match e {
                    Estimate::PlanPrice => crate::localization::t("estimated"),
                    Estimate::SinceTopUp => crate::localization::t("since top-up"),
                    Estimate::YourBudget => crate::localization::t("of your budget"),
                };
                format!("{scoped} · {title}")
            }
            None => scoped,
        }
    }

    /// A fraction as a whole percentage that never rounds away the fact that
    /// there is *some*, or that there is *not all*. NaN first, because clamp
    /// propagates it rather than catching it.
    fn figure(fraction: f64) -> i64 {
        if !fraction.is_finite() {
            return 0;
        }
        let percent = fraction.clamp(0.0, 1.0) * 100.0;
        if percent <= 0.0 {
            return 0;
        }
        if percent >= 100.0 {
            return 100;
        }
        (percent.round() as i64).clamp(1, 99)
    }

    pub fn percent_value(&self, remaining: bool) -> i64 {
        if remaining {
            Self::figure(self.remaining_fraction())
        } else {
            Self::figure(self.used_fraction)
        }
    }

    pub fn percent_text(&self, remaining: bool) -> String {
        format!("{}%", self.percent_value(remaining))
    }

    pub fn remaining_fraction(&self) -> f64 {
        (1.0 - self.used_fraction).clamp(0.0, 1.0)
    }

    /// "5h" / "7d" style description of the window's length.
    pub fn length_text(&self) -> String {
        let hours = self.window_seconds as f64 / 3600.0;
        if hours >= 24.0 {
            crate::localization::t_fmt(
                "{n} days",
                &[(&((hours / 24.0).round() as i64).to_string())],
            )
        } else {
            crate::localization::t_fmt("{n} hours", &[(&(hours.round() as i64).to_string())])
        }
    }
}

/// Money the provider says is left, and what it is denominated in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreditAmount {
    pub amount: f64,
    /// An ISO code, as the provider gave it.
    pub currency: String,
}

impl CreditAmount {
    /// Short enough to read inside a ring, **truncated, never rounded** — a
    /// balance shown as more than it is is the wrong way to be wrong.
    pub fn rail_text(&self) -> String {
        let symbol = currency_symbol(&self.currency);
        let magnitude = self.amount.abs();
        let (value, suffix) = if magnitude >= 1_000_000.0 {
            (self.amount / 1_000_000.0, "M")
        } else if magnitude >= 1_000.0 {
            (self.amount / 1_000.0, "k")
        } else {
            (self.amount, "")
        };
        let places: i32 = if value.abs() >= 100.0 {
            0
        } else if suffix.is_empty() {
            2
        } else {
            1
        };
        let scale = 10f64.powi(places);
        // Truncation toward zero, not rounding.
        let shown = (value * scale).trunc() / scale;
        format!("{symbol}{shown:.places$}{suffix}", places = places as usize)
    }
}

fn currency_symbol(code: &str) -> String {
    match code {
        "CNY" | "RMB" => "¥".into(),
        "USD" => "$".into(),
        "EUR" => "€".into(),
        "GBP" => "£".into(),
        "JPY" => "¥".into(),
        other => format!("{other} "),
    }
}

/// Why there is nothing to show. Kept as a case rather than a finished
/// sentence: the text is produced when it is displayed, so it follows the
/// language setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unavailability {
    Loading,
    NotConnected,
    AwaitingResponse,
    NoLimitsReported,
    NoPlan,
    SignInRequired,
    ClaudeSignInRequired,
    ClaudeLoginExpired,
    CodexNotInstalled,
    CodexServerFailed,
    KiroNotInstalled,
    KiroSignInRequired,
    KiroVersionUnsupported,
    GrokSignInRequired,
    GrokLoginExpired,
    SignedOut,
    NotSignedIn,
    ApiKeyMissing,
    ApiKeyRefused,
    ServerAddressMissing,
    ServerAddressRefused,
    Unreachable,
    UnreadableReply,
    RateLimited,
    ServerError,
    /// Antigravity's limits live in a server it only runs while it is open.
    AntigravityNotRunning,
    /// Antigravity **is** open, and every helper it runs refused this RPC —
    /// a version bump, or the app still starting.
    AntigravityNotAnswering,
    /// Cursor has never been signed in on this PC, so there is no login to
    /// borrow.
    CursorSignInRequired,
    /// There is a Cursor login, and the account refused it.
    CursorLoginExpired,
    /// Not ported to Windows yet — named once, in Settings only.
    NotOnWindows,
    ZaiNoCodingPlan,
    /// An extension's program is not where its manifest said, or will not
    /// start.
    ExtensionMissing,
    /// An extension ran past the timeout its manifest asked for.
    ExtensionTimedOut,
    /// An extension exited without printing a usable report.
    ExtensionFailed,
    /// An extension's own word for "the account this program reads is
    /// signed out".
    ExtensionSignedOut,
    /// No browser session has been imported for this account yet.
    SessionMissing,
    /// The imported browser session no longer answers — the site's own
    /// refusal of a credential that used to work.
    SessionExpired,
    LocalLoginMissing,
    LocalLoginExpired,
    LocalAppMissing,
}

impl Unavailability {
    pub fn message(&self) -> &'static str {
        crate::localization::t(match self {
            Unavailability::LocalLoginMissing => "Sign in to the provider's CLI first.",
            Unavailability::LocalLoginExpired => {
                "The local login expired. Sign in to the provider's CLI again."
            }
            Unavailability::LocalAppMissing => "No saved quota found in the provider's local app.",
            Unavailability::Loading => "Loading…",
            Unavailability::NotConnected => "notConnected",
            Unavailability::AwaitingResponse => "awaitingResponse",
            Unavailability::NoLimitsReported => "No limits reported.",
            Unavailability::NoPlan => "This account has no plan with usage limits.",
            Unavailability::SignInRequired => "signInRequired",
            Unavailability::ClaudeSignInRequired => "claudeSignInRequired",
            Unavailability::ClaudeLoginExpired => "claudeLoginExpired",
            Unavailability::CodexNotInstalled => "Codex isn't installed.",
            Unavailability::CodexServerFailed => "codexServerFailed",
            Unavailability::KiroNotInstalled => "Kiro CLI isn't installed.",
            Unavailability::KiroSignInRequired => "Sign in to Kiro CLI to see usage.",
            Unavailability::KiroVersionUnsupported => "Update Kiro CLI to read subscription usage.",
            Unavailability::GrokSignInRequired => "grokSignInRequired",
            Unavailability::GrokLoginExpired => "grokLoginExpired",
            Unavailability::SignedOut => "signedOut",
            Unavailability::NotSignedIn => "notSignedIn",
            Unavailability::ApiKeyMissing => "Add an API key in Settings.",
            Unavailability::ApiKeyRefused => "That key was refused. Check it in Settings.",
            Unavailability::SessionMissing => "Import a browser session in Settings.",
            Unavailability::SessionExpired => {
                "The browser session has expired — import it again in Settings."
            }
            Unavailability::ServerAddressMissing => "Add a gateway address in Settings.",
            Unavailability::ServerAddressRefused => "That gateway address is not allowed.",
            Unavailability::Unreachable => "The service didn't respond.",
            Unavailability::UnreadableReply => "Couldn't read the reply.",
            Unavailability::RateLimited => "Checking too often — easing off.",
            Unavailability::ServerError => "The service returned an error.",
            Unavailability::AntigravityNotRunning => "Open Antigravity to see its usage.",
            Unavailability::AntigravityNotAnswering => {
                "Antigravity is open but didn't answer. Restarting it usually helps."
            }
            Unavailability::CursorSignInRequired => "Sign in to Cursor to see usage.",
            Unavailability::CursorLoginExpired => {
                "Cursor's saved login was refused. Open Cursor to renew it."
            }
            Unavailability::NotOnWindows => "notOnWindows",
            Unavailability::ZaiNoCodingPlan => "zaiNoCodingPlan",
            Unavailability::ExtensionMissing => "extensionMissing",
            Unavailability::ExtensionTimedOut => "extensionTimedOut",
            Unavailability::ExtensionFailed => "extensionFailed",
            Unavailability::ExtensionSignedOut => "extensionSignedOut",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "camelCase")]
pub enum State {
    Live,
    Stale,
    Unavailable(Unavailability),
}

/// Everything QuotaScope currently knows about one provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub account: AccountKey,
    /// Ordered as the provider reports them; the first one drives the ring.
    pub windows: Vec<UsageWindow>,
    /// Epoch milliseconds.
    #[serde(default)]
    pub observed_at: Option<i64>,
    pub state: State,
    #[serde(default)]
    pub plan: Option<String>,
    /// Remaining credit, when the provider reports it, formatted for display.
    #[serde(default)]
    pub credit_balance: Option<String>,
    /// The same figure as a number, where there is one to compare. Separate
    /// from `credit_balance` on purpose: that is a display string and is
    /// sometimes prose — Codex says "Unlimited".
    #[serde(default)]
    pub credit_remaining: Option<CreditAmount>,
    /// Where this reading came from — a stable token, the `--json` `source`.
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub is_cached: bool,
}

impl ProviderUsage {
    pub fn unavailable(account: AccountKey, reason: Unavailability) -> ProviderUsage {
        ProviderUsage {
            account,
            windows: Vec::new(),
            observed_at: None,
            state: State::Unavailable(reason),
            plan: None,
            credit_balance: None,
            credit_remaining: None,
            origin: None,
            is_cached: false,
        }
    }

    pub fn live_now(account: AccountKey, windows: Vec<UsageWindow>) -> ProviderUsage {
        ProviderUsage {
            account,
            windows,
            observed_at: Some(crate::timeutil::now_ms()),
            state: State::Live,
            plan: None,
            credit_balance: None,
            credit_remaining: None,
            origin: None,
            is_cached: false,
        }
    }

    pub fn reports_something(&self) -> bool {
        !self.windows.is_empty() || self.credit_balance.is_some()
    }

    /// The window the rail's ring shows: the pinned one when it still matches,
    /// else the one closest to being used up.
    pub fn headline_window(&self, pinned: Option<&str>) -> Option<&UsageWindow> {
        if let Some(id) = pinned {
            if let Some(w) = self.windows.iter().find(|w| w.id == id) {
                return Some(w);
            }
        }
        self.windows.iter().max_by(|a, b| {
            a.used_fraction
                .partial_cmp(&b.used_fraction)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// The limit the second ring shows: **the fullest of the ring's own model
    /// group**, or of everything else where the group has nothing more.
    pub fn second_window(&self, pinned: Option<&str>) -> Option<&UsageWindow> {
        let headline = self.headline_window(pinned)?;
        if self.windows.len() < 2 {
            return None;
        }
        let rest: Vec<&UsageWindow> = self
            .windows
            .iter()
            .filter(|w| w.id != headline.id)
            .collect();
        let same_group: Vec<&UsageWindow> = rest
            .iter()
            .copied()
            .filter(|w| w.scope == headline.scope)
            .collect();
        let pool = if same_group.is_empty() {
            rest
        } else {
            same_group
        };
        pool.into_iter().max_by(|a, b| {
            a.used_fraction
                .partial_cmp(&b.used_fraction)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    pub fn provider(&self) -> Provider {
        self.account.provider
    }
}

/// Colour means **usage**, not identity. These are the panel's four hues,
/// picked to read at ring size on the dark surface.
pub mod usage_tint {
    use crate::model::UsageWindow;
    pub const CAUTION_THRESHOLD: f64 = 0.5;
    pub const WARNING_THRESHOLD: f64 = 0.75;

    /// (r, g, b) 0..1
    pub const GOOD: [f32; 3] = [0.00, 0.90, 0.55];
    pub const CAUTION: [f32; 3] = [1.00, 0.76, 0.15];
    pub const WARNING: [f32; 3] = [1.00, 0.31, 0.26];
    /// Deeper and flatter than the warning red, so a spent limit doesn't just
    /// look like a slightly redder nearly-spent one.
    pub const EXHAUSTED: [f32; 3] = [0.85, 0.09, 0.13];
    /// The colour the sliver takes past the alert threshold.
    pub const ALERT_GLOW: [f32; 3] = WARNING;

    pub fn color(used_fraction: f64, is_exhausted: bool) -> [f32; 3] {
        color_with(used_fraction, is_exhausted, WARNING_THRESHOLD)
    }

    /// The same scale with the warning line where the reader put it — the
    /// port of the macOS app's "turn red at" choice.
    pub fn color_with(used_fraction: f64, is_exhausted: bool, warning_at: f64) -> [f32; 3] {
        if is_exhausted || used_fraction >= 1.0 {
            return EXHAUSTED;
        }
        if used_fraction < CAUTION_THRESHOLD {
            GOOD
        } else if used_fraction < warning_at {
            CAUTION
        } else {
            WARNING
        }
    }

    /// A settings percentage (60..90) as the fraction the tints compare
    /// against, clamped to the values the picker offers.
    pub fn warning_fraction(threshold_percent: i64) -> f64 {
        (threshold_percent.clamp(50, 95) as f64) / 100.0
    }

    pub fn is_spent(window: Option<&UsageWindow>) -> bool {
        match window {
            None => false,
            Some(w) => w.is_exhausted || w.used_fraction >= 1.0,
        }
    }
}

/// The burn-rate forecast. Off by default; a **projection**, and the copy
/// says so.
pub mod burn_rate {
    use super::UsageWindow;

    /// Nothing is said about a window barely open: early on, dividing by the
    /// elapsed share turns a single burst into a rate that would empty the
    /// account before lunch.
    pub const MINIMUM_ELAPSED: f64 = 0.03;
    /// A prediction further out than this stops being actionable long before
    /// it stops being computable.
    pub const HORIZON_MS: i64 = 2 * 3600 * 1000;

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Reading {
        pub exhausts_before_reset: bool,
        /// Only when that is before the reset **and** inside the horizon.
        pub time_to_exhaustion_ms: Option<i64>,
    }

    pub fn reading(window: &UsageWindow, now_ms: i64) -> Option<Reading> {
        let elapsed = window.elapsed_fraction(now_ms)?;
        if elapsed < MINIMUM_ELAPSED {
            return None;
        }
        let resets = window.resets_at?;
        let until_reset = resets - now_ms;
        if until_reset <= 0 {
            return None;
        }

        let used = window.used_fraction.clamp(0.0, 1.0);
        // Reset timestamps are milliseconds; keep the rate and projection
        // in the same unit instead of treating milliseconds as seconds.
        let elapsed_ms = window.window_seconds as f64 * 1000.0 * elapsed;
        if elapsed_ms <= 0.0 {
            return None;
        }

        let per_ms = used / elapsed_ms;
        if !(used < 1.0) || per_ms <= 0.0 {
            return Some(Reading {
                exhausts_before_reset: used >= 1.0,
                time_to_exhaustion_ms: if used >= 1.0 { Some(0) } else { None },
            });
        }

        let until_empty_ms = ((1.0 - used) / per_ms) as i64;
        // Equivalent to comparing the projected exhaustion with reset,
        // without truncating an exact-at-reset forecast a millisecond early.
        let first = used > elapsed;

        Some(Reading {
            exhausts_before_reset: first,
            time_to_exhaustion_ms: if first && until_empty_ms <= HORIZON_MS {
                Some(until_empty_ms)
            } else {
                None
            },
        })
    }

    /// "about an hour", "about 40 minutes" — **rounded, because the precision
    /// is not real.**
    pub fn approximate(ms: i64) -> String {
        let minutes = (ms / 60_000) as f64;
        let minutes = minutes.round() as i64;
        if minutes < 15 {
            return crate::localization::t("under 15 minutes").to_string();
        }
        if minutes < 68 {
            let quarters = ((minutes as f64 / 15.0).round() as i64).max(1);
            return if quarters == 4 {
                crate::localization::t("about an hour").to_string()
            } else {
                crate::localization::t_fmt("about {n} minutes", &[(&(quarters * 15).to_string())])
            };
        }
        let hours = minutes as f64 / 60.0;
        let halves = ((hours * 2.0).round() as i64).max(2);
        if halves == 2 {
            crate::localization::t("about an hour").to_string()
        } else {
            crate::localization::t_fmt("about {n} hours", &[(&(halves as f64 / 2.0).to_string())])
        }
    }
}
