//! Pluggable renderer & native-library extension mechanism (task 9).
//!
//! This module turns the hard-coded [`crate::launch::options::Renderer`]
//! contract the launch engine consumes into a *pluggable* plugin system, in
//! the spirit of FCL's `RendererPlugin` and Zalith's `NativeLibPlugin` /
//! `VerifiedPluginLoad`:
//!
//! * [`renderer`] — [`RendererPlugin`]: a data-driven description of an
//!   OpenGL(ES) translation stack (GL4ES / NG-GL4ES / VirGL / Zink / ANGLE /
//!   SDL ...): its LWJGL `libname`, renderer environment, the native libraries
//!   it injects, the ABIs it ships for, and the provenance/trust metadata the
//!   safe-loading pipeline needs. [`RendererRegistry`] is the discoverable,
//!   mutable catalogue of renderers plus the resolver that computes the
//!   native-library directories to inject for a given `(plugin, abi)`.
//! * [`native_lib`] — [`NativeLib`] / [`NativeLibSource`]: the model behind
//!   *NativeLib injection*: which `.so` files a plugin contributes, from which
//!   search root, in what load order, for which ABI.
//! * [`validation`] — the *safe loading* half: a [`validation::TrustStore`]
//!   (trusted authors / integrity hashes, the Zalith `trusted-authors.json`
//!   idea), a pluggable [`validation::SignatureVerifier`] and a
//!   [`validation::validate`] pass that checks ABI support, native-lib presence,
//!   path safety and trust before a plugin is allowed to load.
//!
//! The launch engine keeps using [`crate::launch::options::Renderer`] (whose
//! `id()` / `gl_libname()` / `env()` are derived from the same built-in plugin
//! descriptors via [`renderer::renderer_plugin`]); the UI / plugin manager uses
//! this module to enumerate, register, inject and *verify* renderers.

pub mod fcl_apk;
pub mod native_lib;
pub mod renderer;
pub mod validation;

pub use fcl_apk::{
    manifest, preset_registry, ApkLibEntry, FclApkRenderer, FclApkRendererManifest,
    FCL_APK_RENDERER_MANIFEST,
};
pub use native_lib::{LibVerify, NativeLib, NativeLibSource};
pub use renderer::{
    renderer_plugin, RendererPlugin, RendererPluginBuilder, RendererRegistry, TrustLevel,
    WindowingBackend,
};
pub use validation::{
    validate, validate_with_verifier, HashTrustStoreVerifier, HmacSha1Verifier, IssueSeverity,
    SignatureVerifier, TrustStore, ValidationContext, ValidationIssue, ValidationReport,
};

/// All built-in renderers (the 5 FCL stacks plus the LWJGL SDL backend), in the
/// order they appear in the settings UI.
///
/// Kept as a free function (rather than a `const`) so callers that want to
/// extend it ([`RendererRegistry::with_plugin`]) get an owned, mutable copy.
pub fn builtin_registry() -> RendererRegistry {
    RendererRegistry::builtin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_is_wired() {
        // Ensure the public re-exports resolve and the doc'd types exist.
        let _ = std::mem::size_of::<RendererPlugin>();
        let _ = std::mem::size_of::<RendererRegistry>();
        let _ = std::mem::size_of::<NativeLib>();
        let _ = std::mem::size_of::<TrustStore>();
        let _ = std::mem::size_of::<ValidationReport>();
    }

    #[test]
    fn builtin_registry_is_populated() {
        let reg = builtin_registry();
        assert_eq!(reg.all().len(), 6);
        assert!(reg.get("opengles2").is_some());
        assert!(reg.get("sdl2").is_some());
    }
}
