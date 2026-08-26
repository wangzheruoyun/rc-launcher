//! Safe-loading / verification of renderer plugins (task 9).
//!
//! Mirrors the *safe loading* half of FCL's `RendererPlugin` and Zalith's
//! `VerifiedPluginLoad` / `NativeLibPlugin`: before a (non-system) renderer is
//! allowed to load, it must be checked for ABI support, native-lib presence,
//! path safety and *trust*. Trust is established either by being a built-in
//! (`System`), by being signed by a trusted author, or by matching a trusted
//! integrity hash. The signature check is pluggable behind [`SignatureVerifier`]
//! so a real Ed25519 backend (à la Zalith's `Ed25519Verifier`) can be dropped
//! in without touching the rest of the pipeline.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::runtime::Abi;

use super::native_lib::{LibVerify, NativeLibSource};
use super::renderer::{RendererPlugin, TrustLevel};

/// Everything the validator needs about the host to decide if a plugin is safe.
#[derive(Debug, Clone)]
pub struct ValidationContext {
    /// The ABI the game will actually run on.
    pub abi: Abi,
    /// The app's `nativeLibraryDir` (where renderer `.so`s live).
    pub native_lib_dir: PathBuf,
}

impl ValidationContext {
    pub fn new(abi: Abi, native_lib_dir: impl Into<PathBuf>) -> Self {
        Self {
            abi,
            native_lib_dir: native_lib_dir.into(),
        }
    }
}

/// Severity of a validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    /// Hard failure — the plugin must not be loaded.
    Error,
    /// Soft warning — loading may proceed but the user should be told.
    Warning,
}

/// A single validation finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    pub code: String,
    pub message: String,
}

impl ValidationIssue {
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Error,
            code: code.into(),
            message: message.into(),
        }
    }
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Warning,
            code: code.into(),
            message: message.into(),
        }
    }
}

/// The outcome of validating a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// Final trust classification after validation.
    pub trust: TrustLevel,
    /// Whether the plugin is safe to load.
    pub safe_to_load: bool,
    /// Findings (errors + warnings).
    pub issues: Vec<ValidationIssue>,
    /// The integrity hash computed for trust-list matching.
    pub integrity_hash: String,
}

impl ValidationReport {
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
    }
}

/// A pluggable signature verifier (the verification backend is swappable).
///
/// The default [`HashTrustStoreVerifier`] uses integrity-hash + author matching
/// (no external crypto needed). A real Ed25519 verifier (Zalith's
/// `Ed25519Verifier`) can implement this trait and be supplied to
/// [`TrustStore::verify_signature`].
pub trait SignatureVerifier {
    /// Verify `signature` over `message` (the plugin's canonical descriptor).
    fn verify(&self, message: &str, signature: &str) -> bool;
}

/// The default trust strategy: integrity-hash + author matching (no crypto dep).
pub struct HashTrustStoreVerifier;

impl SignatureVerifier for HashTrustStoreVerifier {
    fn verify(&self, _message: &str, _signature: &str) -> bool {
        // No crypto backend by default: we must never rubber-stamp a signature
        // we cannot actually check. Real verification plugs in via the trait.
        false
    }
}

/// A real, keyed-HMAC signature verifier (HMAC-SHA1, built on the `sha1` crate
/// already used for integrity).
///
/// This turns the pluggable [`SignatureVerifier`] trait into something that
/// actually works: a launcher (or trusted author) signs a plugin's canonical
/// integrity hash with a shared key and the validator verifies it before
/// granting trust — the Zalith `VerifiedPluginLoad` / FCL `RendererPlugin`
/// safe-loading pattern, implemented without pulling in an Ed25519 dependency.
/// The default [`HashTrustStoreVerifier`] stays as the explicit "never
/// rubber-stamp" fallback for stores that carry no signing key.
pub struct HmacSha1Verifier {
    /// The shared verification key (held by the launcher / trust store).
    pub key: Vec<u8>,
}

impl HmacSha1Verifier {
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        Self { key: key.into() }
    }

    /// Sign `message` (typically a plugin's `integrity_hash()`) and return the
    /// hex signature, for use by signing tooling and tests.
    pub fn sign(&self, message: &str) -> String {
        hex_encode(&hmac_sha1(&self.key, message.as_bytes()))
    }
}

impl SignatureVerifier for HmacSha1Verifier {
    fn verify(&self, message: &str, signature: &str) -> bool {
        let expected = hmac_sha1(&self.key, message.as_bytes());
        let got = match hex_decode(signature) {
            Some(b) => b,
            None => return false,
        };
        if expected.len() != got.len() {
            return false;
        }
        // constant-time comparison so the key is not leaked via timing
        let mut diff = 0u8;
        for (a, b) in expected.iter().zip(got.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// SHA-1 over `data` (the same primitive used for native-lib integrity).
fn sha1_bytes(data: &[u8]) -> Vec<u8> {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(data);
    h.finalize().to_vec()
}

/// HMAC-SHA1 (RFC 2104) over `msg` with `key`.
fn hmac_sha1(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        sha1_bytes(key)
    } else {
        key.to_vec()
    };
    k.resize(BLOCK, 0);
    let mut ipad = Vec::with_capacity(BLOCK + msg.len());
    let mut opad = Vec::with_capacity(BLOCK);
    for &ki in k.iter().take(BLOCK) {
        ipad.push(ki ^ 0x36);
        opad.push(ki ^ 0x5c);
    }
    ipad.extend_from_slice(msg);
    let inner = sha1_bytes(&ipad);
    let mut outer = opad;
    outer.extend_from_slice(&inner);
    sha1_bytes(&outer)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let val = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = val(bytes[i])?;
        let lo = val(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

/// Trust store: the set of authors / integrity hashes the launcher trusts.
///
/// Mirrors Zalith's `trusted-authors.json` + `.sig`: instead of embedding a
/// signature verifier, we record *who* and *what* we trust, and match a plugin
/// against that list by author name and/or SHA-1 integrity hash.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    /// Trusted author identities (plugin `author` field).
    #[serde(default)]
    pub trusted_authors: HashSet<String>,
    /// Trusted plugin integrity hashes (see [`RendererPlugin::integrity_hash`]).
    #[serde(default)]
    pub trusted_hashes: HashSet<String>,
    /// Authors that are explicitly distrusted (override).
    #[serde(default)]
    pub distrusted_authors: HashSet<String>,
}

impl TrustStore {
    /// An empty (paranoid) trust store — nothing third-party is trusted.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parse a trust store from JSON (the on-disk `trusted-authors.json`).
    pub fn from_json(s: &str) -> RcResult<Self> {
        serde_json::from_str(s).map_err(RcError::Json)
    }

    /// Insert a trusted author.
    pub fn trust_author(&mut self, author: impl Into<String>) {
        self.trusted_authors.insert(author.into());
    }

    /// Insert a trusted integrity hash.
    pub fn trust_hash(&mut self, hash: impl Into<String>) {
        self.trusted_hashes.insert(hash.into());
    }

    /// Distrust an author (overrides a trusted-authors entry).
    pub fn distrust_author(&mut self, author: impl Into<String>) {
        self.distrusted_authors.insert(author.into());
    }

    /// Is `author` in the trusted authors set (`None` => not trusted)?
    pub fn is_trusted_author(&self, author: Option<&str>) -> bool {
        match author {
            Some(a) => !self.distrusted_authors.contains(a) && self.trusted_authors.contains(a),
            None => false,
        }
    }

    /// Is `hash` in the trusted hashes set?
    pub fn is_trusted_hash(&self, hash: &str) -> bool {
        self.trusted_hashes.contains(hash)
    }

    /// Verify `signature` over `message` using `verifier`. Hooks in a real
    /// crypto backend; the default [`HashTrustStoreVerifier`] always returns
    /// `false` (see its docs).
    pub fn verify_signature(
        &self,
        verifier: &dyn SignatureVerifier,
        message: &str,
        signature: &str,
    ) -> bool {
        verifier.verify(message, signature)
    }
}

/// Validate `plugin` against `ctx` and `store`, producing a [`ValidationReport`].
///
/// The checks (order matters):
/// 1. **Path safety** — every native lib name is a safe bare file name.
/// 2. **ABI support** — the host ABI is covered by `supported_abis` (if set).
/// 3. **Native-lib presence** — non-optional libs exist under `native_lib_dir`.
/// 4. **Trust** — `System` plugins always pass; others must be trusted by
///    `store` (author or integrity hash) or be `UserApproved`; if
///    `requires_validation` and untrusted, that is an error.
pub fn validate(
    plugin: &RendererPlugin,
    ctx: &ValidationContext,
    store: &TrustStore,
) -> ValidationReport {
    validate_with_verifier(plugin, ctx, store, None)
}

/// Validate `plugin` against `ctx` and `store`, optionally verifying a detached
/// signature with `verifier`, producing a [`ValidationReport`].
///
/// The checks (order matters):
/// 1. **Path safety** — every native lib name is a safe bare file name.
/// 2. **ABI support** — the host ABI is covered by `supported_abis` (if set).
/// 3. **Native-lib presence + integrity** — non-optional libs exist under
///    `native_lib_dir` *and* match their preset size + SHA-1
///    ([`crate::plugins::native_lib::NativeLib::verify_on_disk`]), so a tampered
///    lib is rejected rather than silently trusted.
/// 4. **Detached signature** (if the plugin ships one) — verified against the
///    canonical integrity hash via `verifier`. A missing verifier degrades to a
///    warning instead of a silent pass.
/// 5. **Trust** — `System` plugins always pass; others must be trusted by
///    `store` (author or integrity hash) or be `UserApproved`; if
///    `requires_validation` and untrusted, that is an error.
pub fn validate_with_verifier(
    plugin: &RendererPlugin,
    ctx: &ValidationContext,
    store: &TrustStore,
    verifier: Option<&dyn SignatureVerifier>,
) -> ValidationReport {
    let mut issues: Vec<ValidationIssue> = Vec::new();
    let hash = plugin.integrity_hash();

    // 1) path safety
    for lib in &plugin.native_libs {
        if let Err(msg) = lib.validate_name() {
            issues.push(ValidationIssue::error("unsafe_native_lib_name", msg));
        }
    }

    // 2) ABI support
    if !plugin.supports_abi(ctx.abi) {
        issues.push(ValidationIssue::error(
            "unsupported_abi",
            format!(
                "renderer `{}` does not ship natives for ABI {}",
                plugin.id,
                ctx.abi.as_android_abi()
            ),
        ));
    }

    // 3) native-lib presence + integrity
    for lib in plugin.native_libs_for(ctx.abi) {
        let dir = match lib.source {
            NativeLibSource::NativeLibDir => ctx.native_lib_dir.clone(),
            NativeLibSource::PluginOwned => ctx.native_lib_dir.join(&plugin.id),
            NativeLibSource::LwjglNatives => ctx
                .native_lib_dir
                .join(format!("lwjgl-natives-{}", ctx.abi.as_android_abi())),
            NativeLibSource::JreLib => ctx.native_lib_dir.join("jre-lib"),
        };
        let path = dir.join(&lib.file_name);
        // Delegate to `NativeLib::verify_on_disk` so a *present* lib is checked
        // for both size and SHA-1 against its preset (the FCL APK inventory),
        // not merely existence. A tampered/mismatched lib becomes a hard error
        // instead of being silently trusted.
        match lib.verify_on_disk(&path) {
            Ok(LibVerify::Match) => {}
            Ok(LibVerify::Missing) => {
                if lib.optional {
                    issues.push(ValidationIssue::warning(
                        "missing_optional_native_lib",
                        format!("optional native lib missing: {}", path.display()),
                    ));
                } else {
                    issues.push(ValidationIssue::error(
                        "missing_native_lib",
                        format!("required native lib missing: {}", path.display()),
                    ));
                }
            }
            Ok(LibVerify::SizeMismatch) => {
                issues.push(ValidationIssue::error(
                    "native_lib_size_mismatch",
                    format!(
                        "native lib {} size does not match the preset",
                        path.display()
                    ),
                ));
            }
            Ok(LibVerify::HashMismatch) => {
                issues.push(ValidationIssue::error(
                    "native_lib_hash_mismatch",
                    format!(
                        "native lib {} SHA-1 does not match the preset (tampered?)",
                        path.display()
                    ),
                ));
            }
            Err(e) => {
                issues.push(ValidationIssue::error(
                    "native_lib_read_error",
                    format!("failed to read native lib {}: {e}", path.display()),
                ));
            }
        }
    }

    // 4) detached signature (if the plugin ships one)
    if let Some(sig) = &plugin.signature {
        match verifier {
            Some(v) => {
                if !v.verify(&hash, sig) {
                    issues.push(ValidationIssue::error(
                        "signature_mismatch",
                        format!(
                            "renderer `{}` detached signature does not verify against its integrity hash",
                            plugin.id
                        ),
                    ));
                }
            }
            None => {
                issues.push(ValidationIssue::warning(
                    "signature_unverifiable",
                    format!(
                        "renderer `{}` ships a signature but no verifier was supplied",
                        plugin.id
                    ),
                ));
            }
        }
    }

    // 5) trust
    // 4) trust
    let trusted_by_author = store.is_trusted_author(plugin.author.as_deref());
    let trusted_by_hash = store.is_trusted_hash(&hash);
    let mut effective_trust = plugin.trust;
    if plugin.trust != TrustLevel::System && (trusted_by_author || trusted_by_hash) {
        effective_trust = TrustLevel::TrustedAuthor;
    }

    let trusted = plugin.trust == TrustLevel::System
        || plugin.trust == TrustLevel::UserApproved
        || trusted_by_author
        || trusted_by_hash;

    if plugin.requires_validation && !trusted {
        issues.push(ValidationIssue::error(
            "untrusted_plugin",
            format!(
                "renderer `{}` requires validation but is not trusted (author={:?}, hash={})",
                plugin.id, plugin.author, hash
            ),
        ));
    }

    let safe_to_load = !issues.iter().any(|i| i.severity == IssueSeverity::Error);

    ValidationReport {
        trust: effective_trust,
        safe_to_load,
        issues,
        integrity_hash: hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::options::Renderer;
    use crate::plugins::native_lib::NativeLib;
    use crate::plugins::renderer::{
        renderer_plugin, RendererPlugin, RendererRegistry, TrustLevel, WindowingBackend,
    };
    use sha1::{Digest, Sha1};
    use tempfile;

    #[test]
    fn system_plugin_is_always_safe() {
        let p = renderer_plugin(Renderer::Gl4es);
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let store = TrustStore::empty();
        let report = validate(&p, &ctx, &store);
        // GL4ES's lib isn't on disk at /data/app/lib/arm64, so presence fails;
        // but trust + ABI + path-safety must hold for a System plugin.
        assert_eq!(report.trust, TrustLevel::System);
        assert!(report.issues.iter().any(|i| i.code == "missing_native_lib"));
    }

    #[test]
    fn untrusted_required_plugin_is_rejected() {
        let p = RendererPlugin::builder("evil", "Evil Renderer")
            .gl_libname("libevil.so")
            .requires_validation(true)
            .author("mallory")
            .build();
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let store = TrustStore::empty();
        let report = validate(&p, &ctx, &store);
        assert!(!report.safe_to_load);
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "untrusted_plugin" && i.severity == IssueSeverity::Error));
    }

    #[test]
    fn trusted_author_passes_validation() {
        let p = RendererPlugin::builder("sdl", "SDL Renderer")
            .gl_libname("liblwjgl_sdl.so")
            .backend(WindowingBackend::Sdl)
            .requires_validation(true)
            .author("trusted-dev")
            .build();
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let mut store = TrustStore::empty();
        store.trust_author("trusted-dev");
        let report = validate(&p, &ctx, &store);
        assert_eq!(report.trust, TrustLevel::TrustedAuthor);
        assert!(!report.issues.iter().any(|i| i.code == "untrusted_plugin"));
    }

    #[test]
    fn trusted_hash_passes_validation() {
        let mut p = RendererPlugin::builder("sdl", "SDL Renderer")
            .gl_libname("liblwjgl_sdl.so")
            .backend(WindowingBackend::Sdl)
            .requires_validation(true)
            .build();
        let hash = p.integrity_hash();
        let mut store = TrustStore::empty();
        store.trust_hash(hash);
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let report = validate(&p, &ctx, &store);
        assert_eq!(report.trust, TrustLevel::TrustedAuthor);
        assert!(!report.issues.iter().any(|i| i.code == "untrusted_plugin"));
        // tampering with the descriptor changes the hash and breaks the trust
        p.display_name = "Tampered".into();
        let report2 = validate(&p, &ctx, &store);
        assert!(report2.issues.iter().any(|i| i.code == "untrusted_plugin"));
    }

    #[test]
    fn distrusted_author_overrides() {
        let p = RendererPlugin::builder("sdl", "SDL Renderer")
            .gl_libname("liblwjgl_sdl.so")
            .backend(WindowingBackend::Sdl)
            .requires_validation(true)
            .author("trusted-dev")
            .build();
        let mut store = TrustStore::empty();
        store.trust_author("trusted-dev");
        store.distrust_author("trusted-dev");
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let report = validate(&p, &ctx, &store);
        assert!(report.issues.iter().any(|i| i.code == "untrusted_plugin"));
    }

    #[test]
    fn traversal_in_native_lib_fails_validation() {
        let mut p = renderer_plugin(Renderer::Gl4es);
        p.native_libs
            .push(crate::plugins::native_lib::NativeLib::in_native_lib_dir(
                "../evil.so",
            ));
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let store = TrustStore::empty();
        let report = validate(&p, &ctx, &store);
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "unsafe_native_lib_name"));
    }

    #[test]
    fn trust_store_roundtrips_json() {
        let mut store = TrustStore::empty();
        store.trust_author("alice");
        store.trust_hash("deadbeef");
        let json = serde_json::to_string(&store).unwrap();
        let back = TrustStore::from_json(&json).unwrap();
        assert!(back.is_trusted_author(Some("alice")));
        assert!(back.is_trusted_hash("deadbeef"));
        assert!(!back.is_trusted_author(Some("bob")));
    }

    #[test]
    fn registry_validate_plugin_returns_report() {
        let reg = RendererRegistry::builtin();
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let store = TrustStore::empty();
        let report = reg.validate_plugin("opengles2", &ctx, &store).unwrap();
        assert_eq!(report.trust, TrustLevel::System);
        // unknown id is an error
        assert!(reg.validate_plugin("nope", &ctx, &store).is_err());
    }

    #[test]
    fn hmac_signature_verifies() {
        let mut p = RendererPlugin::builder("sdl", "SDL Renderer")
            .gl_libname("liblwjgl_sdl.so")
            .backend(WindowingBackend::Sdl)
            .requires_validation(true)
            .author("trusted-dev")
            .build();
        let hash = p.integrity_hash();
        let verifier = HmacSha1Verifier::new(b"launcher-key");
        p.signature = Some(verifier.sign(&hash));
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let mut store = TrustStore::empty();
        store.trust_author("trusted-dev");
        let report = validate_with_verifier(&p, &ctx, &store, Some(&verifier));
        assert!(
            !report.issues.iter().any(|i| i.code == "signature_mismatch"),
            "valid signature should verify"
        );
    }

    #[test]
    fn hmac_signature_rejects_tamper() {
        let mut p = RendererPlugin::builder("sdl", "SDL Renderer")
            .gl_libname("liblwjgl_sdl.so")
            .backend(WindowingBackend::Sdl)
            .build();
        let hash = p.integrity_hash();
        let verifier = HmacSha1Verifier::new(b"launcher-key");
        p.signature = Some(verifier.sign(&hash));
        // tamper with the descriptor after signing
        p.display_name = "Evil".into();
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let store = TrustStore::empty();
        let report = validate_with_verifier(&p, &ctx, &store, Some(&verifier));
        assert!(
            report.issues.iter().any(|i| i.code == "signature_mismatch"),
            "tampered descriptor must fail signature verification"
        );
    }

    #[test]
    fn signature_without_verifier_warns() {
        let mut p = RendererPlugin::builder("sdl", "SDL Renderer")
            .gl_libname("liblwjgl_sdl.so")
            .backend(WindowingBackend::Sdl)
            .build();
        let verifier = HmacSha1Verifier::new(b"launcher-key");
        p.signature = Some(verifier.sign(&p.integrity_hash()));
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let store = TrustStore::empty();
        // no verifier supplied -> warning, never a silent pass
        let report = validate(&p, &ctx, &store);
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "signature_unverifiable"));
        assert!(!report.issues.iter().any(|i| i.code == "signature_mismatch"));
    }

    #[test]
    fn integrity_check_rejects_sha1_mismatch() {
        let td = tempfile::tempdir().unwrap();
        // present lib: size matches the preset but bytes do not
        std::fs::write(td.path().join("libfake.so"), b"abcd").unwrap();
        let mut lib = NativeLib::in_native_lib_dir("libfake.so");
        lib.expected_size = Some(4);
        lib.expected_sha1 = Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string());
        let p = RendererPlugin::builder("fake", "Fake")
            .gl_libname("libfake.so")
            .native_lib(lib)
            .build();
        let ctx = ValidationContext::new(Abi::Arm64V8a, td.path());
        let store = TrustStore::empty();
        let report = validate(&p, &ctx, &store);
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.code == "native_lib_hash_mismatch"),
            "tampered present lib must be flagged"
        );
    }

    #[test]
    fn integrity_check_passes_when_sha1_matches() {
        let td = tempfile::tempdir().unwrap();
        let bytes = b"hello-world";
        std::fs::write(td.path().join("libfake.so"), bytes).unwrap();
        let mut h = Sha1::new();
        h.update(bytes);
        let sha1_hex = {
            let out = h.finalize();
            let mut s = String::new();
            for b in out {
                s.push_str(&format!("{:02x}", b));
            }
            s
        };
        let mut lib = NativeLib::in_native_lib_dir("libfake.so");
        lib.expected_size = Some(bytes.len() as u64);
        lib.expected_sha1 = Some(sha1_hex);
        let p = RendererPlugin::builder("fake", "Fake")
            .gl_libname("libfake.so")
            .native_lib(lib)
            .build();
        let ctx = ValidationContext::new(Abi::Arm64V8a, td.path());
        let store = TrustStore::empty();
        let report = validate(&p, &ctx, &store);
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.severity == IssueSeverity::Error),
            "matching size+sha1 should pass: {:?}",
            report.issues
        );
    }
}
