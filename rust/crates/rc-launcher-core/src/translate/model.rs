//! Translation data model (task 13).
//!
//! Everything that crosses the FFI / Kotlin boundary is defined here:
//!
//! * [`TranslationLanguage`] — the supported target / source languages
//!   (zh-CN, zh-Hant, en) plus the `auto` sentinel that lets the gateway
//!   detect the language itself.
//! * [`TranslationMode`] — `Online` (gateway only), `Offline` (dictionary
//!   only) or `Hybrid` (try the dictionary first, then the gateway if the
//!   entry is missing — the recommended default).
//! * [`TranslationGateway`] — the configurable HTTP endpoint used to call
//!   the translation LLM (default: the `kilo.ai` chat-completions gateway
//!   requested in the task brief).
//! * [`TranslationRequest`] / [`TranslationResult`] / [`TranslationSource`] —
//!   the request payload, the resolved result, and the provenance
//!   (`Dictionary`, `Gateway`, `Cache`, `Passthrough`).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// A translation target language. `Auto` lets the gateway detect the
/// language; the built-in dictionary always treats it as `En` (English),
/// because most Modrinth/CurseForge copy is English.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TranslationLanguage {
    /// Let the gateway detect the source language.
    Auto,
    /// Simplified Chinese (`zh-CN`).
    ZhCn,
    /// Traditional Chinese (`zh-Hant`).
    ZhHant,
    /// English (`en`).
    En,
}

impl serde::Serialize for TranslationLanguage {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.tag())
    }
}

impl<'de> serde::Deserialize<'de> for TranslationLanguage {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        TranslationLanguage::from_tag(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!("unknown TranslationLanguage tag: {raw:?}"))
        })
    }
}

impl TranslationLanguage {
    /// The catalogue tag persisted by the UI / settings screen.
    pub fn tag(self) -> &'static str {
        match self {
            TranslationLanguage::Auto => "auto",
            TranslationLanguage::ZhCn => "zh-CN",
            TranslationLanguage::ZhHant => "zh-Hant",
            TranslationLanguage::En => "en",
        }
    }

    /// Inverse of [`TranslationLanguage::tag`]; `None` for an unrecognised
    /// tag so a corrupted preference cannot crash the resolver.
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(TranslationLanguage::Auto),
            "zh" | "zh-cn" | "zh-hans" => Some(TranslationLanguage::ZhCn),
            "zh-hant" | "zh-tw" | "zh-hk" | "zh-mo" => Some(TranslationLanguage::ZhHant),
            "en" | "en-us" | "en-gb" => Some(TranslationLanguage::En),
            _ => None,
        }
    }

    /// All shipped languages, in display order (`Auto` first so the UI can
    /// show "跟随原文" as the default).
    pub const ALL: [TranslationLanguage; 4] = [
        TranslationLanguage::Auto,
        TranslationLanguage::ZhCn,
        TranslationLanguage::ZhHant,
        TranslationLanguage::En,
    ];

    /// Human-readable label (`auto`/`zh-CN`/...).
    pub fn label(self) -> &'static str {
        match self {
            TranslationLanguage::Auto => "跟随原文",
            TranslationLanguage::ZhCn => "简体中文",
            TranslationLanguage::ZhHant => "繁體中文",
            TranslationLanguage::En => "English",
        }
    }

    /// English label for the picker (`auto` / `Simplified Chinese` / …).
    pub fn english_label(self) -> &'static str {
        match self {
            TranslationLanguage::Auto => "Auto",
            TranslationLanguage::ZhCn => "Simplified Chinese",
            TranslationLanguage::ZhHant => "Traditional Chinese",
            TranslationLanguage::En => "English",
        }
    }
}

impl fmt::Display for TranslationLanguage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// Allow "zh-CN".into() in API call sites and doctests (task 13). Unknown
/// tags resolve to [TranslationLanguage::Auto], mirroring the lenient
/// `from_tag` policy used for persisted preferences.
impl From<&str> for TranslationLanguage {
    fn from(tag: &str) -> Self {
        TranslationLanguage::from_tag(tag).unwrap_or(TranslationLanguage::Auto)
    }
}

/// How [`TranslationService::translate`] resolves a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranslationMode {
    /// Use the gateway only (the dictionary is consulted as a *prefix* cache
    /// so well-known entries never hit the network).
    Online,
    /// Use the dictionary only — no network traffic at all.
    Offline,
    /// Dictionary first, then the gateway. **Default** — keeps traffic low
    /// and degrades to a still-useful UI on a bad link.
    Hybrid,
}

impl Default for TranslationMode {
    fn default() -> Self {
        TranslationMode::Hybrid
    }
}

/// Where the translated text came from. Surfaced to the UI so it can show a
/// small badge ("字典"/"在线"/"缓存") next to each row, exactly as the task
/// 13 spec asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationSource {
    /// The input was already in the target language (no work needed).
    Passthrough,
    /// Hit the built-in dictionary (offline).
    Dictionary,
    /// Hit the on-disk cache (offline, but for content the gateway already
    /// translated in a previous session).
    Cache,
    /// Hit the configured gateway.
    Gateway,
    /// All sources failed — UI should display the original text.
    Unavailable,
}

impl TranslationSource {
    /// Short label for the badge.
    pub fn badge(self) -> &'static str {
        match self {
            TranslationSource::Passthrough => "",
            TranslationSource::Dictionary => "字典",
            TranslationSource::Cache => "缓存",
            TranslationSource::Gateway => "在线",
            TranslationSource::Unavailable => "离线",
        }
    }
}

/// The translation gateway configuration. Defaults to the `kilo.ai`
/// chat-completions endpoint from the task brief, but the URL, model,
/// auth header and prompt are all configurable so any OpenAI-compatible
/// endpoint (a self-hosted Llama, a corporate proxy, a community mirror)
/// works out of the box.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationGateway {
    /// Gateway URL (default: `https://api.kilo.ai/api/gateway/chat/completions`).
    pub url: String,
    /// Model name (default: `poolside/laguna-xs-2.1:free`).
    pub model: String,
    /// Optional bearer token (kept in memory only — never written to logs).
    pub auth_bearer: Option<String>,
    /// Optional extra header (for custom auth schemes).
    pub auth_header: Option<String>,
    /// Optional API-key header name (default: `Authorization`).
    pub auth_header_name: String,
    /// System prompt (defaults to a concise translator prompt; the user can
    /// override it to teach the model their own style / glossary).
    pub system_prompt: String,
    /// Per-request token budget (`max_tokens` in the request body).
    pub max_tokens: u32,
    /// Temperature (`temperature` in the request body). `0.0` is the right
    /// default — translations should be deterministic.
    pub temperature: f32,
    /// Optional request timeout (overrides the default in
    /// [`crate::net::NetworkConfig::read_timeout`]).
    pub timeout_secs: Option<u64>,
}

impl Default for TranslationGateway {
    fn default() -> Self {
        Self {
            url: "https://api.kilo.ai/api/gateway/chat/completions".to_string(),
            model: "poolside/laguna-xs-2.1:free".to_string(),
            auth_bearer: None,
            auth_header: None,
            auth_header_name: "Authorization".to_string(),
            system_prompt:
                "你是一个 Minecraft 模组浏览器内联的简明翻译助手。请把用户提供的英文原文翻译成中文（简体），保持专业、准确、简洁，\
                 不要解释、不要复述、不要加引号或前缀；如果原文本身已是中文则原样返回。"
                    .to_string(),
            max_tokens: 512,
            temperature: 0.0,
            timeout_secs: Some(30),
        }
    }
}

/// One translation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationRequest {
    /// The text to translate.
    pub text: String,
    /// Optional source language. `None` ⇒ auto-detect.
    pub source: Option<TranslationLanguage>,
    /// Target language (default: `ZhCn`).
    #[serde(default = "default_target")]
    pub target: TranslationLanguage,
    /// Override the service mode for this one call.
    pub mode: Option<TranslationMode>,
    /// Optional cache TTL in seconds. `None` ⇒ use the service default.
    pub cache_ttl_secs: Option<u64>,
    /// Optional extra HTTP headers (e.g. `X-Trace-Id`).
    #[serde(default)]
    pub extra_headers: BTreeMap<String, String>,
}

fn default_target() -> TranslationLanguage {
    TranslationLanguage::ZhCn
}

impl Default for TranslationRequest {
    fn default() -> Self {
        Self {
            text: String::new(),
            source: None,
            target: default_target(),
            mode: None,
            cache_ttl_secs: None,
            extra_headers: BTreeMap::new(),
        }
    }
}

impl TranslationRequest {
    /// A request that asks for translation to the given target language.
    pub fn to(text: impl Into<String>, target: TranslationLanguage) -> Self {
        Self {
            text: text.into(),
            target,
            ..Self::default()
        }
    }

    /// Stable cache key for this request. Two requests with the same key
    /// produce the same translation, so they share a cache slot.
    pub fn cache_key(&self) -> String {
        // The cache key is text + source + target + mode. We do *not* hash
        // TTL or extra headers: those are routing decisions, not content.
        let src = self.source.map(|l| l.tag()).unwrap_or("auto");
        let mode = self
            .mode
            .map(|m| match m {
                TranslationMode::Online => "online",
                TranslationMode::Offline => "offline",
                TranslationMode::Hybrid => "hybrid",
            })
            .unwrap_or("hybrid");
        format!("tr|{}|{}|{}|{}", src, self.target, mode, self.text)
    }
}

/// One translation result, with provenance metadata so the UI can show a
/// badge and the player can trust the content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationResult {
    /// The original text (echoed back so the UI does not have to keep its
    /// own copy).
    pub original: String,
    /// The translated text. Equals `original` when `source == target`
    /// ([`TranslationSource::Passthrough`]).
    pub translated: String,
    /// The target language that was used.
    pub target: TranslationLanguage,
    /// The source language the gateway detected (only meaningful when
    /// [`TranslationRequest::source`] is `None`).
    pub detected_source: Option<TranslationLanguage>,
    /// Where the translation came from.
    pub source: TranslationSource,
    /// `true` when the result came from a non-network source (dictionary,
    /// cache or passthrough). Used by the UI to skip a "翻译中" spinner.
    pub offline: bool,
}

impl TranslationResult {
    /// Build a "we did nothing" result for a request whose text is already
    /// in the target language.
    pub fn passthrough(req: &TranslationRequest) -> Self {
        Self {
            original: req.text.clone(),
            translated: req.text.clone(),
            target: req.target,
            detected_source: Some(req.target),
            source: TranslationSource::Passthrough,
            offline: true,
        }
    }
}
