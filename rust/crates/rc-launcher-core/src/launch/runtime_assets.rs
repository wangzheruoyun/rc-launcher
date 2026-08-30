//! The on-device `app_runtime/` layout (task 7).
//!
//! FCL ships the pieces the JVM needs to run Minecraft on Android as APK assets
//! and unpacks them into the app's data directory. The layout is exactly the one
//! catalogued from the FCL APK (`FCL_APK_RUNTIME_ASSETS_CATALOG.md`):
//!
//! ```text
//! app_runtime/
//! ├── java/jre{8,17,21,25}/{universal,bin-<abi>}.tar.xz   (task 6)
//! ├── lwjgl/{3.3.3,3.4.1}/*.jar + natives/<abi>/*.so
//! ├── caciocavallo/{cacio-shared,cacio-androidnw,ResConfHack}.jar   (Java 8)
//! ├── caciocavallo17/{cacio-shared,cacio-tta,cacio-agent}.jar       (Java 17+)
//! └── jna/jna-<abi>.zip
//! ```
//!
//! [`AppRuntime`] turns that directory into the concrete classpath / native
//! search-path inputs the launch engine needs, and validates that the pieces are
//! actually present (robustness: a missing LWJGL bundle must fail *before* we
//! spawn a JVM that would die with an unhelpful `UnsatisfiedLinkError`).

use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

use crate::error::{RcError, RcResult};
use crate::launch::options::LwjglVersion;
use crate::runtime::manifest::LwjglBundle;
use crate::runtime::{Abi, JavaVersion};

/// Directory name of the Java-8 caciocavallo bundle.
pub const CACIO_DIR: &str = "caciocavallo";
/// Directory name of the Java-17+ caciocavallo bundle.
pub const CACIO17_DIR: &str = "caciocavallo17";
/// The Java-17+ cacio java agent (loaded with `-javaagent:`).
pub const CACIO_AGENT_JAR: &str = "cacio-agent.jar";

/// An extracted `app_runtime/` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRuntime {
    root: PathBuf,
}

impl AppRuntime {
    /// Wrap an `app_runtime/` directory (no I/O yet).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The wrapped directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `app_runtime/java/` (task-6 JRE packages live here).
    pub fn java_dir(&self) -> PathBuf {
        self.root.join("java")
    }

    /// `app_runtime/java/jre<major>/`.
    pub fn jre_dir(&self, version: JavaVersion) -> PathBuf {
        self.java_dir().join(version.as_jre_dir())
    }

    /// `app_runtime/lwjgl/<version>/`.
    pub fn lwjgl_dir(&self, version: LwjglVersion) -> PathBuf {
        self.root.join("lwjgl").join(version.as_dir())
    }

    /// `app_runtime/lwjgl/<version>/natives/<abi>/` (prebuilt `liblwjgl*.so`).
    pub fn lwjgl_natives_dir(&self, version: LwjglVersion, abi: Abi) -> PathBuf {
        self.lwjgl_dir(version)
            .join("natives")
            .join(abi.as_android_abi())
    }

    /// Every LWJGL jar of the bundle, sorted for a deterministic classpath.
    ///
    /// These *replace* the `org.lwjgl:*` libraries a vanilla `version.json`
    /// declares — the Mojang jars ship desktop natives that cannot load on
    /// Android, which is exactly why FCL bundles its own build.
    pub fn lwjgl_jars(&self, version: LwjglVersion) -> RcResult<Vec<PathBuf>> {
        let dir = self.lwjgl_dir(version);
        let jars = list_jars(&dir)?;
        if jars.is_empty() {
            return Err(RcError::MissingFile(format!(
                "no LWJGL jars in {}",
                dir.display()
            )));
        }
        Ok(jars)
    }

    /// The caciocavallo bundle directory for a Java version (task 18 AWT bridge).
    ///
    /// Java 8 uses the original `caciocavallo` build, Java 17+ the
    /// `caciocavallo17` one — the same split FCL makes.
    pub fn cacio_dir(&self, java: JavaVersion) -> PathBuf {
        match java {
            JavaVersion::Java8 => self.root.join(CACIO_DIR),
            _ => self.root.join(CACIO17_DIR),
        }
    }

    /// Caciocavallo jars for a Java version (sorted; empty when not installed).
    ///
    /// The Java-17+ `cacio-agent.jar` is *excluded*: it is passed with
    /// `-javaagent:` rather than placed on the classpath.
    pub fn cacio_jars(&self, java: JavaVersion) -> RcResult<Vec<PathBuf>> {
        let dir = self.cacio_dir(java);
        let jars = list_jars(&dir)?;
        Ok(jars
            .into_iter()
            .filter(|p| p.file_name().map(|n| n != CACIO_AGENT_JAR).unwrap_or(false))
            .collect())
    }

    /// The `cacio-agent.jar` path when present (Java 17+ only).
    pub fn cacio_agent(&self, java: JavaVersion) -> Option<PathBuf> {
        if java == JavaVersion::Java8 {
            return None;
        }
        let p = self.cacio_dir(java).join(CACIO_AGENT_JAR);
        p.is_file().then_some(p)
    }

    /// Unpack the per-ABI JNA native dispatch archive into the runtime dir.
    ///
    /// JNA loads its dispatcher (`libjnidispatch.so`) from
    /// `jna.boot.library.path`. The copy bundled inside `jna.jar` is a
    /// *desktop* build that cannot load on Android, which is why a JNA-using
    /// mod dies with `Unable to load JNA library`. FCL ships
    /// `app_runtime/jna/jna-<abi>.zip` with one `libjnidispatch.so` per JNA
    /// release (`jna/<version>/libjnidispatch.so`); we unpack it next to the
    /// archive (`app_runtime/jna/<abi>/`) and return that directory.
    ///
    /// The dispatcher from the *newest* bundled JNA release is also surfaced as
    /// a bare `app_runtime/jna/<abi>/libjnidispatch.so` because JNA probes
    /// `jna.boot.library.path` for the bare file name; the versioned originals
    /// are kept for callers / future JNA builds that resolve by version.
    ///
    /// Idempotent: a `.rc-jna-extracted` marker skips re-extraction on every
    /// launch, and the bare dispatcher's presence is the real readiness guard.
    pub fn extract_jna(&self, abi: Abi) -> RcResult<PathBuf> {
        let archive = self.jna_archive(abi).ok_or_else(|| {
            RcError::MissingFile(format!(
                "no JNA native dispatch archive for {} (expected app_runtime/jna/jna-{}.zip)",
                abi.as_android_abi(),
                abi.as_fcl_suffix()
            ))
        })?;
        let out_dir = self.root.join("jna").join(abi.as_fcl_suffix());
        let marker = out_dir.join(".rc-jna-extracted");
        let bare = out_dir.join("libjnidispatch.so");
        // Already extracted (and the dispatcher JNA probes for is present)?
        if marker.is_file() && bare.is_file() {
            return Ok(out_dir);
        }

        std::fs::create_dir_all(&out_dir).map_err(RcError::Io)?;
        let file = std::fs::File::open(&archive).map_err(RcError::Io)?;
        let mut zip = ZipArchive::new(file).map_err(|e| {
            RcError::Other(format!(
                "could not open JNA archive {}: {e}",
                archive.display()
            ))
        })?;

        // Track the newest bundled JNA dispatcher so we can expose a bare copy.
        let mut newest: Option<(Vec<u64>, PathBuf)> = None;

        for i in 0..zip.len() {
            let mut entry = zip
                .by_index(i)
                .map_err(|e| RcError::Other(format!("reading JNA archive entry {i}: {e}")))?;
            // Defend against path traversal in the (APK-supplied) archive.
            let rel = entry.enclosed_name().ok_or_else(|| {
                RcError::Other(format!("JNA archive entry {i} has an unsafe path"))
            })?;
            let target = jna_safe_join(&out_dir, &rel)?;
            if entry.is_dir() {
                std::fs::create_dir_all(&target).map_err(RcError::Io)?;
                continue;
            }
            if !entry.name().ends_with("libjnidispatch.so") {
                continue;
            }
            // `write` needs the (nested) parent directory to exist; the archive
            // keeps each dispatcher under `jna/<version>/`, so create it first.
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(RcError::Io)?;
            }
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf).map_err(RcError::Io)?;
            std::fs::write(&target, &buf).map_err(RcError::Io)?;
            if let Some(ver) = jna_version_of(&rel) {
                let better = match &newest {
                    Some((best, _)) => version_gt(&ver, best),
                    None => true,
                };
                if better {
                    newest = Some((ver, target.clone()));
                }
            }
        }

        match &newest {
            Some((_, src)) => {
                std::fs::copy(src, &bare).map_err(RcError::Io)?;
            }
            None => {
                return Err(RcError::MissingFile(format!(
                    "JNA archive {} contains no libjnidispatch.so",
                    archive.display()
                )));
            }
        }

        // Best-effort marker; the bare dispatcher above is the real guard.
        let _ = std::fs::write(&marker, "1");
        Ok(out_dir)
    }

    /// `app_runtime/jna/jna-<suffix>.zip` when present.
    pub fn jna_archive(&self, abi: Abi) -> Option<PathBuf> {
        let p = self
            .root
            .join("jna")
            .join(format!("jna-{}.zip", abi.as_fcl_suffix()));
        p.is_file().then_some(p)
    }

    /// Validate the pieces a launch needs, returning a precise error.
    pub fn verify(&self, java: JavaVersion, lwjgl: LwjglVersion, abi: Abi) -> RcResult<()> {
        if !self.root.is_dir() {
            return Err(RcError::MissingFile(format!(
                "app_runtime directory not found: {}",
                self.root.display()
            )));
        }
        self.lwjgl_jars(lwjgl)?;
        let natives = self.lwjgl_natives_dir(lwjgl, abi);
        if !natives.is_dir() {
            return Err(RcError::MissingFile(format!(
                "LWJGL natives for {} not found: {}",
                abi.as_android_abi(),
                natives.display()
            )));
        }
        // The AWT bridge is optional (only needed when `use_cacio` is on), so a
        // missing cacio bundle is reported by the caller, not here.
        let _ = java;
        Ok(())
    }

    /// Verify the LWJGL bundle (JARs + `arm64-v8a` natives) for `version`
    /// against its recorded SHA-1 / SHA-256 / size, scoped to `abi` (task 1).
    ///
    /// This is the "extract-and-verify on first launch" gate for the LWJGL
    /// runtime: the Android side unpacks `assets/app_runtime/lwjgl/<version>/`
    /// out of the APK on first boot, and this method proves the unpacked bytes
    /// still match FCL's prebuilt bundle before a JVM is spawned. A corrupt or
    /// truncated `.so`/`.jar` is rejected here with a precise
    /// [`RcError::ChecksumMismatch`] instead of letting Minecraft 1.13+
    /// (which loads LWJGL's OpenGL binding) die with an opaque
    /// `UnsatisfiedLinkError` at the first GL call.
    pub fn verify_lwjgl(
        &self,
        version: LwjglVersion,
        abi: Abi,
        bundle: &LwjglBundle,
    ) -> RcResult<()> {
        let dir = self.lwjgl_dir(version);
        if !dir.is_dir() {
            return Err(RcError::MissingFile(format!(
                "LWJGL {} bundle not found: {}",
                version.as_dir(),
                dir.display()
            )));
        }
        // JARs live at the top level of the bundle directory.
        for art in &bundle.jars {
            let p = dir.join(&art.file);
            let data = std::fs::read(&p).map_err(RcError::Io)?;
            art.verify(&data).map_err(|e| {
                RcError::Other(format!(
                    "LWJGL {} jar {} failed verification: {e}",
                    version.as_dir(),
                    art.file
                ))
            })?;
        }
        // Natives are ABI-scoped (`natives/<abi>/`). RC only ships arm64-v8a, so
        // any other ABI correctly fails the directory-existence check below.
        let natives = self.lwjgl_natives_dir(version, abi);
        if !natives.is_dir() {
            return Err(RcError::MissingFile(format!(
                "LWJGL {} natives for {} not found: {}",
                version.as_dir(),
                abi.as_android_abi(),
                natives.display()
            )));
        }
        for art in &bundle.natives {
            let p = natives.join(&art.file);
            let data = std::fs::read(&p).map_err(RcError::Io)?;
            art.verify(&data).map_err(|e| {
                RcError::Other(format!(
                    "LWJGL {} native {} ({}) failed verification: {e}",
                    version.as_dir(),
                    art.file,
                    abi.as_android_abi()
                ))
            })?;
        }
        Ok(())
    }
}

/// Sorted `*.jar` list of a directory. A missing directory yields an empty list
/// so optional bundles do not need an `is_dir()` dance at every call site.
fn list_jars(dir: &Path) -> RcResult<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("jar"))
                .unwrap_or(false)
        {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Join `rel` onto `base`, rejecting anything that would escape `base`.
///
/// `ZipFile::enclosed_name` already strips absolute / `..` components for us, but
/// we re-check here (defence in depth) and return a precise error instead of an
/// `fs` panic if a hostile archive slips through.
fn jna_safe_join(base: &Path, rel: &Path) -> RcResult<PathBuf> {
    if rel.is_absolute() {
        return Err(RcError::Other(format!(
            "JNA archive entry is an absolute path: {rel:?}"
        )));
    }
    let mut out = base.to_path_buf();
    for comp in rel.components() {
        match comp {
            std::path::Component::Normal(c) => out.push(c),
            std::path::Component::CurDir => {}
            _ => {
                return Err(RcError::Other(format!(
                    "JNA archive entry escapes root: {rel:?}"
                )));
            }
        }
    }
    Ok(out)
}

/// `jna/<version>/libjnidispatch.so` -> `Some([version numbers])` (else `None`).
fn jna_version_of(rel: &Path) -> Option<Vec<u64>> {
    let comps: Vec<std::ffi::OsString> = rel
        .components()
        .map(|c| c.as_os_str().to_os_string())
        .collect();
    if comps.len() < 3 {
        return None;
    }
    if comps[comps.len() - 1] != std::ffi::OsStr::new("libjnidispatch.so") {
        return None;
    }
    if comps[0] != std::ffi::OsStr::new("jna") {
        return None;
    }
    let ver = comps[comps.len() - 2].to_string_lossy();
    let parts: Vec<u64> = ver
        .split('.')
        .filter_map(|p| p.parse::<u64>().ok())
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

/// `a > b` for dot-separated numeric version vectors (shorter is older).
fn version_gt(a: &[u64], b: &[u64]) -> bool {
    let n = a.len().max(b.len());
    for i in 0..n {
        let av = a.get(i).copied().unwrap_or(0);
        let bv = b.get(i).copied().unwrap_or(0);
        if av != bv {
            return av > bv;
        }
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fake `app_runtime/` tree using the *real* file names catalogued
    /// from the FCL APK (`FCL_APK_RUNTIME_ASSETS_CATALOG.md`).
    fn fake_runtime() -> (tempfile::TempDir, AppRuntime) {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("app_runtime");
        let lwjgl = root.join("lwjgl").join("3.3.3");
        std::fs::create_dir_all(lwjgl.join("natives").join("arm64-v8a")).unwrap();
        for jar in [
            "lwjgl.jar",
            "lwjgl-3.3.3-merged-modules.jar",
            "lwjgl-glfw.jar",
            "lwjgl-openal.jar",
            "lwjgl-stb.jar",
            "jsr305.jar",
        ] {
            std::fs::write(lwjgl.join(jar), b"jar").unwrap();
        }
        std::fs::write(lwjgl.join("version"), b"3.3.3").unwrap();
        for so in ["liblwjgl.so", "liblwjgl_opengl.so", "libfreetype.so"] {
            std::fs::write(lwjgl.join("natives").join("arm64-v8a").join(so), b"so").unwrap();
        }
        let cacio = root.join(CACIO_DIR);
        std::fs::create_dir_all(&cacio).unwrap();
        for jar in [
            "cacio-shared-1.10-SNAPSHOT.jar",
            "cacio-androidnw-1.10-SNAPSHOT.jar",
            "ResConfHack.jar",
        ] {
            std::fs::write(cacio.join(jar), b"jar").unwrap();
        }
        let cacio17 = root.join(CACIO17_DIR);
        std::fs::create_dir_all(&cacio17).unwrap();
        for jar in [
            "cacio-shared-1.19.1-SNAPSHOT.jar",
            "cacio-tta-1.19.1-SNAPSHOT.jar",
            CACIO_AGENT_JAR,
        ] {
            std::fs::write(cacio17.join(jar), b"jar").unwrap();
        }
        std::fs::create_dir_all(root.join("jna")).unwrap();
        std::fs::write(root.join("jna").join("jna-arm64.zip"), b"zip").unwrap();
        let rt = AppRuntime::new(&root);
        (td, rt)
    }

    #[test]
    fn paths_follow_the_fcl_layout() {
        let rt = AppRuntime::new("/data/app_runtime");
        assert_eq!(
            rt.lwjgl_dir(LwjglVersion::V3_4_1),
            PathBuf::from("/data/app_runtime/lwjgl/3.4.1")
        );
        assert_eq!(
            rt.lwjgl_natives_dir(LwjglVersion::V3_3_3, Abi::Arm64V8a),
            PathBuf::from("/data/app_runtime/lwjgl/3.3.3/natives/arm64-v8a")
        );
        assert_eq!(
            rt.jre_dir(JavaVersion::Java21),
            PathBuf::from("/data/app_runtime/java/jre21")
        );
        assert_eq!(
            rt.cacio_dir(JavaVersion::Java8),
            PathBuf::from("/data/app_runtime/caciocavallo")
        );
        assert_eq!(
            rt.cacio_dir(JavaVersion::Java17),
            PathBuf::from("/data/app_runtime/caciocavallo17")
        );
    }

    #[test]
    fn lwjgl_jars_are_sorted_and_complete() {
        let (_td, rt) = fake_runtime();
        let jars = rt.lwjgl_jars(LwjglVersion::V3_3_3).unwrap();
        assert_eq!(jars.len(), 6, "{jars:?}");
        // deterministic order
        let mut sorted = jars.clone();
        sorted.sort();
        assert_eq!(jars, sorted);
        // `version` (not a jar) is skipped
        assert!(jars.iter().all(|p| p.extension().unwrap() == "jar"));
    }

    #[test]
    fn missing_lwjgl_bundle_is_an_error() {
        let (_td, rt) = fake_runtime();
        let err = rt.lwjgl_jars(LwjglVersion::V3_4_1).unwrap_err();
        assert!(err.to_string().contains("no LWJGL jars"), "{err}");
        assert!(rt
            .verify(JavaVersion::Java17, LwjglVersion::V3_4_1, Abi::Arm64V8a)
            .is_err());
    }

    #[test]
    fn verify_checks_natives_for_the_requested_abi() {
        let (_td, rt) = fake_runtime();
        assert!(rt
            .verify(JavaVersion::Java17, LwjglVersion::V3_3_3, Abi::Arm64V8a)
            .is_ok());
        let err = rt
            .verify(JavaVersion::Java17, LwjglVersion::V3_3_3, Abi::X86_64)
            .unwrap_err();
        assert!(err.to_string().contains("x86_64"), "{err}");
    }

    #[test]
    fn cacio_jars_exclude_the_agent() {
        let (_td, rt) = fake_runtime();
        let j17 = rt.cacio_jars(JavaVersion::Java17).unwrap();
        assert_eq!(j17.len(), 2, "{j17:?}");
        assert!(j17
            .iter()
            .all(|p| p.file_name().unwrap() != CACIO_AGENT_JAR));
        assert!(rt.cacio_agent(JavaVersion::Java17).is_some());
        // Java 8 has no agent and uses the other bundle
        assert!(rt.cacio_agent(JavaVersion::Java8).is_none());
        assert_eq!(rt.cacio_jars(JavaVersion::Java8).unwrap().len(), 3);
    }

    #[test]
    fn jna_archive_is_abi_scoped() {
        let (_td, rt) = fake_runtime();
        assert!(rt.jna_archive(Abi::Arm64V8a).is_some());
        assert!(rt.jna_archive(Abi::X86).is_none());
    }

    #[test]
    fn extract_jna_unpacks_and_exposes_a_bare_dispatcher() {
        let td = tempfile::tempdir().unwrap();
        let rt = AppRuntime::new(td.path());

        // No archive yet -> precise error, not a panic on a missing file.
        assert!(rt.extract_jna(Abi::Arm64V8a).is_err());

        // Use FCL's real per-ABI JNA archive (one `libjnidispatch.so` per JNA
        // release, under `jna/<version>/`) shipped as an app_runtime asset.
        let asset = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../runtime/src/main/assets/app_runtime/jna/jna-arm64.zip");
        assert!(asset.is_file(), "JNA asset missing at {asset:?}");
        std::fs::create_dir_all(td.path().join("jna")).unwrap();
        std::fs::copy(&asset, td.path().join("jna").join("jna-arm64.zip")).unwrap();

        let out = rt.extract_jna(Abi::Arm64V8a).unwrap();
        assert_eq!(out, td.path().join("jna").join("arm64"));
        // The newest dispatcher is surfaced bare for JNA's boot-path probe.
        assert!(out.join("libjnidispatch.so").is_file(), "bare dispatcher");
        // Versioned originals are preserved for per-version resolution.
        let mut versioned = 0;
        if let Ok(entries) = std::fs::read_dir(out.join("jna")) {
            for e in entries.flatten() {
                if e.path().join("libjnidispatch.so").is_file() {
                    versioned += 1;
                }
            }
        }
        assert!(versioned > 0, "versioned dispatcher dirs preserved");

        // Idempotent: a second call reuses the already-extracted dir and does
        // not re-extract / error.
        let out2 = rt.extract_jna(Abi::Arm64V8a).unwrap();
        assert_eq!(out, out2);

        // An ABI with no archive is reported precisely.
        assert!(rt.extract_jna(Abi::X86).is_err());
    }

    #[test]
    fn verify_reports_a_missing_root() {
        let rt = AppRuntime::new("/definitely/not/here");
        let err = rt
            .verify(JavaVersion::Java17, LwjglVersion::V3_3_3, Abi::Arm64V8a)
            .unwrap_err();
        assert!(err.to_string().contains("app_runtime directory not found"));
    }

    #[test]
    fn missing_optional_dir_yields_empty_list() {
        let td = tempfile::tempdir().unwrap();
        let rt = AppRuntime::new(td.path());
        assert!(rt.cacio_jars(JavaVersion::Java17).unwrap().is_empty());
    }
}
