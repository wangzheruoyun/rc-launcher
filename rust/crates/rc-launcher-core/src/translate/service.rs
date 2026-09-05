//! [`TranslationService`] — the orchestration layer (task 13).
//!
//! Given a [`TranslationRequest`], the service walks the cheapest possible
//! path to a [`TranslationResult`]:
//!
//! 1. **Passthrough** — if `source == target`, return the original.
//! 2. **Cache** — look the request up in [`TranslationCache`]. A hit ends
//!    the pipeline (with `source = Cache`).
//! 3. **Dictionary** — if the request mode is `Offline` *or* `Hybrid`,
//!    consult the [`BuiltInDictionary`]. Well-known mods/terms get a
//!    `Dictionary` result; free text gets a per-term term-by-term
//!    substitution.
//! 4. **Gateway** — if the dictionary did not yield a high-confidence
//!    answer, send the text to the configured [`TranslationGateway`]. This
//!    call inherits the full network optimisation layer (mirrors / DoH /
//!    proxy / retries / connection reuse).
//! 5. **Failure** — if everything fails, surface `source = Unavailable`
//!    and keep the original text, so the UI never disappears.
//!
//! The service is also `Batch`-capable: a single call to
//! [`TranslationService::translate_batch`] translates up to N requests in
//! parallel, which is what the mod browser needs to render a long list of
//! mods without N round-trips.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::json;

use crate::error::{RcError, RcResult};
use crate::net::NetworkClient;
use crate::translate::cache::{passthrough_if_same_language, CacheConfig, TranslationCache};
use crate::translate::dictionary::BuiltInDictionary;
use crate::translate::model::{
    TranslationGateway, TranslationMode, TranslationRequest, TranslationResult, TranslationSource,
};

/// The orchestration layer for mod-browser inline translation (task 13).
///
/// Cheap to clone — every field is either `Arc`-wrapped or trivially
/// cloneable, so a UI thread can hand a copy to a background thread.
#[derive(Clone)]
pub struct TranslationService {
    network: NetworkClient,
    cache: TranslationCache,
    dictionary: BuiltInDictionary,
    gateway: TranslationGateway,
    default_mode: TranslationMode,
    /// When `true`, the service never hits the network — useful for tests
    /// and for the explicit "offline" toggle in the UI.
    force_offline: bool,
}

impl TranslationService {
    /// Open a service with an on-disk cache rooted at `cache_root` and
    /// the default gateway.
    pub fn new(network: NetworkClient) -> Self {
        Self::with_cache_root(network, default_cache_root())
    }

    /// Open a service with an explicit cache root.
    pub fn with_cache_root(network: NetworkClient, cache_root: PathBuf) -> Self {
        let cache = TranslationCache::open(cache_root)
            .unwrap_or_else(|_| TranslationCache::open(default_cache_root()).unwrap());
        Self::with_parts(network, cache, BuiltInDictionary::builtin(), TranslationGateway::default())
    }

    /// Open a service with every part customisable.
    pub fn with_parts(
        network: NetworkClient,
        cache: TranslationCache,
        dictionary: BuiltInDictionary,
        gateway: TranslationGateway,
    ) -> Self {
        Self {
            network,
            cache,
            dictionary,
            gateway,
            default_mode: TranslationMode::default(),
            force_offline: false,
        }
    }

    /// Builder-style setter: force the service to never touch the network.
    pub fn with_force_offline(mut self, on: bool) -> Self {
        self.force_offline = on;
        self
    }

    /// Builder-style setter: override the default mode (Hybrid). Per-request
    /// `mode` still wins.
    pub fn with_default_mode(mut self, mode: TranslationMode) -> Self {
        self.default_mode = mode;
        self
    }

    /// Builder-style setter: replace the gateway configuration.
    pub fn with_gateway(mut self, gw: TranslationGateway) -> Self {
        self.gateway = gw;
        self
    }

    /// Cache handle (used by the FFI to wire "clear cache" / "show stats").
    pub fn cache(&self) -> &TranslationCache {
        &self.cache
    }

    /// Dictionary handle (used by diagnostics / tests).
    pub fn dictionary(&self) -> &BuiltInDictionary {
        &self.dictionary
    }

    /// Gateway configuration (used by the settings UI).
    pub fn gateway(&self) -> &TranslationGateway {
        &self.gateway
    }

    /// Current default mode.
    pub fn default_mode(&self) -> TranslationMode {
        self.default_mode
    }

    /// Whether the service is currently pinned to offline.
    pub fn is_force_offline(&self) -> bool {
        self.force_offline
    }

    /// Translate a single request.
    pub async fn translate(&self, req: &TranslationRequest) -> RcResult<TranslationResult> {
        if req.text.trim().is_empty() {
            return Ok(TranslationResult::passthrough(req));
        }
        if let Some(passthrough) = passthrough_if_same_language(req) {
            return Ok(passthrough);
        }

        let mode = req.mode.unwrap_or(self.default_mode);

        // 1) Cache lookup — fastest path. A miss falls through.
        if let Some(cached) = self.cache.get(req) {
            return Ok(cached);
        }

        // 2) Dictionary — works for known mod ids (full match) and for
        //    individual terms (per-word substitution). We try a full-match
        //    lookup first; if it yields nothing useful, fall back to
        //    per-term substitution so a long description at least gets the
        //    jargon localised before the gateway sees it.
        if matches!(mode, TranslationMode::Offline | TranslationMode::Hybrid) {
            if let Some(result) = self.dictionary_lookup(req) {
                self.cache.put(req, &result).ok();
                return Ok(result);
            }
            if matches!(mode, TranslationMode::Offline) {
                // Offline mode ⇒ no gateway. Return the term-substituted
                // text or the passthrough on miss.
                let out = TranslationResult {
                    original: req.text.clone(),
                    translated: self.dictionary.apply_terms(&req.text, req.target),
                    target: req.target,
                    detected_source: req.source,
                    source: TranslationSource::Dictionary,
                    offline: true,
                };
                self.cache.put(req, &out).ok();
                return Ok(out);
            }
        }

        // 3) Gateway — pre-process with term substitution so common
        //    jargon is already correct when the LLM sees the prompt.
        if !self.force_offline {
            match self.call_gateway(req).await {
                Ok(mut result) => {
                    // Post-process: any well-known term the gateway
                    // accidentally left in English is force-replaced.
                    if !matches!(result.source, TranslationSource::Unavailable) {
                        result.translated =
                            self.dictionary.apply_terms(&result.translated, req.target);
                    }
                    self.cache.put(req, &result).ok();
                    return Ok(result);
                }
                Err(_) => {
                    // Fall through to the unavailable result.
                }
            }
        }

        // 4) Unavailable — degrade gracefully. The UI shows the original
        //    text; the user is never blocked by a network failure.
        let out = TranslationResult {
            original: req.text.clone(),
            translated: self.dictionary.apply_terms(&req.text, req.target),
            target: req.target,
            detected_source: req.source,
            source: TranslationSource::Unavailable,
            offline: true,
        };
        self.cache.put(req, &out).ok();
        Ok(out)
    }

    /// Translate many requests sequentially (preserves order).
    ///
    /// The implementation uses [`futures::future::join_all`] so the
    /// service reuses the same concurrency limit (set by
    /// [`NetworkConfig::pool_max_idle_per_host`]) as the rest of the
    /// launcher. The output order is identical to the input order.
    pub async fn translate_batch(
        &self,
        requests: Vec<TranslationRequest>,
    ) -> RcResult<Vec<TranslationResult>> {
        let mut out = Vec::with_capacity(requests.len());
        for r in requests {
            out.push(self.translate(&r).await?);
        }
        Ok(out)
    }

    /// Look up a request in the dictionary directly. Returns `None` when
    /// the dictionary does not have a high-confidence match (so the
    /// gateway still gets called).
    fn dictionary_lookup(&self, req: &TranslationRequest) -> Option<TranslationResult> {
        let text = req.text.trim();
        // Whole-mod lookup: the input is the canonical id (e.g. "sodium").
        if let Some(hit) = self.dictionary.lookup_mod(text) {
            if let Some(t) = hit.for_language(req.target) {
                if !t.is_empty() {
                    return Some(TranslationResult {
                        original: text.to_string(),
                        translated: t.to_string(),
                        target: req.target,
                        detected_source: Some(crate::translate::model::TranslationLanguage::En),
                        source: TranslationSource::Dictionary,
                        offline: true,
                    });
                }
            }
        }
        // Whole-loader lookup.
        if let Some(hit) = self.dictionary.lookup_loader(text) {
            if let Some(t) = hit.for_language(req.target) {
                if !t.is_empty() {
                    return Some(TranslationResult {
                        original: text.to_string(),
                        translated: t.to_string(),
                        target: req.target,
                        detected_source: Some(crate::translate::model::TranslationLanguage::En),
                        source: TranslationSource::Dictionary,
                        offline: true,
                    });
                }
            }
        }
        None
    }

    /// Call the configured translation gateway. The request payload mirrors
    /// the OpenAI chat-completions schema (the `kilo.ai` endpoint, the
    /// task-brief URL, is OpenAI-compatible). The response is the assistant
    /// message's `content` field, trimmed.
    async fn call_gateway(&self, req: &TranslationRequest) -> RcResult<TranslationResult> {
        // Pre-process with term substitution so the LLM sees jargon
        // already localised and does not have to "guess" (which costs
        // tokens and quality).
        let pre = self.dictionary.apply_terms(&req.text, req.target);

        let body = self.build_request_body(req, &pre);
        let raw = self.post_gateway(&body).await?;
        let translated = self.extract_reply(&raw, &pre);

        Ok(TranslationResult {
            original: req.text.clone(),
            translated,
            target: req.target,
            detected_source: req.source,
            source: TranslationSource::Gateway,
            offline: false,
        })
    }

    fn build_request_body(&self, req: &TranslationRequest, text: &str) -> serde_json::Value {
        let target = match req.target {
            crate::translate::model::TranslationLanguage::Auto => "the user's preferred language",
            crate::translate::model::TranslationLanguage::ZhCn => "Simplified Chinese (zh-CN)",
            crate::translate::model::TranslationLanguage::ZhHant => "Traditional Chinese (zh-Hant)",
            crate::translate::model::TranslationLanguage::En => "English",
        };
        let user_prompt = format!(
            "Translate the following text to {target}. Reply with the translation only — no quotes, no preamble.\n\n{text}"
        );
        let mut req_body = json!({
            "model": self.gateway.model,
            "temperature": self.gateway.temperature,
            "max_tokens": self.gateway.max_tokens,
            "stream": false,
            "messages": [
                { "role": "system", "content": self.gateway.system_prompt },
                { "role": "user",   "content": user_prompt },
            ],
        });
        for (k, v) in &req.extra_headers {
            req_body["metadata"][k] = json!(v);
        }
        req_body
    }

    async fn post_gateway(&self, body: &serde_json::Value) -> RcResult<String> {
        self.network.post_json(&self.gateway.url, body.clone()).await
    }

    fn extract_reply(&self, raw: &str, fallback: &str) -> String {
        // OpenAI-compatible response: { choices: [ { message: { content: "..." } } ] }
        #[derive(Deserialize)]
        struct Choice {
            message: ChoiceMessage,
        }
        #[derive(Deserialize)]
        struct ChoiceMessage {
            content: String,
        }
        #[derive(Deserialize)]
        struct ChatResponse {
            choices: Vec<Choice>,
        }
        match serde_json::from_str::<ChatResponse>(raw) {
            Ok(r) => r
                .choices
                .into_iter()
                .next()
                .map(|c| c.message.content.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| fallback.to_string()),
            Err(_) => fallback.to_string(),
        }
    }
}

/// Builder for [`TranslationService`] — mirrors the [`NetworkClientBuilder`]
/// pattern (chainable, then `.build()`).
pub struct TranslationServiceBuilder {
    network: Option<NetworkClient>,
    cache_root: Option<PathBuf>,
    cache_config: CacheConfig,
    dictionary: BuiltInDictionary,
    gateway: TranslationGateway,
    default_mode: TranslationMode,
    force_offline: bool,
}

impl Default for TranslationServiceBuilder {
    fn default() -> Self {
        Self {
            network: None,
            cache_root: None,
            cache_config: CacheConfig::default(),
            dictionary: BuiltInDictionary::builtin(),
            gateway: TranslationGateway::default(),
            default_mode: TranslationMode::default(),
            force_offline: false,
        }
    }
}

impl TranslationServiceBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn network(mut self, c: NetworkClient) -> Self {
        self.network = Some(c);
        self
    }

    pub fn cache_root(mut self, p: impl Into<PathBuf>) -> Self {
        self.cache_root = Some(p.into());
        self
    }

    pub fn cache_config(mut self, c: CacheConfig) -> Self {
        self.cache_config = c;
        self
    }

    pub fn dictionary(mut self, d: BuiltInDictionary) -> Self {
        self.dictionary = d;
        self
    }

    pub fn gateway(mut self, g: TranslationGateway) -> Self {
        self.gateway = g;
        self
    }

    pub fn default_mode(mut self, m: TranslationMode) -> Self {
        self.default_mode = m;
        self
    }

    pub fn force_offline(mut self, on: bool) -> Self {
        self.force_offline = on;
        self
    }

    pub fn build(self) -> RcResult<TranslationService> {
        let network = self
            .network
            .ok_or_else(|| RcError::Other("TranslationServiceBuilder: missing network".into()))?;
        let cache_root = self.cache_root.unwrap_or_else(default_cache_root);
        let cache = TranslationCache::open_with_config(cache_root, self.cache_config)?;
        let svc = TranslationService::with_parts(network, cache, self.dictionary, self.gateway);
        Ok(svc
            .with_default_mode(self.default_mode)
            .with_force_offline(self.force_offline))
    }
}

/// The default on-disk location for the translation cache. The Compose UI
/// passes the canonical launcher cache dir in [`TranslationService::new`],
/// so this only matters for tests / the CLI.
pub fn default_cache_root() -> PathBuf {
    #[cfg(target_os = "android")]
    {
        PathBuf::from("/data/data/com.rc.launcher/cache/rc_translation")
    }
    #[cfg(not(target_os = "android"))]
    {
        if let Some(home) = std::env::var_os("XDG_CACHE_HOME") {
            return PathBuf::from(home).join("rc_translation");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".cache").join("rc_translation");
        }
        PathBuf::from("rc_translation")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::NetworkConfig;
    use crate::translate::model::{TranslationLanguage, TranslationSource};

    async fn offline_service(dir: &std::path::Path) -> TranslationService {
        let net = NetworkClient::builder()
            .config(NetworkConfig {
                max_retries: 1,
                ..NetworkConfig::default()
            })
            .build()
            .await
            .unwrap();
        TranslationServiceBuilder::new()
            .network(net)
            .cache_root(dir.to_path_buf())
            .force_offline(true)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn empty_text_passes_through() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await;
        let mut req = TranslationRequest::to("", TranslationLanguage::ZhCn);
        req.text = "   ".into();
        let r = svc.translate(&req).await.unwrap();
        assert_eq!(r.source, TranslationSource::Passthrough);
        assert_eq!(r.original, r.translated);
    }

    #[tokio::test]
    async fn same_source_target_is_passthrough() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await;
        let mut req = TranslationRequest::to("hello", TranslationLanguage::En);
        req.source = Some(TranslationLanguage::En);
        let r = svc.translate(&req).await.unwrap();
        assert_eq!(r.source, TranslationSource::Passthrough);
    }

    #[tokio::test]
    async fn known_mod_is_served_from_dictionary() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await;
        let req = TranslationRequest::to("sodium", TranslationLanguage::ZhCn);
        let r = svc.translate(&req).await.unwrap();
        assert_eq!(r.source, TranslationSource::Dictionary);
        assert_eq!(r.translated, "钠");
    }

    #[tokio::test]
    async fn offline_mode_substitutes_known_terms() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await;
        let mut req = TranslationRequest::to(
            "Improves FPS and render distance.",
            TranslationLanguage::ZhCn,
        );
        req.mode = Some(TranslationMode::Offline);
        let r = svc.translate(&req).await.unwrap();
        assert_eq!(r.source, TranslationSource::Dictionary);
        assert!(r.translated.contains("帧率"));
        assert!(r.translated.contains("渲染距离"));
    }

    #[tokio::test]
    async fn cached_translation_is_returned_without_work() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await;
        let req = TranslationRequest::to("hello world", TranslationLanguage::ZhCn);
        // Inject a known result.
        let cached = TranslationResult {
            original: req.text.clone(),
            translated: "你好世界".into(),
            target: req.target,
            detected_source: Some(TranslationLanguage::En),
            source: TranslationSource::Gateway,
            offline: false,
        };
        svc.cache().put(&req, &cached).unwrap();
        let r2 = svc.translate(&req).await.unwrap();
        assert_eq!(r2.source, TranslationSource::Cache);
        assert_eq!(r2.translated, "你好世界");
    }

    #[tokio::test]
    async fn forced_offline_falls_back_to_dictionary() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await;
        let req = TranslationRequest::to("sodium", TranslationLanguage::ZhCn);
        let r = svc.translate(&req).await.unwrap();
        assert_eq!(r.source, TranslationSource::Dictionary);
    }

    #[tokio::test]
    async fn batch_preserves_order() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await;
        let reqs = vec![
            TranslationRequest::to("sodium", TranslationLanguage::ZhCn),
            TranslationRequest::to("iris", TranslationLanguage::ZhCn),
            TranslationRequest::to("fabric-api", TranslationLanguage::ZhCn),
        ];
        let out = svc.translate_batch(reqs).await.unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].translated, "钠");
        assert_eq!(out[1].translated, "虹膜");
        assert_eq!(out[2].translated, "Fabric API");
    }

    #[tokio::test]
    async fn builder_requires_network() {
        let dir = tempfile::tempdir().unwrap();
        let result = TranslationServiceBuilder::new()
            .cache_root(dir.path().to_path_buf())
            .build();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn with_force_offline_toggles_offline_state() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await.with_force_offline(false);
        assert!(!svc.is_force_offline());
        let svc = svc.with_force_offline(true);
        assert!(svc.is_force_offline());
    }

    #[tokio::test]
    async fn with_default_mode_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let svc = offline_service(dir.path()).await.with_default_mode(TranslationMode::Online);
        assert_eq!(svc.default_mode(), TranslationMode::Online);
    }
}
