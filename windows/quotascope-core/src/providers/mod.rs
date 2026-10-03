//! One service per product. Each fetches a `ProviderUsage` by whatever route
//! that product actually offers — documented or borrowed — and invents
//! nothing on the way.

pub mod abacus;
pub mod aixy;
pub mod alibaba_coding_plan;
pub mod amp;
pub mod antigravity;
pub mod atlas_cloud;
pub mod augment;
pub mod bifrost;
pub mod chutes;
pub mod claude_code;
pub mod claude_session;
pub mod claw_router;
pub mod cline_pass;
pub mod codebuff;
pub mod codex;
pub mod command_code;
pub mod copilot;
pub mod cursor;
pub mod deepinfra;
pub mod deepseek;
pub mod device_login;
pub mod devpass;
pub mod elevenlabs;
pub mod factory;
pub mod gitkraken;
pub mod grok;
pub mod huggingface;
pub mod hyper;
pub mod ibm_bob;
pub mod kilo_code;
pub mod kimi;
pub mod kiro;
pub mod litellm;
pub mod llm_proxy;
pub mod longcat;
pub mod manus;
pub mod minimax;
pub mod mistral;
pub mod moonshot;
pub mod neuralwatt;
pub mod new_api;
pub mod notion_ai;
pub mod openai_platform;
pub mod opencode;
pub mod perplexity;
pub mod poe;
pub mod qoder;
pub mod qwen_cloud;
pub mod raycast_ai;
pub mod remaining;
pub mod replicate;
pub mod sakana;
pub mod step_fun;
pub mod sub2api;
pub mod synthetic;
pub mod t3chat;
pub mod type_safe;
pub mod v0;
pub mod v2ex;
pub mod venice;
pub mod vercel_ai_gateway;
pub mod volcengine_signer;
pub mod warp;
pub mod xaiapi;
pub mod xiaomi_mimo;
pub mod xkiro;
pub mod zai;
pub mod zed;
pub mod zenmux;
pub mod zoom_mate;

use crate::http::HttpClient;
use crate::model::{AccountKey, Provider, ProviderUsage, Unavailability};
use std::sync::Arc;

/// Where a browser-session credential lives: the site's host (or hosts, for
/// the dual-site products) and the cookie names that authenticate. Settings'
/// import button reads these; the fetch itself only ever sees the stored
/// header, exactly like a pasted key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionSpec {
    pub hosts: &'static [&'static str],
    pub cookies: &'static [&'static str],
}

/// Where each browser-session provider's cookie comes from — the Settings
/// import button reads this. `None` for every provider that keeps a pasted
/// key or no credential at all.
pub fn session_spec(provider: Provider) -> Option<SessionSpec> {
    match provider {
        Provider::Abacus => Some(abacus::SESSION),
        Provider::Augment => Some(augment::SESSION),
        Provider::LongCat => Some(longcat::SESSION),
        Provider::Manus => Some(manus::SESSION),
        Provider::Mistral => Some(mistral::SESSION),
        Provider::NotionAi => Some(notion_ai::SESSION),
        Provider::OllamaCloud => Some(remaining::OLLAMA_SESSION),
        Provider::Perplexity => Some(perplexity::SESSION),
        Provider::Qoder => Some(qoder::SESSION),
        Provider::QwenCloud => Some(qwen_cloud::SESSION),
        Provider::RaycastAi => Some(raycast_ai::SESSION),
        Provider::Replicate => Some(replicate::SESSION),
        Provider::Sakana => Some(sakana::SESSION),
        Provider::StepFun => Some(step_fun::SESSION),
        Provider::T3Chat => Some(t3chat::SESSION),
        Provider::TypeSafe => Some(type_safe::SESSION),
        Provider::XiaomiMiMo => Some(xiaomi_mimo::SESSION),
        Provider::Zed => Some(zed::SESSION),
        Provider::ZoomMate => Some(zoom_mate::SESSION),
        _ => None,
    }
}

/// The credentials a pass may need, resolved once per pass rather than once
/// per request.
#[derive(Default, Clone)]
pub struct KeyRing {
    pub api_keys: std::collections::HashMap<String, String>,
    pub addresses: std::collections::HashMap<String, String>,
    pub copilot_token: Option<String>,
    pub opencode_console: Option<String>,
    /// Account id -> its explicitly stored DeepSeek console session.
    pub deepseek_console: std::collections::HashMap<String, String>,
}

impl KeyRing {
    pub(crate) fn for_account(&self, account: &AccountKey) -> Self {
        if account.is_primary() {
            return self.clone();
        }
        let api_keys = self
            .api_keys
            .get(&account.id())
            .map(|key| {
                std::collections::HashMap::from([(account.provider.raw().to_string(), key.clone())])
            })
            .unwrap_or_default();
        Self {
            api_keys,
            addresses: self.addresses.clone(),
            copilot_token: None,
            opencode_console: None,
            deepseek_console: self
                .deepseek_console
                .get(&account.id())
                .map(|token| std::collections::HashMap::from([(account.id(), token.clone())]))
                .unwrap_or_default(),
        }
    }
    pub fn load() -> KeyRing {
        let mut api_keys = std::collections::HashMap::new();
        for provider in crate::model::ALL_PROVIDERS {
            if provider.uses_api_key() {
                if let Some(key) = crate::secrets::key_for(provider.raw()) {
                    api_keys.insert(provider.raw().to_string(), key);
                }
            }
        }
        for account in crate::settings::with(|s| s.ordered_enabled())
            .into_iter()
            .filter(|a| !a.is_primary())
        {
            if let Some(key) = crate::secrets::key_for(&account.id()) {
                api_keys.insert(account.id(), key);
            }
        }
        if let Some(session) = crate::secrets::key_for(claude_session::SECRET) {
            api_keys.insert(claude_session::SECRET.into(), session);
        }
        let deepseek_console = crate::settings::with(|s| s.ordered_enabled())
            .into_iter()
            .chain(std::iter::once(AccountKey::primary(Provider::DeepSeek)))
            .filter(|a| a.provider == Provider::DeepSeek)
            .filter_map(|a| crate::deepseek_session::token(&a).map(|token| (a.id(), token)))
            .collect();
        KeyRing {
            api_keys,
            addresses: crate::settings::with(|settings| settings.server_addresses.clone()),
            copilot_token: crate::secrets::key_for("copilot"),
            opencode_console: crate::secrets::key_for(crate::opencode_console::SECRET),
            deepseek_console,
        }
    }

    pub fn api_key(&self, provider: Provider) -> Option<String> {
        self.api_keys.get(provider.raw()).cloned()
    }

    pub fn address(&self, provider: Provider) -> Option<String> {
        self.addresses.get(provider.raw()).cloned()
    }
}

/// A deep-read of DeepSeek's settings, passed to its service.
#[derive(Clone, Debug, Default)]
pub struct DeepSeekBasis {
    pub basis: String,
    pub budget: Option<f64>,
    pub currency: Option<String>,
}

pub trait ProviderService: Send + Sync {
    fn provider(&self) -> Provider;

    /// The stable `--json` source token for what this service's route is.
    fn origin_token(&self) -> &'static str;

    fn fetch(&self, keys: &KeyRing) -> ProviderUsage;

    /// One account, not the primary one. Not ported yet — added accounts
    /// need QuotaScope's own OAuth logins.
    fn fetch_account(&self, _keys: &KeyRing, _account: &AccountKey) -> Option<ProviderUsage> {
        None
    }
}

/// A key the user pasted, or the empty string treated as absent — the rule
/// every key service opens with.
pub fn pasted_or_none(key: Option<String>) -> Option<String> {
    key.filter(|k| !k.is_empty())
}

/// What every HTTP-backed service maps a status code to. 401 and 403 mean
/// the credential; the caller refines which credential it was.
pub fn status_reason(status: u16) -> Option<Unavailability> {
    match status {
        401 | 403 => Some(Unavailability::ApiKeyRefused),
        429 => Some(Unavailability::RateLimited),
        s if (500..600).contains(&s) => Some(Unavailability::ServerError),
        _ => Some(Unavailability::ServerError),
    }
}

pub struct Services {
    pub list: Vec<Arc<dyn ProviderService>>,
    pub http: Arc<HttpClient>,
    /// Held concretely because its fetch carries the user's chosen
    /// denominator — the one service whose reading changes meaning with a
    /// setting.
    pub deepseek: Arc<deepseek::DeepSeekService>,
}

impl Services {
    pub fn new() -> Services {
        let http = Arc::new(HttpClient::new());
        let deepseek = Arc::new(deepseek::DeepSeekService::new(http.clone()));
        let mut list: Vec<Arc<dyn ProviderService>> = vec![
            Arc::new(claude_code::ClaudeCodeService::new(http.clone())),
            Arc::new(codex::CodexService::new(http.clone())),
            Arc::new(kiro::KiroService),
            Arc::new(antigravity::AntigravityService::new()),
            Arc::new(cursor::CursorService::new()),
            Arc::new(copilot::CopilotService::new(http.clone())),
            Arc::new(grok::GrokService::new(http.clone())),
            Arc::new(opencode::OpenCodeService::new(http.clone())),
            Arc::new(kimi::KimiService::new(http.clone())),
            Arc::new(zai::ZaiService::new(http.clone())),
            Arc::new(zai::ZaiService::for_mainland(http.clone())),
            Arc::new(minimax::MinimaxService::new(http.clone())),
            Arc::new(minimax::MinimaxService::for_mainland(http.clone())),
            deepseek.clone(),
            Arc::new(command_code::CommandCodeService::new(http.clone())),
            Arc::new(v2ex::V2exService::new(http.clone())),
            Arc::new(moonshot::MoonshotService::new(http.clone())),
            Arc::new(huggingface::HuggingFaceService::new(http.clone())),
            Arc::new(venice::VeniceService::new(http.clone())),
            Arc::new(deepinfra::DeepInfraService::new(http.clone())),
            Arc::new(sub2api::Sub2ApiService::new(http.clone())),
            Arc::new(new_api::NewApiService::new(http.clone())),
            Arc::new(kilo_code::KiloCodeService::new(http.clone())),
            Arc::new(synthetic::SyntheticService::new(http.clone())),
            Arc::new(elevenlabs::ElevenLabsService::new(http.clone())),
            Arc::new(ibm_bob::IbmBobService::new(http.clone())),
            Arc::new(vercel_ai_gateway::VercelAiGatewayService::new(http.clone())),
            Arc::new(amp::AmpService::new(http.clone())),
            Arc::new(codebuff::CodebuffService::new(http.clone())),
            Arc::new(claw_router::ClawRouterService::new(http.clone())),
            Arc::new(devpass::DevPassService::new(http.clone())),
            Arc::new(cline_pass::ClinePassService::new(http.clone())),
            Arc::new(poe::PoeService::new(http.clone())),
            Arc::new(chutes::ChutesService::new(http.clone())),
            Arc::new(v0::V0Service::new(http.clone())),
            Arc::new(aixy::AixyService::new(http.clone())),
            Arc::new(atlas_cloud::AtlasCloudService::new(http.clone())),
            Arc::new(bifrost::BifrostService::new(http.clone())),
            Arc::new(llm_proxy::LlmProxyService::new(http.clone())),
            Arc::new(litellm::LiteLlmService::new(http.clone())),
            Arc::new(factory::FactoryService::new(http.clone())),
            Arc::new(gitkraken::GitKrakenService::new(http.clone())),
            Arc::new(hyper::HyperService::new(http.clone())),
            Arc::new(neuralwatt::NeuralwattService::new(http.clone())),
            Arc::new(zenmux::ZenMuxService::new(http.clone())),
            Arc::new(xaiapi::XaiApiService::new(http.clone())),
            Arc::new(xkiro::XKiroService::new(http.clone())),
            Arc::new(openai_platform::OpenAiPlatformService::new(http.clone())),
            Arc::new(warp::WarpService::new(http.clone())),
            Arc::new(alibaba_coding_plan::AlibabaCodingPlanService::new(
                http.clone(),
            )),
            Arc::new(abacus::AbacusService::new(http.clone())),
            Arc::new(augment::AugmentService::new(http.clone())),
            Arc::new(longcat::LongCatService::new(http.clone())),
            Arc::new(manus::ManusService::new(http.clone())),
            Arc::new(notion_ai::NotionAiService::new(http.clone())),
            Arc::new(zed::ZedService::new(http.clone())),
            Arc::new(perplexity::PerplexityService::new(http.clone())),
            Arc::new(replicate::ReplicateService::new(http.clone())),
            Arc::new(qwen_cloud::QwenCloudService::new(http.clone())),
            Arc::new(mistral::MistralService::new(http.clone())),
            Arc::new(qoder::QoderService::new(http.clone())),
            Arc::new(step_fun::StepFunService::new(http.clone())),
            Arc::new(xiaomi_mimo::XiaomiMiMoService::new(http.clone())),
            Arc::new(raycast_ai::RaycastAiService::new(http.clone())),
            Arc::new(sakana::SakanaService::new(http.clone())),
            Arc::new(t3chat::T3ChatService::new(http.clone())),
            Arc::new(type_safe::TypeSafeService::new(http.clone())),
            Arc::new(zoom_mate::ZoomMateService::new(http.clone())),
        ];
        list.extend(remaining::PROVIDERS.iter().map(|provider| {
            Arc::new(remaining::Service::new(*provider, http.clone())) as Arc<dyn ProviderService>
        }));
        Services {
            list,
            http,
            deepseek,
        }
    }

    pub fn for_provider(&self, provider: Provider) -> Option<&Arc<dyn ProviderService>> {
        self.list.iter().find(|s| s.provider() == provider)
    }
}
