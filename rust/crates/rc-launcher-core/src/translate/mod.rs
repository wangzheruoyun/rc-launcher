//! Inline auto-translation for the mod browser (task 13).
//!
//! When a player opens the **mod browser**, the catalog they see is almost
//! always in English (Modrinth/CurseForge descriptions, project names,
//! changelogs). On a Chinese-first launcher that is a poor experience: the
//! player has to context-switch to a translation app for every interesting
//! mod.
//!
//! [`TranslationService`] solves that with one synchronous, retryable,
//! cache-aware translation pipeline:
//!
//! * **Online translation** through a configurable gateway — by default the
//!   `kilo.ai` chat-completions gateway requested in the task brief — but
//!   the URL, model, prompt and auth header are all pluggable so any other
//!   provider (a self-hosted LLM, a community proxy, an open OpenAI-compatible
//!   endpoint) works out of the box.
//! * **Network optimisation (task 3)** — the same [`crate::net::NetworkClient`]
//!   the catalog / version-manifest downloads use, so the translation
//!   endpoint inherits mirror fallback, DoH, SOCKS5/HTTP proxy, retries and
//!   connection reuse. A flaky mainland link does not break translation.
//! * **Persistent cache** ([`cache::TranslationCache`]) — every successful
//!   translation is hashed into a `text + target_lang` key and stored on
//!   disk with an optional TTL, so a player only pays one network round-trip
//!   per (sentence, target language). The cache also makes
//!   offline / air-plane-mode browsing keep showing the *last* translations
//!   the user has seen (task 13's "缓存以避免重复请求与流量浪费" requirement).
//! * **Built-in dictionary** ([`dictionary::BuiltInDictionary`]) — a curated
//!   `slug → 中文名` table for the most common mods/loaders (`sodium`,
//!   `iris`, `fabric-api`, `jei`, …) plus common technical terms. This is the
//!   **offline fallback**: if the network is gone, the dictionary still
//!   translates every well-known entry, so the launcher stays useful on the
//!   offline-first mainland.
//! * **Show original / show translated** toggle and **target language**
//!   selector — exposed as JSON in [`service::TranslationService::translate`]
//!   and [`service::TranslationService::translate_batch`] so the UI never
//!   has to know about the pipeline.
//!
//! All types are `serde` (de)serialisable, so the FFI bridge in
//! [`crate::ffi`] hands the whole [`service::TranslationService`] to Kotlin
//! as plain JSON.
//!
//! ```no_run
//! use rc_launcher::net::NetworkClient;
//! use rc_launcher::translate::{TranslationService, TranslationRequest};
//!
//! # async fn run() -> rc_launcher::error::RcResult<()> {
//! let net = NetworkClient::builder().build().await?;
//! let svc = TranslationService::new(net);
//! let out = svc.translate(&TranslationRequest {
//!     text: "A lightweight Minecraft mod that improves FPS.".into(),
//!     source: None,
//!     target: "zh-CN".into(),
//!     cache_ttl_secs: Some(86_400 * 30),
//!     ..Default::default()
//! }).await?;
//! assert!(!out.translated.is_empty());
//! # Ok(()) }
//! ```

pub mod cache;
pub mod dictionary;
pub mod model;
pub mod service;

pub use cache::TranslationCache;
pub use dictionary::{BuiltInDictionary, DictionaryHit};
pub use model::{
    TranslationGateway, TranslationLanguage, TranslationMode, TranslationRequest,
    TranslationResult, TranslationSource,
};
pub use service::{TranslationService, TranslationServiceBuilder};
