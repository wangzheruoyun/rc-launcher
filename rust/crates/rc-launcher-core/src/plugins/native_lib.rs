//! Native-library injection model (task 9 — "NativeLib 注入").
//!
//! A renderer plugin contributes one or more native `.so` files — the GL
//! translation layer itself (`libgl4es_114.so`, `libOSMesa_8.so`, ...), OpenAL,
//! SDL, ... [`NativeLib`] describes each one: its file name, the ABI it
//! targets, which search root it is resolved from, its load order and whether
//! it is optional. This mirrors Zalith's `NativeLibPlugin`, which injects
//! native libraries into the running game, and FCL's renderer plugins, which
//! ship their own `jniLibs/<abi>/` natives.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::RcResult;
use crate::runtime::Abi;

/// Which search root a [`NativeLib`] is resolved from when the game JVM loads
/// it. Mirrors the `java.library.path` members the launch engine assembles in
/// [`crate::launch::env`]; a plugin declares *which* libs it needs from *which*
/// source so the loader can inject them and the validator can confirm they are
/// on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NativeLibSource {
    /// The app's `nativeLibraryDir` (renderer `.so`, OpenAL, ...).
    #[default]
    NativeLibDir,
    /// A `.so` bundled inside the plugin package itself (e.g. `jniLibs/<abi>/`).
    PluginOwned,
    /// The prebuilt LWJGL natives directory (`.../natives/<abi>/`).
    LwjglNatives,
    /// The JRE's own native `lib/` directories.
    JreLib,
}

/// Outcome of verifying a native lib on disk against its expected metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibVerify {
    /// File present and size + hash (if expected) match.
    Match,
    /// File absent.
    Missing,
    /// Present but size differs from [`NativeLib::expected_size`].
    SizeMismatch,
    /// Present, size matches, but SHA-1 differs from [`NativeLib::expected_sha1`].
    HashMismatch,
}

/// A native library a renderer plugin injects into the game's load path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeLib {
    /// File name, e.g. `libgl4es_114.so`. Must contain **no** path separator
    /// (anti-traversal guard; enforced by [`NativeLib::validate_name`]).
    pub file_name: String,
    /// ABI this lib targets; `None` means all ABIs.
    #[serde(default)]
    pub abi: Option<Abi>,
    /// Which search root the loader resolves it from.
    #[serde(default)]
    pub source: NativeLibSource,
    /// Load order in `java.library.path` / `LD_LIBRARY_PATH` (lower loads first).
    #[serde(default)]
    pub load_order: u32,
    /// If true, a missing lib is a warning, not a hard error.
    #[serde(default)]
    pub optional: bool,
    /// Expected uncompressed size in bytes (from the source APK), if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_size: Option<u64>,
    /// Expected SHA-256 hex of the lib's bytes (from the source APK), if known.
    /// Recorded for manifest / trust purposes; full SHA-256 verification needs
    /// an extra dependency, so runtime checking uses [`expected_sha1`] instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_sha256: Option<String>,
    /// Expected SHA-1 hex of the lib's bytes, if known. Verifiable at runtime
    /// with the `sha1` crate (unlike SHA-256, which needs an extra dependency).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_sha1: Option<String>,
}

impl NativeLib {
    /// A non-optional lib living in the app's `nativeLibraryDir`, for every ABI.
    pub fn in_native_lib_dir(file_name: impl Into<String>) -> Self {
        Self {
            file_name: file_name.into(),
            abi: None,
            source: NativeLibSource::NativeLibDir,
            load_order: 100,
            optional: false,
            expected_size: None,
            expected_sha256: None,
            expected_sha1: None,
        }
    }

    /// A lib bundled inside the plugin package itself (`PluginOwned`), for `abi`.
    pub fn plugin_owned(file_name: impl Into<String>, abi: Abi) -> Self {
        Self {
            file_name: file_name.into(),
            abi: Some(abi),
            source: NativeLibSource::PluginOwned,
            load_order: 50,
            optional: false,
            expected_size: None,
            expected_sha256: None,
            expected_sha1: None,
        }
    }

    /// A lib from the prebuilt LWJGL natives directory (`.../natives/<abi>/`),
    /// for every ABI. Used by renderers that wrap a LWJGL-bundled native such as
    /// the SDL backend (`liblwjgl_sdl.so`), which ships inside
    /// `assets/app_runtime/lwjgl/3.4.1/natives/arm64-v8a/` (see
    /// `FCL_APK_RUNTIME_ASSETS_CATALOG.md`) rather than the FCL APK
    /// `lib/arm64-v8a/`.
    pub fn in_lwjgl_natives(file_name: impl Into<String>) -> Self {
        Self {
            file_name: file_name.into(),
            abi: None,
            source: NativeLibSource::LwjglNatives,
            load_order: 100,
            optional: false,
            expected_size: None,
            expected_sha256: None,
            expected_sha1: None,
        }
    }

    /// Builder: mark this lib optional (missing => warning, not an error).
    pub fn optional(mut self, v: bool) -> Self {
        self.optional = v;
        self
    }

    /// Builder: set the expected uncompressed size (for integrity verification).
    pub fn expected_size(mut self, size: u64) -> Self {
        self.expected_size = Some(size);
        self
    }

    /// Builder: set the expected SHA-256 (for manifest / trust purposes).
    pub fn expected_sha256(mut self, hash: impl Into<String>) -> Self {
        self.expected_sha256 = Some(hash.into());
        self
    }

    /// Builder: set the expected SHA-1 (verifiable at runtime via `sha1`).
    pub fn expected_sha1(mut self, hash: impl Into<String>) -> Self {
        self.expected_sha1 = Some(hash.into());
        self
    }

    /// Validate the file name is a safe bare file name (anti path traversal).
    ///
    /// Rejects empty names, any path separator and any `..` component. This is
    /// the core of *safe loading*: a plugin can never name a lib outside its
    /// own directory.
    pub fn validate_name(&self) -> Result<(), String> {
        if self.file_name.is_empty() {
            return Err("native lib file name is empty".into());
        }
        if self.file_name.contains('/') || self.file_name.contains('\\') {
            return Err(format!(
                "native lib file name must not contain a path separator: {}",
                self.file_name
            ));
        }
        if self.file_name == "."
            || self.file_name == ".."
            || self.file_name.starts_with("../")
            || self.file_name.starts_with("..\\")
            || self.file_name.contains("/../")
            || self.file_name.contains("\\..\\")
        {
            return Err(format!(
                "native lib file name must not be a path traversal: {}",
                self.file_name
            ));
        }
        Ok(())
    }

    /// Verify an on-disk `path` against this lib's expected size / SHA-1.
    ///
    /// Returns [`LibVerify::Match`] when the file is present and (if expected)
    /// its size and SHA-1 match; otherwise the specific mismatch variant. A
    /// missing file yields [`LibVerify::Missing`] (not an error), so callers can
    /// decide whether absence is fatal based on [`NativeLib::optional`].
    pub fn verify_on_disk(&self, path: &Path) -> RcResult<LibVerify> {
        use sha1::{Digest, Sha1};
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return Ok(LibVerify::Missing),
        };
        if let Some(exp) = self.expected_size {
            if meta.len() != exp {
                return Ok(LibVerify::SizeMismatch);
            }
        }
        if let Some(exp_sha1) = &self.expected_sha1 {
            let data = std::fs::read(path)?;
            let mut h = Sha1::new();
            h.update(&data);
            let got = h.finalize();
            let mut got_hex = String::with_capacity(got.len() * 2);
            for b in got {
                got_hex.push_str(&format!("{:02x}", b));
            }
            if &got_hex != exp_sha1 {
                return Ok(LibVerify::HashMismatch);
            }
        }
        Ok(LibVerify::Match)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_name_is_valid() {
        assert!(NativeLib::in_native_lib_dir("libgl4es_114.so")
            .validate_name()
            .is_ok());
    }

    #[test]
    fn traversal_names_are_rejected() {
        for bad in ["../evil.so", "a/../b.so", "lib/gl.so", "..", "."] {
            let lib = NativeLib {
                file_name: bad.into(),
                abi: None,
                source: NativeLibSource::NativeLibDir,
                load_order: 0,
                optional: false,
                expected_size: None,
                expected_sha256: None,
                expected_sha1: None,
            };
            assert!(
                lib.validate_name().is_err(),
                "expected `{bad}` to be rejected"
            );
        }
    }

    #[test]
    fn helpers_set_the_right_source() {
        let a = NativeLib::in_native_lib_dir("libopenal.so");
        assert_eq!(a.source, NativeLibSource::NativeLibDir);
        assert!(!a.optional);
        let b = NativeLib::plugin_owned("libxx.so", Abi::Arm64V8a);
        assert_eq!(b.source, NativeLibSource::PluginOwned);
        assert_eq!(b.abi, Some(Abi::Arm64V8a));
        let c = NativeLib::in_lwjgl_natives("liblwjgl_sdl.so");
        assert_eq!(c.source, NativeLibSource::LwjglNatives);
        assert_eq!(c.abi, None);
    }

    #[test]
    fn builder_sets_integrity() {
        let lib = NativeLib::in_native_lib_dir("libgl4es_114.so")
            .optional(true)
            .expected_size(123)
            .expected_sha1("abc")
            .expected_sha256("def");
        assert!(lib.optional);
        assert_eq!(lib.expected_size, Some(123));
        assert_eq!(lib.expected_sha1.as_deref(), Some("abc"));
        assert_eq!(lib.expected_sha256.as_deref(), Some("def"));
    }

    #[test]
    fn roundtrips_through_json() {
        let lib = NativeLib::plugin_owned("libxx.so", Abi::ArmeabiV7a)
            .expected_size(42)
            .expected_sha1("deadbeef");
        let json = serde_json::to_string(&lib).unwrap();
        let back: NativeLib = serde_json::from_str(&json).unwrap();
        assert_eq!(lib, back);
        // optional fields are emitted; None fields are skipped
        assert!(json.contains("\"expected_size\":42"));
        assert!(!json.contains("expected_sha256"));
    }

    #[test]
    fn verify_on_disk_reports_missing() {
        let lib = NativeLib::in_native_lib_dir("libnope.so").expected_sha1("x");
        assert_eq!(
            lib.verify_on_disk(Path::new("/no/such/libnope.so"))
                .unwrap(),
            LibVerify::Missing
        );
    }
}
