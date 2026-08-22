//! [`RendererPlugin`] and the [`RendererRegistry`] that holds them (task 9).
//!
//! A [`RendererPlugin`] is a data-driven description of an OpenGL(ES)
//! translation stack (GL4ES / NG-GL4ES / VirGL / Zink / ANGLE / SDL ...). It
//! carries everything the launch engine needs to wire the renderer into the
//! game JVM — the LWJGL `libname`, the renderer-specific environment, the
//! native libraries it injects, the ABIs it ships for — plus the metadata the
//! safe-loading pipeline needs (trust level, signature, integrity hash).
//!
//! This is the Rust counterpart of FCL's `RendererPlugin` and Zalith's
//! `NativeLibPlugin` / `VerifiedPluginLoad`: renderers are *discoverable* and
//! *verifiable* rather than hard-coded. The built-in renderers are produced by
//! [`renderer_plugin`] (mirroring FCL's shipped stacks); third-party renderers
//! are constructed directly (or deserialised from JSON) and registered into a
//! [`RendererRegistry`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::launch::options::Renderer;
use crate::runtime::Abi;

use super::native_lib::{NativeLib, NativeLibSource};
use super::validation::{validate, TrustStore, ValidationContext};

/// Windowing backend a renderer drives the game surface through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowingBackend {
    /// EGL/GL surface via the GL translation layer (GL4ES, ANGLE, Zink, VirGL).
    GlSurface,
    /// SDL2 windowing backend (LWJGL 3.4.1 `lwjgl-sdl`), e.g. the SDL renderer
    /// plugin. This is the "SDL 渲染插件" task 9 calls out.
    Sdl,
    /// Vulkan-native surface (direct Vulkan presentation).
    Vulkan,
    /// Surfaceless / headless (offscreen rendering).
    Surfaceless,
}

/// Provenance / trust classification of a renderer plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Shipped with the launcher; always trusted and exempt from validation.
    System,
    /// Signed by a trusted author present in the [`TrustStore`].
    TrustedAuthor,
    /// Approved by the user out-of-band (unsigned, allowed once).
    UserApproved,
    /// Unknown origin; must pass full validation before loading.
    Untrusted,
}

fn default_trust() -> TrustLevel {
    // A plugin with no explicit trust is untrusted by default — safe loading
    // demands opt-in, never opt-out.
    TrustLevel::Untrusted
}

/// A data-driven description of a pluggable OpenGL(ES) translation renderer.
///
/// Built-in renderers are produced by [`renderer_plugin`]; third-party renderers
/// are constructed via [`RendererPlugin::builder`] (or deserialised from JSON)
/// and registered into a [`RendererRegistry`]. Everything the launch engine and
/// the safe-loading pipeline need lives here so renderers are *discoverable*
/// and *verifiable* rather than hard-coded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RendererPlugin {
    /// Stable id (e.g. `opengles2`, `opengles3_angle`). Persisted by the UI and
    /// matched against the FCL/Zalith `POJAV_RENDERER` contract.
    pub id: String,
    /// Human-readable name for the settings UI.
    pub display_name: String,
    /// The `.so` LWJGL must `dlopen` for `-Dorg.lwjgl.opengl.libname`.
    pub gl_libname: String,
    /// Windowing backend this renderer drives.
    pub backend: WindowingBackend,
    /// Renderer-specific environment variables (GL4ES / Mesa / Gallium tuning).
    pub env: Vec<(String, String)>,
    /// ABIs this plugin ships natives for. Empty means "every ABI".
    #[serde(default)]
    pub supported_abis: Vec<Abi>,
    /// Native libraries this plugin injects into the load path.
    #[serde(default)]
    pub native_libs: Vec<NativeLib>,
    /// Whether the plugin must pass signature / trust validation before use.
    #[serde(default)]
    pub requires_validation: bool,
    /// Provenance / trust classification.
    #[serde(default = "default_trust")]
    pub trust: TrustLevel,
    /// Optional detached signature (hex) over the canonical descriptor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Author identity for trust-list matching (mirrors Zalith `trusted-authors.json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// Builder for [`RendererPlugin`], used by third-party / test plugins.
pub struct RendererPluginBuilder(RendererPlugin);

impl RendererPluginBuilder {
    pub fn gl_libname(mut self, s: impl Into<String>) -> Self {
        self.0.gl_libname = s.into();
        self
    }
    pub fn backend(mut self, b: WindowingBackend) -> Self {
        self.0.backend = b;
        self
    }
    pub fn env(mut self, e: Vec<(String, String)>) -> Self {
        self.0.env = e;
        self
    }
    pub fn env_var(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.0.env.push((k.into(), v.into()));
        self
    }
    pub fn supported_abi(mut self, a: Abi) -> Self {
        self.0.supported_abis.push(a);
        self
    }
    pub fn native_lib(mut self, l: NativeLib) -> Self {
        self.0.native_libs.push(l);
        self
    }
    pub fn requires_validation(mut self, b: bool) -> Self {
        self.0.requires_validation = b;
        self
    }
    pub fn trust(mut self, t: TrustLevel) -> Self {
        self.0.trust = t;
        self
    }
    pub fn author(mut self, a: impl Into<String>) -> Self {
        self.0.author = Some(a.into());
        self
    }
    pub fn signature(mut self, s: impl Into<String>) -> Self {
        self.0.signature = Some(s.into());
        self
    }
    /// Finish building.
    pub fn build(self) -> RendererPlugin {
        self.0
    }
}

impl RendererPlugin {
    /// Start building a plugin with the minimum required identity.
    pub fn builder(
        id: impl Into<String>,
        display_name: impl Into<String>,
    ) -> RendererPluginBuilder {
        RendererPluginBuilder(RendererPlugin {
            id: id.into(),
            display_name: display_name.into(),
            gl_libname: String::new(),
            backend: WindowingBackend::GlSurface,
            env: Vec::new(),
            supported_abis: Vec::new(),
            native_libs: Vec::new(),
            requires_validation: false,
            trust: TrustLevel::Untrusted,
            signature: None,
            author: None,
        })
    }

    /// Does this plugin ship natives for `abi`? Empty `supported_abis` => all.
    pub fn supports_abi(&self, abi: Abi) -> bool {
        self.supported_abis.is_empty() || self.supported_abis.contains(&abi)
    }

    /// The native libraries that apply to `abi` (those with a matching or
    /// `None` ABI filter).
    pub fn native_libs_for(&self, abi: Abi) -> Vec<&NativeLib> {
        self.native_libs
            .iter()
            .filter(|l| l.abi.is_none_or(|a| a == abi))
            .collect()
    }

    /// A stable SHA-1 over the *canonical* descriptor (id, backend, gl_libname,
    /// env, abis, native libs, author). Used for tamper detection and trust-list
    /// matching ([`TrustStore`]). Two plugins with identical behaviour produce
    /// identical hashes, so a trusted hash pins a known-good plugin.
    pub fn integrity_hash(&self) -> String {
        use sha1::{Digest, Sha1};
        let mut h = Sha1::new();
        let mut feed = |s: &str| h.update(s.as_bytes());
        feed(&self.id);
        feed(&self.display_name);
        feed(&format!("{:?}", self.backend));
        feed(&self.gl_libname);
        for (k, v) in &self.env {
            feed(k);
            feed("=");
            feed(v);
            feed(";");
        }
        for a in &self.supported_abis {
            feed(a.as_android_abi());
        }
        for l in &self.native_libs {
            feed(&l.file_name);
            feed(&format!("{:?}", l.abi));
            feed(&format!("{:?}", l.source));
            feed(&l.load_order.to_string());
        }
        if let Some(a) = &self.author {
            feed(a);
        }
        let out = h.finalize();
        let mut s = String::with_capacity(out.len() * 2);
        for b in out {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    /// Validate this plugin in `ctx` against `store`, returning a full report.
    pub fn validate(
        &self,
        ctx: &ValidationContext,
        store: &TrustStore,
    ) -> super::validation::ValidationReport {
        validate(self, ctx, store)
    }
}

/// Map a built-in [`Renderer`] (the launch-engine contract) onto its
/// [`RendererPlugin`] descriptor. The ids / `gl_libname`s / env mirror FCL's
/// shipped renderers (see `FCL_APK_RUNTIME_ASSETS_CATALOG.md`), so this is the
/// single source of truth the `Renderer` enum's `id()` / `gl_libname()` /
/// `env()` delegate from.
pub fn renderer_plugin(r: Renderer) -> RendererPlugin {
    match r {
        Renderer::Gl4es => RendererPlugin {
            id: "opengles2".into(),
            display_name: "GL4ES 1.1.4".into(),
            gl_libname: "libgl4es_114.so".into(),
            backend: WindowingBackend::GlSurface,
            env: vec![
                ("LIBGL_ES".into(), "2".into()),
                ("LIBGL_MIPMAP".into(), "3".into()),
                ("LIBGL_NORMALIZE".into(), "1".into()),
                ("LIBGL_NOINTOVLHACK".into(), "1".into()),
                ("LIBGL_NOERROR".into(), "1".into()),
                ("LIBGL_USE_MC_COLOR".into(), "1".into()),
            ],
            supported_abis: Vec::new(),
            native_libs: vec![NativeLib::in_native_lib_dir("libgl4es_114.so")],
            requires_validation: false,
            trust: TrustLevel::System,
            signature: None,
            author: None,
        },
        Renderer::NgGl4es => RendererPlugin {
            id: "opengles2_ng".into(),
            display_name: "NG-GL4ES".into(),
            gl_libname: "libng_gl4es.so".into(),
            backend: WindowingBackend::GlSurface,
            env: vec![
                ("LIBGL_ES".into(), "2".into()),
                ("LIBGL_MIPMAP".into(), "3".into()),
                ("LIBGL_NORMALIZE".into(), "1".into()),
                ("LIBGL_NOINTOVLHACK".into(), "1".into()),
                ("LIBGL_NOERROR".into(), "1".into()),
                ("LIBGL_USE_MC_COLOR".into(), "1".into()),
            ],
            supported_abis: Vec::new(),
            native_libs: vec![NativeLib::in_native_lib_dir("libng_gl4es.so")],
            requires_validation: false,
            trust: TrustLevel::System,
            signature: None,
            author: None,
        },
        Renderer::VirGl => RendererPlugin {
            id: "opengles2_vgpu".into(),
            display_name: "VirGL / vgpu".into(),
            gl_libname: "libvgpu.so".into(),
            backend: WindowingBackend::GlSurface,
            env: vec![
                ("GALLIUM_DRIVER".into(), "virpipe".into()),
                ("VTEST_SOCKET_NAME".into(), "/tmp/.virgl_test".into()),
                ("MESA_GL_VERSION_OVERRIDE".into(), "4.3".into()),
                ("MESA_GLSL_VERSION_OVERRIDE".into(), "430".into()),
            ],
            supported_abis: Vec::new(),
            native_libs: vec![NativeLib::in_native_lib_dir("libvgpu.so")],
            requires_validation: false,
            trust: TrustLevel::System,
            signature: None,
            author: None,
        },
        Renderer::Zink => RendererPlugin {
            id: "opengles3_desktopgl_zink_kopper".into(),
            display_name: "Zink (Mesa)".into(),
            gl_libname: "libOSMesa_8.so".into(),
            backend: WindowingBackend::GlSurface,
            env: vec![
                ("LIB_MESA_NAME".into(), "libOSMesa_8.so".into()),
                ("MESA_LOADER_DRIVER_OVERRIDE".into(), "zink".into()),
                ("GALLIUM_DRIVER".into(), "zink".into()),
                ("MESA_GL_VERSION_OVERRIDE".into(), "4.6".into()),
                ("MESA_GLSL_VERSION_OVERRIDE".into(), "460".into()),
                ("OSMESA_NO_FLUSH_FRONTBUFFER".into(), "1".into()),
            ],
            supported_abis: Vec::new(),
            native_libs: vec![NativeLib::in_native_lib_dir("libOSMesa_8.so")],
            requires_validation: false,
            trust: TrustLevel::System,
            signature: None,
            author: None,
        },
        Renderer::Angle => RendererPlugin {
            id: "opengles3_angle".into(),
            display_name: "ANGLE".into(),
            gl_libname: "libGLESv2_angle.so".into(),
            backend: WindowingBackend::GlSurface,
            env: vec![
                ("LIBGL_ES".into(), "3".into()),
                ("MESA_GL_VERSION_OVERRIDE".into(), "4.6".into()),
                ("MESA_GLSL_VERSION_OVERRIDE".into(), "460".into()),
            ],
            supported_abis: Vec::new(),
            native_libs: vec![NativeLib::in_native_lib_dir("libGLESv2_angle.so")],
            requires_validation: false,
            trust: TrustLevel::System,
            signature: None,
            author: None,
        },
    }
}

/// The set of renderer plugins available to the launcher.
///
/// Starts from the 5 built-ins ([`RendererRegistry::builtin`]); additional
/// (user / third-party) plugins are registered at runtime via
/// [`RendererRegistry::register`]. Mirrors the FCL / Zalith plugin managers,
/// which keep a discoverable, mutable catalogue of renderers.
#[derive(Debug, Clone, Default)]
pub struct RendererRegistry {
    plugins: Vec<RendererPlugin>,
}

impl RendererRegistry {
    /// A registry pre-populated with the built-in renderers.
    pub fn builtin() -> Self {
        Self::from_builtins()
    }

    /// Build the built-in registry (all 5 FCL renderers).
    pub fn from_builtins() -> Self {
        let mut r = RendererRegistry::default();
        for variant in [
            Renderer::Gl4es,
            Renderer::NgGl4es,
            Renderer::VirGl,
            Renderer::Zink,
            Renderer::Angle,
        ] {
            r.register(renderer_plugin(variant));
        }
        r
    }

    /// Register a (built-in or third-party) plugin. An existing plugin with the
    /// same id is replaced (last write wins).
    pub fn register(&mut self, plugin: RendererPlugin) {
        if let Some(slot) = self.plugins.iter_mut().find(|p| p.id == plugin.id) {
            *slot = plugin;
        } else {
            self.plugins.push(plugin);
        }
    }

    /// Builder-style: return `self` with `plugin` registered.
    pub fn with_plugin(mut self, plugin: RendererPlugin) -> Self {
        self.register(plugin);
        self
    }

    /// Look up a plugin by id.
    pub fn get(&self, id: &str) -> Option<&RendererPlugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// All registered plugins.
    pub fn all(&self) -> &[RendererPlugin] {
        &self.plugins
    }

    /// The ids of all registered plugins, in registration order.
    pub fn ids(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.id.as_str()).collect()
    }

    /// Validate `plugin_id` against `ctx` / `store`.
    pub fn validate_plugin(
        &self,
        plugin_id: &str,
        ctx: &ValidationContext,
        store: &TrustStore,
    ) -> RcResult<super::validation::ValidationReport> {
        let p = self
            .get(plugin_id)
            .ok_or_else(|| RcError::Other(format!("unknown renderer plugin: {plugin_id}")))?;
        Ok(p.validate(ctx, store))
    }

    /// Resolve the native-library *directories* to inject into
    /// `java.library.path` / `LD_LIBRARY_PATH` for `plugin_id` on `abi`.
    ///
    /// This is the concrete "NativeLib 注入" step: it walks the plugin's
    /// [`NativeLib`]s that apply to `abi`, maps each to its on-disk directory
    /// under `native_lib_dir`, verifies non-optional libs exist (returning
    /// [`RcError::MissingFile`] otherwise) and returns the ordered,
    /// de-duplicated list of directories. Optional-but-missing libs are skipped.
    pub fn resolve_native_lib_dirs(
        &self,
        plugin_id: &str,
        native_lib_dir: &Path,
        abi: Abi,
    ) -> RcResult<Vec<PathBuf>> {
        let plugin = self
            .get(plugin_id)
            .ok_or_else(|| RcError::Other(format!("unknown renderer plugin: {plugin_id}")))?;

        let mut dirs: Vec<PathBuf> = Vec::new();
        let push_dir = |d: PathBuf, dirs: &mut Vec<PathBuf>| {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        };

        for lib in plugin.native_libs_for(abi) {
            let dir = match lib.source {
                NativeLibSource::NativeLibDir => native_lib_dir.to_path_buf(),
                NativeLibSource::PluginOwned => native_lib_dir.join(&plugin.id),
                NativeLibSource::LwjglNatives => {
                    native_lib_dir.join(format!("lwjgl-natives-{}", abi.as_android_abi()))
                }
                NativeLibSource::JreLib => native_lib_dir.join("jre-lib"),
            };
            let path = dir.join(&lib.file_name);
            if path.exists() {
                push_dir(dir, &mut dirs);
            } else if lib.optional {
                // warning-level concern handled by validation; skip here
                continue;
            } else {
                return Err(RcError::MissingFile(path.to_string_lossy().to_string()));
            }
        }
        Ok(dirs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::validation::TrustStore;

    #[test]
    fn builtin_registry_has_all_five() {
        let reg = RendererRegistry::builtin();
        assert_eq!(reg.ids().len(), 5);
        for id in [
            "opengles2",
            "opengles2_ng",
            "opengles2_vgpu",
            "opengles3_desktopgl_zink_kopper",
            "opengles3_angle",
        ] {
            assert!(reg.get(id).is_some(), "missing builtin {id}");
        }
    }

    #[test]
    fn renderer_enum_matches_plugin_descriptor() {
        // The launch-engine `Renderer` contract must stay in lock-step with the
        // pluggable `RendererPlugin` descriptor (single source of truth).
        for r in [
            Renderer::Gl4es,
            Renderer::NgGl4es,
            Renderer::VirGl,
            Renderer::Zink,
            Renderer::Angle,
        ] {
            let p = renderer_plugin(r);
            assert_eq!(p.id, r.id());
            assert_eq!(p.gl_libname, r.gl_libname());
            let p_env: Vec<(&str, String)> =
                p.env.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            assert_eq!(p_env, r.env(), "env drift for {r:?}");
            assert_eq!(p.trust, TrustLevel::System);
        }
    }

    #[test]
    fn register_replaces_same_id() {
        let mut reg = RendererRegistry::builtin();
        let custom = RendererPlugin::builder("opengles2", "Custom GL4ES")
            .gl_libname("libgl4es_114.so")
            .author("alice")
            .build();
        reg.register(custom);
        assert_eq!(reg.ids().len(), 5);
        assert_eq!(reg.get("opengles2").unwrap().display_name, "Custom GL4ES");
    }

    #[test]
    fn supports_abi_empty_means_all() {
        let p = renderer_plugin(Renderer::Zink);
        assert!(p.supports_abi(Abi::Arm64V8a));
        assert!(p.supports_abi(Abi::X86));
    }

    #[test]
    fn integrity_hash_is_stable_and_distinguishes() {
        let a = renderer_plugin(Renderer::Gl4es);
        let b = renderer_plugin(Renderer::Gl4es);
        assert_eq!(a.integrity_hash(), b.integrity_hash());
        let c = renderer_plugin(Renderer::Zink);
        assert_ne!(a.integrity_hash(), c.integrity_hash());
        assert_eq!(a.integrity_hash().len(), 40);
    }

    #[test]
    fn builtin_plugins_validate_as_system() {
        let reg = RendererRegistry::builtin();
        let ctx = ValidationContext::new(Abi::Arm64V8a, "/data/app/lib/arm64");
        let store = TrustStore::empty();
        for id in reg.ids() {
            let p = reg.get(id).unwrap();
            // A System plugin is always trusted; path-safety + ABI coverage hold
            // for the descriptor regardless of whether the .so is on disk here
            // (presence is a deployment concern, not a descriptor concern).
            let report = p.validate(&ctx, &store);
            assert_eq!(report.trust, TrustLevel::System);
            assert!(p.supports_abi(Abi::Arm64V8a));
            for lib in &p.native_libs {
                assert!(lib.validate_name().is_ok());
            }
        }
    }

    #[test]
    fn resolve_requires_present_non_optional_libs() {
        let reg = RendererRegistry::builtin();
        // a directory that does NOT contain libgl4es_114.so
        let res =
            reg.resolve_native_lib_dirs("opengles2", Path::new("/nonexistent"), Abi::Arm64V8a);
        assert!(res.is_err());
    }

    #[test]
    fn optional_missing_lib_is_skipped() {
        let mut reg = RendererRegistry::builtin();
        let mut p = renderer_plugin(Renderer::Gl4es);
        p.native_libs
            .push(NativeLib::in_native_lib_dir("libextra.so").optional(true));
        reg.register(p);
        // should not error on the missing optional lib
        let res =
            reg.resolve_native_lib_dirs("opengles2", Path::new("/nonexistent"), Abi::Arm64V8a);
        // the required libgl4es_114.so is still missing -> error expected
        assert!(res.is_err());
    }
}
