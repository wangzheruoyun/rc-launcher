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

use std::path::{Path, PathBuf};

use crate::error::{RcError, RcResult};
use crate::launch::options::LwjglVersion;
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
