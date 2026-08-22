//! JRE runtime directory management (task 6).
//!
//! [`RuntimeManager`] turns the prebuilt FCL JRE packages into usable, on-disk
//! JRE homes:
//!
//! * **download + verify + extract** — for a requested `(JavaVersion, Abi)` it
//!   pulls the `universal` + `bin-<abi>` slices from the [`crate::runtime::source::JreSource`],
//!   checks each archive's SHA-1 against the manifest, then unpacks them
//!   (universal first, ABI slice overlaid on top) into a private install dir;
//! * **multi-version coexist** — every `(version, abi)` gets its own isolated
//!   directory (`<root>/<jreX>/<abi>/`), so Java 8 / 17 / 21 can live side by
//!   side;
//! * **on-demand release** — a JRE can be freed (`release` / `release_unused`)
//!   to reclaim space and re-materialised lazily by the next `ensure` call;
//! * **integrity** — a marker file records the installed archives' SHA-1s, so a
//!   stale or corrupted install is detected and transparently rebuilt.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::runtime::abi::Abi;
use crate::runtime::extract::{ensure_java_executable, extract_tar_xz};
use crate::runtime::java_version::JavaVersion;
use crate::runtime::manifest::{JreArchive, JreManifest};
use crate::runtime::source::JreSource;

/// Marker file name written into each installed JRE home.
const MARKER: &str = ".rc-jre-installed.json";

/// A usable, extracted JRE home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JreHome {
    /// Java version of this home.
    pub version: JavaVersion,
    /// ABI this home was built for.
    pub abi: Abi,
    /// Root of the extracted JRE (contains `bin/`, `lib/`, `release`, …).
    pub home: PathBuf,
    /// Path to the `java` launcher.
    pub java_executable: PathBuf,
}

impl JreHome {
    /// `bin/java` inside the home.
    pub fn java_executable(&self) -> &Path {
        &self.java_executable
    }
}

/// Persisted proof of a successful install.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstallMarker {
    version: JavaVersion,
    abi: Abi,
    /// The exact archive slices (with SHA-1) that were extracted.
    archives: Vec<JreArchive>,
}

/// Provisions and tracks on-disk JRE homes.
pub struct RuntimeManager {
    root: PathBuf,
    source: Box<dyn JreSource>,
    manifest: JreManifest,
}

impl RuntimeManager {
    /// Create a manager rooted at `root`. `source` yields the archive bytes and
    /// `manifest` provides their verification metadata.
    pub fn new(
        root: impl Into<PathBuf>,
        source: Box<dyn JreSource>,
        manifest: JreManifest,
    ) -> Self {
        Self {
            root: root.into(),
            source,
            manifest,
        }
    }

    /// The install directory for `(version, abi)`.
    pub fn install_dir(&self, version: JavaVersion, abi: Abi) -> PathBuf {
        self.root
            .join(version.as_jre_dir())
            .join(abi.as_android_abi())
    }

    fn marker_path(&self, version: JavaVersion, abi: Abi) -> PathBuf {
        self.install_dir(version, abi).join(MARKER)
    }

    /// The manifest this manager validates against.
    pub fn manifest(&self) -> &JreManifest {
        &self.manifest
    }

    /// `(universal, bin)` archive pair for `(version, abi)`, in extraction order.
    fn expected_archives(&self, version: JavaVersion, abi: Abi) -> RcResult<Vec<JreArchive>> {
        let (universal, bin) = self.manifest.archives_for(version, abi)?;
        Ok(vec![universal, bin])
    }

    /// True iff a valid, marker-backed install exists for `(version, abi)` whose
    /// recorded archive SHA-1s still match the manifest (i.e. not stale).
    pub fn is_installed(&self, version: JavaVersion, abi: Abi) -> bool {
        let marker = match read_marker(&self.marker_path(version, abi)) {
            Ok(m) => m,
            Err(_) => return false,
        };
        match self.expected_archives(version, abi) {
            Ok(expected) => marker.archives == expected,
            Err(_) => false,
        }
    }

    /// Return the [`JreHome`] if already installed, else `None`.
    pub fn home(&self, version: JavaVersion, abi: Abi) -> Option<JreHome> {
        if self.is_installed(version, abi) {
            Some(self.make_home(version, abi))
        } else {
            None
        }
    }

    /// Path to `bin/java` if installed, else `None`.
    pub fn java_executable(&self, version: JavaVersion, abi: Abi) -> Option<PathBuf> {
        self.home(version, abi).map(|h| h.java_executable)
    }

    fn make_home(&self, version: JavaVersion, abi: Abi) -> JreHome {
        let home = self.install_dir(version, abi);
        let java_executable = home.join("bin").join("java");
        JreHome {
            version,
            abi,
            home,
            java_executable,
        }
    }

    /// Ensure a usable JRE home exists for `(version, abi)`, installing it on
    /// demand if missing or stale. Returns the [`JreHome`].
    pub async fn ensure(&self, version: JavaVersion, abi: Abi) -> RcResult<JreHome> {
        if self.is_installed(version, abi) {
            return Ok(self.make_home(version, abi));
        }
        self.install(version, abi).await
    }

    /// Download, verify and extract the JRE for `(version, abi)`.
    pub async fn install(&self, version: JavaVersion, abi: Abi) -> RcResult<JreHome> {
        let archives = self.expected_archives(version, abi)?;
        let dir = self.install_dir(version, abi);
        // Clean any partial / stale overlay before a fresh extract.
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(RcError::Io)?;
        }
        std::fs::create_dir_all(&dir).map_err(RcError::Io)?;

        for artifact in &archives {
            let bytes = self.source.read_artifact(version, artifact).await?;
            artifact.verify(&bytes)?;
            extract_tar_xz(&bytes, &dir)?;
        }

        ensure_java_executable(&dir)?;
        self.verify_required_files(&dir)?;

        write_marker(
            &self.marker_path(version, abi),
            &InstallMarker {
                version,
                abi,
                archives,
            },
        )?;
        Ok(self.make_home(version, abi))
    }

    /// Confirm the minimum files a launchable JRE must contain.
    ///
    /// `bin/java` and `release` are layout-stable across Java versions, but the
    /// JVM shared library lives at different paths: Java 9+ flattens it to
    /// `lib/server/libjvm.so`, whereas Java 8 keeps it under the arch subdir
    /// (`lib/aarch64/server/libjvm.so`). We therefore require `bin/java` +
    /// `release` explicitly and then look for `libjvm.so` anywhere under the
    /// home (the exact location varies by version).
    fn verify_required_files(&self, dir: &Path) -> RcResult<()> {
        let stable = [dir.join("bin").join("java"), dir.join("release")];
        for p in stable {
            if !p.exists() {
                return Err(RcError::Other(format!(
                    "extracted JRE is incomplete, missing {}",
                    p.display()
                )));
            }
        }
        if !find_file(dir, "libjvm.so") {
            return Err(RcError::Other(format!(
                "extracted JRE is incomplete, no libjvm.so under {}",
                dir.display()
            )));
        }
        Ok(())
    }

    /// Free the on-disk JRE for `(version, abi)` (on-demand space reclaim).
    pub fn release(&self, version: JavaVersion, abi: Abi) -> RcResult<()> {
        let dir = self.install_dir(version, abi);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(RcError::Io)?;
        }
        Ok(())
    }

    /// Release every installed JRE except those in `keep` (used to reclaim space
    /// after a profile that no longer needs a given Java version). Returns the
    /// `(version, abi)` pairs that were released.
    pub fn release_unused(&self, keep: &[(JavaVersion, Abi)]) -> RcResult<Vec<(JavaVersion, Abi)>> {
        let installed = self.installed();
        let mut released = Vec::new();
        for (v, abi) in installed {
            if !keep.contains(&(v, abi)) {
                self.release(v, abi)?;
                released.push((v, abi));
            }
        }
        Ok(released)
    }

    /// List every currently-installed `(version, abi)` pair.
    pub fn installed(&self) -> Vec<(JavaVersion, Abi)> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for e in entries.flatten() {
            let Some(version) = e.file_name().to_str().and_then(JavaVersion::from_jre_dir) else {
                continue;
            };
            let Ok(abi_entries) = std::fs::read_dir(e.path()) else {
                continue;
            };
            for ae in abi_entries.flatten() {
                let Some(abi) = ae.file_name().to_str().and_then(Abi::from_android_abi) else {
                    continue;
                };
                if self.is_installed(version, abi) {
                    out.push((version, abi));
                }
            }
        }
        out.sort_by_key(|(v, a)| (v.major(), a.as_android_abi().to_string()));
        out
    }
}

fn find_file(dir: &Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            if find_file(&path, name) {
                return true;
            }
        } else if e.file_name() == name {
            return true;
        }
    }
    false
}

fn read_marker(path: &Path) -> RcResult<InstallMarker> {
    let data = std::fs::read(path).map_err(RcError::Io)?;
    serde_json::from_slice(&data).map_err(RcError::Json)
}

fn write_marker(path: &Path, marker: &InstallMarker) -> RcResult<()> {
    let data = serde_json::to_vec_pretty(marker).map_err(RcError::Json)?;
    std::fs::write(path, data).map_err(RcError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::source::LocalDirSource;

    fn prebuilt_dir() -> PathBuf {
        if let Ok(d) = std::env::var("RC_JRE_PREBUILT_DIR") {
            return PathBuf::from(d);
        }
        // CARGO_MANIFEST_DIR = .../rust/crates/rc-launcher-core
        // repo root            = ../../../..  -> then runtime/src/main/assets/app_runtime/java
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .join("..")
            .join("..")
            .join("..")
            .join("runtime")
            .join("src")
            .join("main")
            .join("assets")
            .join("app_runtime")
            .join("java")
    }

    fn manager() -> (RuntimeManager, tempfile::TempDir) {
        let prebuilt = prebuilt_dir();
        assert!(
            prebuilt.exists(),
            "prebuilt JRE dir not found at {}; set RC_JRE_PREBUILT_DIR",
            prebuilt.display()
        );
        let manifest = JreManifest::from_prebuilt_dir(&prebuilt).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mgr = RuntimeManager::new(
            tmp.path().to_path_buf(),
            Box::new(LocalDirSource::new(prebuilt)),
            manifest,
        );
        (mgr, tmp)
    }

    #[tokio::test]
    async fn ensures_and_reports_installed() {
        let (mgr, _t) = manager();
        assert!(!mgr.is_installed(JavaVersion::Java17, Abi::Arm64V8a));
        let home = mgr
            .ensure(JavaVersion::Java17, Abi::Arm64V8a)
            .await
            .unwrap();
        assert!(home.java_executable.exists());
        // libjvm.so lives at lib/server (Java 9+) or lib/<arch>/server (Java 8);
        // assert it exists *somewhere* under the home instead of a fixed path.
        assert!(find_file(&home.home, "libjvm.so"));
        assert!(home.home.join("release").exists());
        assert!(mgr.is_installed(JavaVersion::Java17, Abi::Arm64V8a));
        let again = mgr
            .ensure(JavaVersion::Java17, Abi::Arm64V8a)
            .await
            .unwrap();
        assert_eq!(again.home, home.home);
    }

    #[tokio::test]
    async fn multi_version_coexist() {
        let (mgr, _t) = manager();
        mgr.ensure(JavaVersion::Java8, Abi::Arm64V8a).await.unwrap();
        mgr.ensure(JavaVersion::Java17, Abi::Arm64V8a)
            .await
            .unwrap();
        mgr.ensure(JavaVersion::Java21, Abi::Arm64V8a)
            .await
            .unwrap();
        let installed = mgr.installed();
        assert!(installed.contains(&(JavaVersion::Java8, Abi::Arm64V8a)));
        assert!(installed.contains(&(JavaVersion::Java17, Abi::Arm64V8a)));
        assert!(installed.contains(&(JavaVersion::Java21, Abi::Arm64V8a)));
        assert_eq!(installed.len(), 3);
    }

    #[tokio::test]
    async fn release_frees_and_ensure_reinstalls() {
        let (mgr, _t) = manager();
        mgr.ensure(JavaVersion::Java17, Abi::Arm64V8a)
            .await
            .unwrap();
        mgr.release(JavaVersion::Java17, Abi::Arm64V8a).unwrap();
        assert!(!mgr.is_installed(JavaVersion::Java17, Abi::Arm64V8a));
        // on-demand re-materialisation
        let home = mgr
            .ensure(JavaVersion::Java17, Abi::Arm64V8a)
            .await
            .unwrap();
        assert!(home.java_executable.exists());
    }

    #[tokio::test]
    async fn release_unused_keeps_requested() {
        let (mgr, _t) = manager();
        mgr.ensure(JavaVersion::Java8, Abi::Arm64V8a).await.unwrap();
        mgr.ensure(JavaVersion::Java17, Abi::Arm64V8a)
            .await
            .unwrap();
        let released = mgr
            .release_unused(&[(JavaVersion::Java17, Abi::Arm64V8a)])
            .unwrap();
        assert_eq!(released, vec![(JavaVersion::Java8, Abi::Arm64V8a)]);
        assert!(!mgr.is_installed(JavaVersion::Java8, Abi::Arm64V8a));
        assert!(mgr.is_installed(JavaVersion::Java17, Abi::Arm64V8a));
    }

    #[tokio::test]
    async fn corrupt_source_is_rejected() {
        let prebuilt = prebuilt_dir();
        let manifest = JreManifest::from_prebuilt_dir(&prebuilt).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        // A source that always returns garbage fails the SHA-1 check.
        let bad = BadSource;
        let mgr = RuntimeManager::new(tmp.path().to_path_buf(), Box::new(bad), manifest);
        let res = mgr.ensure(JavaVersion::Java17, Abi::Arm64V8a).await;
        assert!(res.is_err());
    }

    #[test]
    fn committed_manifest_matches_binaries() {
        let prebuilt = prebuilt_dir();
        let manifest_path = prebuilt.join("jre_manifest.json");
        if !manifest_path.exists() {
            // The committed manifest is optional in environments that only run
            // the scan-based tests; skip rather than fail.
            return;
        }
        let json = std::fs::read_to_string(&manifest_path).unwrap();
        let from_json = JreManifest::from_json_str(&json).unwrap();
        let scanned = JreManifest::from_prebuilt_dir(&prebuilt).unwrap();
        assert_eq!(
            from_json.versions, scanned.versions,
            "jre_manifest.json is out of sync with the prebuilt binaries"
        );
    }

    struct BadSource;
    #[async_trait::async_trait]
    impl JreSource for BadSource {
        async fn read_artifact(
            &self,
            _version: JavaVersion,
            artifact: &JreArchive,
        ) -> RcResult<Vec<u8>> {
            Ok(vec![0u8; artifact.size as usize])
        }
    }
}
