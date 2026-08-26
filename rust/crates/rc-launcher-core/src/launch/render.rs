//! Render integration: LWJGL + GL4ES / ANGLE on Android (task 17).
//!
//! Minecraft ships a *desktop* LWJGL that links against desktop OpenGL. On
//! Android there is no desktop OpenGL — only OpenGL ES exposed by the system
//! driver, or by a Vulkan-backed ANGLE. This module wires the three pieces FCL
//! / Zalith bundle together so the game gets real graphics output:
//!
//! 1. **LWJGL natives** — `app_runtime/lwjgl/<ver>/natives/<abi>/*.so`
//!    (`liblwjgl.so`, `liblwjgl_opengl.so`, …). These replace the desktop
//!    natives the vanilla `version.json` would otherwise load (see the classpath
//!    substitution in task 7). [`LwjglNativeBundle`] *validates* them on disk.
//! 2. **GL4ES / ANGLE** — a translation layer that turns OpenGL into OpenGL ES.
//!    GL4ES (`libgl4es_114.so`) is `dlopen`ed by LWJGL's `liblwjgl_opengl.so`
//!    through `-Dorg.lwjgl.opengl.libname`; ANGLE ships `libGLESv2_angle.so` +
//!    `libEGL_angle.so` and is selected through `ANGLE_DEFAULT_PLATFORM`. The
//!    [`gl_translation_env`] helpers emit the translation-layer environment.
//! 3. **Tuning** — a [`PerfProfile`] that trades GL error-checking for throughput
//!    on weak devices, emitted through [`PerfProfile::env`].
//!
//! The launch engine (task 7) already installs the renderer's base environment
//! (`Renderer::env()`) and `-Dorg.lwjgl.opengl.libname`; this module *complements*
//! it with the GL4ES/ANGLE translation backend selection and the performance
//! profile, and *hardens* the launch with an on-disk LWJGL native-library check
//! so a missing `liblwjgl_opengl.so` fails *before* a JVM is spawned (instead of
//! dying as an opaque `UnsatisfiedLinkError`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::launch::options::{LwjglVersion, Renderer};
use crate::launch::runtime_assets::AppRuntime;
use crate::runtime::Abi;

/// A native library an LWJGL bundle is expected to ship (arm64-v8a, taken from
/// `FCL_APK_RUNTIME_ASSETS_CATALOG.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LwjglNativeLib {
    /// File name inside `natives/<abi>/`.
    pub file_name: &'static str,
    /// `true` for the libs the game *cannot* start without; `false` for the
    /// optional extras (stb, nanovg, tinyfd, vma, freetype, shaderc, spng).
    pub required: bool,
}

/// The core LWJGL 3.3.3 native libraries (FCL `lwjgl/3.3.3/natives/arm64-v8a`).
///
/// `liblwjgl.so` is the core runtime; `liblwjgl_opengl.so` is the OpenGL binding
/// LWJGL `dlopen`s (and which GL4ES/ANGLE satisfy). Both are required — without
/// `liblwjgl_opengl.so` the game dies with `UnsatisfiedLinkError` at first GL call.
pub const LWJGL_3_3_3_NATIVES: &[LwjglNativeLib] = &[
    LwjglNativeLib {
        file_name: "liblwjgl.so",
        required: true,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_opengl.so",
        required: true,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_stb.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_nanovg.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_tinyfd.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_vma.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "libfreetype.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "libshaderc.so",
        required: false,
    },
];

/// LWJGL 3.4.1 ships the same set plus `liblwjgl_spng.so` (PNG codec backend).
pub const LWJGL_3_4_1_NATIVES: &[LwjglNativeLib] = &[
    LwjglNativeLib {
        file_name: "liblwjgl.so",
        required: true,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_opengl.so",
        required: true,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_stb.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_nanovg.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_tinyfd.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_vma.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "liblwjgl_spng.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "libfreetype.so",
        required: false,
    },
    LwjglNativeLib {
        file_name: "libshaderc.so",
        required: false,
    },
];

/// The expected native-library manifest for an LWJGL bundle version.
pub fn lwjgl_native_manifest(version: LwjglVersion) -> &'static [LwjglNativeLib] {
    match version {
        LwjglVersion::V3_3_3 => LWJGL_3_3_3_NATIVES,
        LwjglVersion::V3_4_1 => LWJGL_3_4_1_NATIVES,
    }
}

/// A discovered, validated LWJGL native bundle on disk.
///
/// Produced by [`LwjglNativeBundle::discover`] from an [`AppRuntime`]. The launch
/// engine preflights with it so a missing core native fails *before* a JVM is
/// spawned (task 17: "integrate the LWJGL 3.3.x native libraries").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwjglNativeBundle {
    /// The bundle version that was scanned.
    pub version: LwjglVersion,
    /// The ABI the directory is laid out for.
    pub abi: Abi,
    /// `app_runtime/lwjgl/<ver>/natives/<abi>/`.
    pub natives_dir: PathBuf,
    /// Every expected library that is actually present on disk.
    pub present: Vec<LwjglNativeLib>,
    /// Expected libraries that are missing (optional ones included).
    pub missing: Vec<LwjglNativeLib>,
}

impl LwjglNativeBundle {
    /// Scan `app_runtime/lwjgl/<ver>/natives/<abi>/` and classify every expected
    /// native as present or missing. Never errors — the caller decides what the
    /// missing set means via [`LwjglNativeBundle::is_complete`].
    pub fn scan(app_runtime: &AppRuntime, version: LwjglVersion, abi: Abi) -> Self {
        let natives_dir = app_runtime.lwjgl_natives_dir(version, abi);
        let manifest = lwjgl_native_manifest(version);
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for lib in manifest {
            if natives_dir.join(lib.file_name).is_file() {
                present.push(*lib);
            } else {
                missing.push(*lib);
            }
        }
        Self {
            version,
            abi,
            natives_dir,
            present,
            missing,
        }
    }

    /// Discover & validate: missing *required* libs become an [`RcError::MissingFile`]
    /// that lists every absent required native, so the UI can tell the user to
    /// re-extract the LWJGL bundle rather than letting Minecraft crash.
    pub fn discover(app_runtime: &AppRuntime, version: LwjglVersion, abi: Abi) -> RcResult<Self> {
        let bundle = Self::scan(app_runtime, version, abi);
        let missing_required: Vec<&str> = bundle.missing_required();
        if !missing_required.is_empty() {
            return Err(RcError::MissingFile(format!(
                "LWJGL {} natives for {} are incomplete in {}; missing required: {}",
                bundle.version.as_dir(),
                bundle.abi,
                bundle.natives_dir.display(),
                missing_required.join(", ")
            )));
        }
        Ok(bundle)
    }

    /// `true` when every required native lib is present on disk.
    pub fn is_complete(&self) -> bool {
        self.missing.iter().all(|l| !l.required)
    }

    /// Only the required libraries that are missing.
    pub fn missing_required(&self) -> Vec<&'static str> {
        self.missing
            .iter()
            .filter(|l| l.required)
            .map(|l| l.file_name)
            .collect()
    }

    /// Absolute paths of every native that is present (for `nativeLibraryDir`
    /// assembly / diagnostics).
    pub fn present_paths(&self) -> Vec<PathBuf> {
        self.present
            .iter()
            .map(|l| self.natives_dir.join(l.file_name))
            .collect()
    }
}

/// A native library a renderer's OpenGL→OpenGL ES translation backend needs on
/// disk (task 17 robustness: the LWJGL natives are only half the story — without
/// the GL4ES / ANGLE / Mesa / Zink backing `.so`, the chosen renderer dies at
/// the first GL call with an opaque `UnsatisfiedLinkError` or `EGL_BAD_CONFIG`,
/// long after the JVM has booted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RendererNativeLib {
    /// File name inside the renderer's `nativeLibraryDir`.
    pub file_name: &'static str,
    /// `true` for the libs the renderer *cannot* initialise without.
    pub required: bool,
}

/// The backing native libraries for a renderer's translation stack, taken from
/// `FCL_NATIVE_LIBRARIES.md` (the 30 `.so` files FCL bundles) and matched to
/// each [`Renderer`]'s `gl_libname()` (the library LWJGL `dlopen`s).
///
/// * GL4ES / NG-GL4ES `dlopen` `libgl4es_114.so` / `libng_gl4es.so` and dispatch
///   through `libglapi.so` (optional on devices where it is statically folded in).
/// * ANGLE needs `libGLESv2_angle.so` (the `gl_libname`) **and** `libEGL_angle.so`
///   for the EGL context — both are required.
/// * Zink (`libOSMesa_8.so`) cannot rasterise without the `libzink_dri.so` DRI
///   driver, so both are required.
/// * VirGL / SDL need only their single `gl_libname`.
pub fn renderer_native_manifest(renderer: Renderer) -> &'static [RendererNativeLib] {
    match renderer {
        Renderer::Gl4es => &[
            RendererNativeLib {
                file_name: "libgl4es_114.so",
                required: true,
            },
            RendererNativeLib {
                file_name: "libglapi.so",
                required: false,
            },
        ],
        Renderer::NgGl4es => &[
            RendererNativeLib {
                file_name: "libng_gl4es.so",
                required: true,
            },
            RendererNativeLib {
                file_name: "libglapi.so",
                required: false,
            },
        ],
        Renderer::VirGl => &[RendererNativeLib {
            file_name: "libvgpu.so",
            required: true,
        }],
        Renderer::Zink => &[
            RendererNativeLib {
                file_name: "libOSMesa_8.so",
                required: true,
            },
            RendererNativeLib {
                file_name: "libzink_dri.so",
                required: true,
            },
        ],
        Renderer::Angle => &[
            RendererNativeLib {
                file_name: "libGLESv2_angle.so",
                required: true,
            },
            RendererNativeLib {
                file_name: "libEGL_angle.so",
                required: true,
            },
        ],
        Renderer::Sdl => &[RendererNativeLib {
            file_name: "liblwjgl_sdl.so",
            required: true,
        }],
    }
}

/// A discovered, validated set of renderer backing libraries on disk.
///
/// Produced by [`RendererNativeBundle::discover`]. The launch engine preflights
/// with it so a missing `libGLESv2_angle.so` / `libzink_dri.so` fails *before*
/// the JVM is spawned (completing the task-17 preflight that the LWJGL-native
/// check started).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererNativeBundle {
    /// The renderer whose backing libs were scanned.
    pub renderer: Renderer,
    /// The dir that was scanned (`nativeLibraryDir`), if any.
    pub native_lib_dir: Option<PathBuf>,
    /// Every expected backing lib that is actually present on disk.
    pub present: Vec<RendererNativeLib>,
    /// Expected backing libs that are missing.
    pub missing: Vec<RendererNativeLib>,
}

impl RendererNativeBundle {
    /// Scan the renderer's `nativeLibraryDir` and classify every expected backing
    /// lib as present or missing. Never errors — the caller decides what the
    /// missing set means via [`RendererNativeBundle::is_complete`] /
    /// [`RendererNativeBundle::discover`].
    pub fn scan(native_lib_dir: Option<&Path>, renderer: Renderer) -> Self {
        let manifest = renderer_native_manifest(renderer);
        let mut present = Vec::new();
        let mut missing = Vec::new();
        if let Some(dir) = native_lib_dir {
            for lib in manifest {
                if dir.join(lib.file_name).is_file() {
                    present.push(*lib);
                } else {
                    missing.push(*lib);
                }
            }
        } else {
            // No dir to look in: everything is "missing" from our point of view.
            missing.extend_from_slice(manifest);
        }
        Self {
            renderer,
            native_lib_dir: native_lib_dir.map(|p| p.to_path_buf()),
            present,
            missing,
        }
    }

    /// Discover & validate: missing *required* backing libs become an
    /// [`RcError::MissingFile`] that lists every absent required native.
    ///
    /// Validation is *best-effort*: when `native_lib_dir` is `None` or does not
    /// exist on disk we cannot check the libs (they may still resolve from the
    /// APK's own `lib/` dir at runtime), so we accept the scan instead of
    /// failing. Only an existing directory that is missing required libs errors.
    pub fn discover(native_lib_dir: Option<&Path>, renderer: Renderer) -> RcResult<Self> {
        let bundle = Self::scan(native_lib_dir, renderer);
        match native_lib_dir {
            None => Ok(bundle),
            Some(dir) => {
                if !dir.is_dir() {
                    return Ok(bundle);
                }
                let missing_required: Vec<&str> = bundle.missing_required();
                if !missing_required.is_empty() {
                    return Err(RcError::MissingFile(format!(
                        "renderer {} backing libraries are incomplete in {}; missing required: {}",
                        bundle.renderer.id(),
                        dir.display(),
                        missing_required.join(", ")
                    )));
                }
                Ok(bundle)
            }
        }
    }

    /// `true` when every required backing lib is present on disk.
    pub fn is_complete(&self) -> bool {
        self.missing.iter().all(|l| !l.required)
    }

    /// Only the required backing libs that are missing.
    pub fn missing_required(&self) -> Vec<&'static str> {
        self.missing
            .iter()
            .filter(|l| l.required)
            .map(|l| l.file_name)
            .collect()
    }

    /// Absolute paths of every backing lib that is present (diagnostics).
    pub fn present_paths(&self) -> Vec<PathBuf> {
        match &self.native_lib_dir {
            Some(dir) => self.present.iter().map(|l| dir.join(l.file_name)).collect(),
            None => Vec::new(),
        }
    }
}
/// Graphics performance profile (task 17 perf tuning).
///
/// Cheap devices cannot afford desktop-GL error checking; turning it off trades
/// strict conformance for throughput. The profile selects the `LIBGL_*` / Mesa
/// knobs [`PerfProfile::env`] emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerfProfile {
    /// Safe defaults: error checking on, no forced knobs. Best for debugging.
    Diagnostic,
    /// Balanced — the default. Keeps GL error checking, lets the driver decide
    /// vsync / indirection.
    #[default]
    Balanced,
    /// Low power: disable GL error checking to save battery on weak GPUs.
    LowPower,
    /// High performance: no error checking, no indirect-rendering overhead.
    HighPerformance,
    /// Maximum: every throughput knob on (no error checking, no indirection,
    /// on-screen FPS).
    Maximum,
}

impl PerfProfile {
    /// The tuning environment variables for this profile.
    ///
    /// These are additive on top of the renderer's base environment
    /// (`Renderer::env()`) and the translation layer (`gl_translation_env`); they
    /// only ever *disable* expensive checks, so they are safe to apply last.
    pub fn env(self) -> Vec<(String, String)> {
        match self {
            PerfProfile::Diagnostic => Vec::new(),
            PerfProfile::Balanced => Vec::new(),
            PerfProfile::LowPower => vec![
                ("LIBGL_NOERROR".into(), "1".into()),
                ("MESA_NO_ERROR".into(), "1".into()),
            ],
            PerfProfile::HighPerformance => vec![
                ("LIBGL_NOERROR".into(), "1".into()),
                ("MESA_NO_ERROR".into(), "1".into()),
                ("LIBGL_NOINDIRECT".into(), "1".into()),
            ],
            PerfProfile::Maximum => vec![
                ("LIBGL_NOERROR".into(), "1".into()),
                ("MESA_NO_ERROR".into(), "1".into()),
                ("LIBGL_NOINDIRECT".into(), "1".into()),
                ("LIBGL_FPS".into(), "1".into()),
            ],
        }
    }

    /// A short, human-readable label (settings UI / logs).
    pub fn label(self) -> &'static str {
        match self {
            PerfProfile::Diagnostic => "Diagnostic",
            PerfProfile::Balanced => "Balanced",
            PerfProfile::LowPower => "Low power",
            PerfProfile::HighPerformance => "High performance",
            PerfProfile::Maximum => "Maximum",
        }
    }
}

/// Composes the LWJGL native bundle + the chosen renderer into the concrete
/// native-library search dirs and environment the launch engine hands to the
/// game JVM (task 17: GL4ES/ANGLE OpenGL→OpenGL ES translation + perf tuning).
#[derive(Debug, Clone)]
pub struct RenderIntegration {
    bundle: LwjglNativeBundle,
    renderer: Renderer,
    /// The app's `nativeLibraryDir` (holds `libgl4es_114.so`, `libGLESv2_angle.so`, …).
    native_lib_dir: Option<PathBuf>,
    /// The renderer's OpenGL→OpenGL ES backing libs, scanned (never fails).
    renderer_native: RendererNativeBundle,
}

impl RenderIntegration {
    /// Build a render integration from a discovered LWJGL bundle and the renderer
    /// the user picked.
    pub fn new(
        bundle: LwjglNativeBundle,
        renderer: Renderer,
        native_lib_dir: Option<PathBuf>,
    ) -> Self {
        let renderer_native = RendererNativeBundle::scan(native_lib_dir.as_deref(), renderer);
        Self {
            bundle,
            renderer,
            native_lib_dir,
            renderer_native,
        }
    }

    /// The underlying, already-validated LWJGL native bundle.
    pub fn bundle(&self) -> &LwjglNativeBundle {
        &self.bundle
    }

    /// The scanned renderer backing-library set (see [`RendererNativeBundle`]).
    pub fn renderer_native_bundle(&self) -> &RendererNativeBundle {
        &self.renderer_native
    }

    /// `true` when the renderer's required backing libs are all present on disk.
    pub fn renderer_backing_ok(&self) -> bool {
        self.renderer_native.is_complete()
    }

    /// Preflight the renderer's OpenGL→OpenGL ES backing libraries.
    ///
    /// Returns the validated [`RendererNativeBundle`], or an
    /// [`RcError::MissingFile`] when an *existing* `nativeLibraryDir` is missing a
    /// required backing lib (e.g. `libGLESv2_angle.so`, `libzink_dri.so`). A
    /// `None` / absent dir is accepted — the libs may still resolve from the
    /// APK's own `lib/` dir at runtime (see [`RendererNativeBundle::discover`]).
    /// This completes the task-17 preflight the LWJGL-native check started.
    pub fn preflight_renderer(&self) -> RcResult<RendererNativeBundle> {
        RendererNativeBundle::discover(self.native_lib_dir.as_deref(), self.renderer)
    }

    /// Native-library search dirs, in loader order:
    /// 1. the LWJGL natives dir (core + GL bindings),
    /// 2. the renderer's `nativeLibraryDir` (GL4ES / ANGLE / OpenAL …).
    pub fn native_lib_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.bundle.natives_dir.clone()];
        if let Some(nl) = &self.native_lib_dir {
            if !dirs.contains(nl) {
                dirs.push(nl.clone());
            }
        }
        dirs
    }

    /// The OpenGL→OpenGL ES translation-layer environment *specific* to the chosen
    /// renderer, complementing the base variables from `Renderer::env()`.
    ///
    /// GL4ES's `LIBGL_ES` / `LIBGL_DRIVERS_PATH` are already set by the launch
    /// engine; here we add the backend-selection the engine does not know about:
    /// ANGLE must be told to use its Vulkan backend (`ANGLE_DEFAULT_PLATFORM`), and
    /// GL4ES gets an explicit GL extension dir. The Mesa/VirGL/Zink stacks only
    /// need their driver path, which the engine already provides.
    pub fn gl_translation_env(&self) -> Vec<(String, String)> {
        gl_translation_env(self.renderer, self.native_lib_dir.as_deref())
    }

    /// Performance-tuning environment for a [`PerfProfile`] (see [`PerfProfile::env`]).
    pub fn perf_env(&self, profile: PerfProfile) -> Vec<(String, String)> {
        profile.env()
    }

    /// Full renderer environment: translation-backend selection + perf tuning.
    /// (The renderer's own base variables from `Renderer::env()` are applied
    /// separately by the launch engine.)
    pub fn env(&self, profile: PerfProfile) -> Vec<(String, String)> {
        let mut env = self.gl_translation_env();
        env.extend(self.perf_env(profile));
        env
    }

    /// A one-line summary for the launch log header (diagnostics).
    pub fn describe(&self) -> String {
        format!(
            "renderer={} (gl_libname={}) lwjgl={} abi={} natives_ok={} renderer_backing_ok={}",
            self.renderer.id(),
            self.renderer.gl_libname(),
            self.bundle.version.as_dir(),
            self.bundle.abi,
            self.bundle.is_complete(),
            self.renderer_native.is_complete(),
        )
    }
}

/// The renderer-specific OpenGL→OpenGL ES translation environment (free function
/// form, so the launch engine can call it without a [`LwjglNativeBundle`]).
///
/// See [`RenderIntegration::gl_translation_env`] for the semantics.
pub fn gl_translation_env(
    renderer: Renderer,
    native_lib_dir: Option<&Path>,
) -> Vec<(String, String)> {
    let nl = native_lib_dir.map(|p| p.to_string_lossy().to_string());
    match renderer {
        Renderer::Angle => {
            // ANGLE renders GL through Vulkan; select the backend explicitly so it
            // never falls back to the (absent) desktop GL, and skip validation to
            // match FCL/Zalith's shipped ANGLE builds.
            vec![
                ("ANGLE_DEFAULT_PLATFORM".into(), "vulkan".into()),
                ("ANGLE_NO_VALIDATION".into(), "1".into()),
            ]
        }
        Renderer::Gl4es | Renderer::NgGl4es => {
            // GL4ES already gets LIBGL_ES / LIBGL_DRIVERS_PATH from the engine; add
            // the GL extension table location when a native dir is known.
            let mut env = Vec::new();
            if let Some(nl) = nl {
                env.push(("LIBGL_GLEXT".into(), nl));
            }
            env
        }
        Renderer::VirGl | Renderer::Zink => {
            // Mesa Gallium (virgl/zink) loads its driver .so from nativeLibraryDir;
            // the engine already sets LIBGL_DRIVERS_PATH, so nothing extra here.
            Vec::new()
        }
        Renderer::Sdl => {
            // SDL2 backend (LWJGL 3.4.1) renders to an Android surface; the
            // engine already wires `java.library.path` to the LWJGL natives dir
            // where `liblwjgl_sdl.so` lives, so no extra GL/Gallium env is needed.
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::runtime_assets::AppRuntime;

    fn fake_app_runtime() -> (tempfile::TempDir, AppRuntime) {
        let td = tempfile::tempdir().unwrap();
        let rt = AppRuntime::new(td.path());
        for v in [LwjglVersion::V3_3_3, LwjglVersion::V3_4_1] {
            let dir = rt.lwjgl_natives_dir(v, Abi::Arm64V8a);
            std::fs::create_dir_all(&dir).unwrap();
            // write every *required* native so the bundle is complete
            for lib in lwjgl_native_manifest(v) {
                if lib.required {
                    std::fs::write(dir.join(lib.file_name), b"so").unwrap();
                }
            }
        }
        (td, rt)
    }

    #[test]
    fn manifest_distinguishes_required_and_optional() {
        assert!(LWJGL_3_3_3_NATIVES.iter().filter(|l| l.required).count() == 2);
        // 3.4.1 adds spng but keeps the same two required cores
        assert!(LWJGL_3_4_1_NATIVES.iter().filter(|l| l.required).count() == 2);
        assert!(lwjgl_native_manifest(LwjglVersion::V3_4_1)
            .iter()
            .any(|l| l.file_name == "liblwjgl_spng.so"));
    }

    #[test]
    fn scan_classifies_present_and_missing() {
        let (_td, rt) = fake_app_runtime();
        let b = LwjglNativeBundle::scan(&rt, LwjglVersion::V3_3_3, Abi::Arm64V8a);
        // required libs present -> complete
        assert!(b.is_complete());
        // optional libs are *not* written by the fixture, so they show as missing
        assert!(!b.missing.is_empty());
        assert!(b.missing.iter().any(|l| l.file_name == "liblwjgl_stb.so"));
        assert!(b
            .present
            .iter()
            .any(|l| l.file_name == "liblwjgl_opengl.so"));
    }

    #[test]
    fn discover_accepts_complete_bundle() {
        let (_td, rt) = fake_app_runtime();
        let b = LwjglNativeBundle::discover(&rt, LwjglVersion::V3_3_3, Abi::Arm64V8a).unwrap();
        assert!(b.is_complete());
        assert!(b.missing_required().is_empty());
    }

    #[test]
    fn discover_rejects_missing_required_native() {
        let td = tempfile::tempdir().unwrap();
        let rt = AppRuntime::new(td.path());
        let dir = rt.lwjgl_natives_dir(LwjglVersion::V3_3_3, Abi::Arm64V8a);
        std::fs::create_dir_all(&dir).unwrap();
        // Only the core is present; the GL binding (.so) is missing.
        std::fs::write(dir.join("liblwjgl.so"), b"so").unwrap();
        let err =
            LwjglNativeBundle::discover(&rt, LwjglVersion::V3_3_3, Abi::Arm64V8a).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("incomplete"), "{msg}");
        assert!(msg.contains("liblwjgl_opengl.so"), "{msg}");
    }

    #[test]
    fn discover_is_fine_with_missing_optional_native() {
        let (_td, rt) = fake_app_runtime();
        // write an optional native, then remove it to simulate an *optional* lib
        // being absent: discover must still succeed (only required libs block).
        let dir = rt.lwjgl_natives_dir(LwjglVersion::V3_3_3, Abi::Arm64V8a);
        std::fs::write(dir.join("liblwjgl_stb.so"), b"so").unwrap();
        std::fs::remove_file(dir.join("liblwjgl_stb.so")).unwrap();
        let b = LwjglNativeBundle::discover(&rt, LwjglVersion::V3_3_3, Abi::Arm64V8a).unwrap();
        assert!(b.is_complete());
        assert!(b.missing.iter().any(|l| l.file_name == "liblwjgl_stb.so"));
    }

    #[test]
    fn perf_profile_balanced_is_silent() {
        assert!(PerfProfile::Balanced.env().is_empty());
        assert!(PerfProfile::Diagnostic.env().is_empty());
        // higher tiers disable error checking (the big GL4ES/Mesa CPU sink)
        let hp: Vec<String> = PerfProfile::HighPerformance
            .env()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(hp.contains(&"LIBGL_NOERROR".to_string()));
        assert!(hp.contains(&"MESA_NO_ERROR".to_string()));
        assert!(hp.contains(&"LIBGL_NOINDIRECT".to_string()));
        let max: Vec<String> = PerfProfile::Maximum
            .env()
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert!(max.contains(&"LIBGL_FPS".to_string()));
    }

    #[test]
    fn perf_profile_serde_roundtrip() {
        for p in [
            PerfProfile::Diagnostic,
            PerfProfile::Balanced,
            PerfProfile::LowPower,
            PerfProfile::HighPerformance,
            PerfProfile::Maximum,
        ] {
            let j = serde_json::to_string(&p).unwrap();
            let back: PerfProfile = serde_json::from_str(&j).unwrap();
            assert_eq!(p, back);
        }
        // default is Balanced
        let def: PerfProfile = serde_json::from_str("\"balanced\"").unwrap();
        assert_eq!(def, PerfProfile::Balanced);
    }

    #[test]
    fn gl_translation_selects_angle_vulkan() {
        let env: Vec<(String, String)> = gl_translation_env(Renderer::Angle, None);
        let map: std::collections::BTreeMap<_, _> = env.into_iter().collect();
        assert_eq!(
            map.get("ANGLE_DEFAULT_PLATFORM").map(|s| s.as_str()),
            Some("vulkan")
        );
        assert_eq!(
            map.get("ANGLE_NO_VALIDATION").map(|s| s.as_str()),
            Some("1")
        );
    }

    #[test]
    fn gl_translation_gl4es_glext_points_at_native_dir() {
        let env = gl_translation_env(Renderer::Gl4es, Some(std::path::Path::new("/nat")));
        let map: std::collections::BTreeMap<_, _> = env.into_iter().collect();
        assert_eq!(map.get("LIBGL_GLEXT").map(|s| s.as_str()), Some("/nat"));
        // without a native dir, GL4ES adds nothing extra (engine sets the rest)
        assert!(gl_translation_env(Renderer::Gl4es, None).is_empty());
    }

    #[test]
    fn gl_translation_mesa_adds_nothing_extra() {
        assert!(gl_translation_env(Renderer::Zink, Some(std::path::Path::new("/n"))).is_empty());
        assert!(gl_translation_env(Renderer::VirGl, None).is_empty());
    }

    #[test]
    fn integration_composes_native_dirs_and_env() {
        let (_td, rt) = fake_app_runtime();
        let bundle = LwjglNativeBundle::discover(&rt, LwjglVersion::V3_3_3, Abi::Arm64V8a).unwrap();
        let ri = RenderIntegration::new(
            bundle,
            Renderer::Angle,
            Some(std::path::Path::new("/data/app/lib/arm64").to_path_buf()),
        );
        // LWJGL natives dir first, then the renderer nativeLibraryDir
        let dirs = ri.native_lib_dirs();
        assert_eq!(
            dirs[0],
            rt.lwjgl_natives_dir(LwjglVersion::V3_3_3, Abi::Arm64V8a)
        );
        assert_eq!(dirs[1], std::path::Path::new("/data/app/lib/arm64"));
        // env carries the ANGLE backend + the perf profile
        let full: Vec<(String, String)> = ri.env(PerfProfile::Maximum);
        let keys: Vec<&String> = full.iter().map(|(k, _)| k).collect();
        assert!(keys.iter().any(|k| k.as_str() == "ANGLE_DEFAULT_PLATFORM"));
        assert!(keys.iter().any(|k| k.as_str() == "LIBGL_NOERROR"));
        assert!(keys.iter().any(|k| k.as_str() == "LIBGL_FPS"));
        assert!(ri.describe().contains("renderer=opengles3_angle"));
    }
    #[test]
    fn renderer_native_manifest_groups_required_and_optional() {
        // GL4ES: gl_libname required, glapi optional
        let g = renderer_native_manifest(Renderer::Gl4es);
        assert_eq!(g.iter().filter(|l| l.required).count(), 1);
        assert!(g
            .iter()
            .any(|l| l.file_name == "libgl4es_114.so" && l.required));
        assert!(g
            .iter()
            .any(|l| l.file_name == "libglapi.so" && !l.required));
        // ANGLE needs both GLESv2 + EGL
        let a = renderer_native_manifest(Renderer::Angle);
        assert!(a.iter().all(|l| l.required));
        assert!(a.iter().any(|l| l.file_name == "libGLESv2_angle.so"));
        assert!(a.iter().any(|l| l.file_name == "libEGL_angle.so"));
        // Zink needs the OSMesa lib + the zink DRI driver
        let z = renderer_native_manifest(Renderer::Zink);
        assert_eq!(z.len(), 2);
        assert!(z.iter().all(|l| l.required));
        assert!(z.iter().any(|l| l.file_name == "libOSMesa_8.so"));
        assert!(z.iter().any(|l| l.file_name == "libzink_dri.so"));
        // SDL's libname is the lwjgl sdl binding
        assert!(renderer_native_manifest(Renderer::Sdl)
            .iter()
            .any(|l| l.file_name == "liblwjgl_sdl.so"));
    }

    #[test]
    fn renderer_native_scan_classifies_present_and_missing() {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path();
        // only one of the two required ANGLE libs present
        std::fs::write(dir.join("libGLESv2_angle.so"), b"so").unwrap();
        let b = RendererNativeBundle::scan(Some(dir), Renderer::Angle);
        assert!(!b.is_complete());
        assert!(b
            .present
            .iter()
            .any(|l| l.file_name == "libGLESv2_angle.so"));
        assert!(b.missing.iter().any(|l| l.file_name == "libEGL_angle.so"));
    }

    #[test]
    fn renderer_native_discover_accepts_unknown_dir() {
        // None → cannot validate → accepted (incomplete but no error)
        let b = RendererNativeBundle::discover(None, Renderer::Angle).unwrap();
        assert!(!b.is_complete());
        // non-existent dir → treated like unknown → accepted
        let b2 = RendererNativeBundle::discover(
            Some(std::path::Path::new("/no/such/dir/rc-launcher")),
            Renderer::Angle,
        )
        .unwrap();
        assert!(!b2.is_complete());
    }

    #[test]
    fn renderer_native_discover_rejects_missing_required() {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path();
        // GL4ES dir missing its required libgl4es_114.so (only the optional lib present)
        std::fs::write(dir.join("libglapi.so"), b"so").unwrap();
        let err = RendererNativeBundle::discover(Some(dir), Renderer::Gl4es).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("incomplete"), "{msg}");
        assert!(msg.contains("libgl4es_114.so"), "{msg}");
        assert!(
            !msg.contains("libglapi.so"),
            "optional libs must not be reported: {msg}"
        );
    }

    #[test]
    fn renderer_native_discover_accepts_optional_missing() {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path();
        // only the required gl4es lib; optional glapi absent
        std::fs::write(dir.join("libgl4es_114.so"), b"so").unwrap();
        let b = RendererNativeBundle::discover(Some(dir), Renderer::Gl4es).unwrap();
        assert!(b.is_complete());
        assert!(b.missing.iter().any(|l| l.file_name == "libglapi.so"));
    }

    #[test]
    fn render_integration_preflight_rejects_missing_backing() {
        let td = tempfile::tempdir().unwrap();
        let rt = AppRuntime::new(td.path());
        let dir = rt.lwjgl_natives_dir(LwjglVersion::V3_3_3, Abi::Arm64V8a);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("liblwjgl.so"), b"so").unwrap();
        std::fs::write(dir.join("liblwjgl_opengl.so"), b"so").unwrap();
        let bundle = LwjglNativeBundle::discover(&rt, LwjglVersion::V3_3_3, Abi::Arm64V8a).unwrap();
        // ANGLE selected and its backing dir *exists* but is missing
        // libGLESv2_angle.so — an existing, incomplete dir must be rejected.
        let ndir = td.path().join("nativeLibraryDir");
        std::fs::create_dir_all(&ndir).unwrap();
        let ri = RenderIntegration::new(bundle, Renderer::Angle, Some(ndir));
        assert!(!ri.renderer_backing_ok());
        let err = ri.preflight_renderer().unwrap_err();
        assert!(err.to_string().contains("libGLESv2_angle.so"), "{}", err);
    }

    #[test]
    fn render_integration_preflight_accepts_present_backing() {
        let td = tempfile::tempdir().unwrap();
        let rt = AppRuntime::new(td.path());
        let dir = rt.lwjgl_natives_dir(LwjglVersion::V3_3_3, Abi::Arm64V8a);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("liblwjgl.so"), b"so").unwrap();
        std::fs::write(dir.join("liblwjgl_opengl.so"), b"so").unwrap();
        let bundle = LwjglNativeBundle::discover(&rt, LwjglVersion::V3_3_3, Abi::Arm64V8a).unwrap();
        // create the ANGLE backing dir + both required libs
        let ndir = td.path().join("nativeLibraryDir");
        std::fs::create_dir_all(&ndir).unwrap();
        std::fs::write(ndir.join("libGLESv2_angle.so"), b"so").unwrap();
        std::fs::write(ndir.join("libEGL_angle.so"), b"so").unwrap();
        let ri = RenderIntegration::new(bundle, Renderer::Angle, Some(ndir));
        assert!(ri.renderer_backing_ok());
        let b = ri.preflight_renderer().unwrap();
        assert!(b.is_complete());
        // describe() now reports renderer backing completeness (fixes the old
        // `perf=` placeholder that actually printed the LWJGL natives state)
        let d = ri.describe();
        assert!(d.contains("renderer=opengles3_angle"));
        assert!(d.contains("renderer_backing_ok=true"), "{d}");
    }
}
