//! JRE manifest: describes the prebuilt FCL JRE packages (task 6).
//!
//! FCL bundles each JRE as a directory `app_runtime/java/jre<major>/`
//! containing a shared `universal.tar.xz`, one `bin-<abi>.tar.xz` slice per
//! ABI, and a `version` file (the FCL build number). The [`JreManifest`]
//! enumerates every `(java_version, abi)` archive together with its SHA-1 and
//! size so the supply layer can verify downloads/extractions end-to-end.
//!
//! The manifest is produced by `runtime/generate_jre_manifest.py` (run against
//! the prebuilt assets extracted from the FCL APK) and can also be reconstructed
//! at runtime with [`JreManifest::from_prebuilt_dir`], which is what CI uses to
//! prove the committed JSON still matches the binary packages.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::{hex_eq, sha1_bytes, sha1_path, sha256_bytes, sha256_path};
use crate::error::{RcError, RcResult};
use crate::runtime::abi::Abi;
use crate::runtime::java_version::JavaVersion;

/// Kind of a JRE archive slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveKind {
    /// `universal.tar.xz` — the ABI-independent part (modules, conf, legal…).
    Universal,
    /// `bin-<abi>.tar.xz` — the ABI-specific native binaries (`lib/*.so`,
    /// `bin/java`, `lib/server/libjvm.so`, …).
    Bin,
}

/// A single JRE archive slice with its verification metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JreArchive {
    /// Whether this is the universal or a per-ABI slice.
    pub kind: ArchiveKind,
    /// ABI for `bin` slices; `None` for the universal slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi: Option<Abi>,
    /// File name inside the `jre<major>/` directory (`universal.tar.xz`, …).
    pub file: String,
    /// Expected SHA-1 of the archive (lowercase hex).
    pub sha1: String,
    /// Expected size of the archive in bytes.
    pub size: u64,
}

impl JreArchive {
    /// Verify `data` against the recorded SHA-1 and size.
    pub fn verify(&self, data: &[u8]) -> RcResult<()> {
        if data.len() as u64 != self.size {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.size.to_string(),
                actual: data.len().to_string(),
            });
        }
        let actual = sha1_bytes(data);
        if actual.eq_ignore_ascii_case(&self.sha1) {
            Ok(())
        } else {
            Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.sha1.clone(),
                actual,
            })
        }
    }

    /// Verify a file on disk against the recorded SHA-1 and size without
    /// loading it fully into memory (the hash is streamed in fixed-size
    /// blocks). Used by [`crate::runtime::source::LocalDirSource`] before it
    /// streams the extraction, so a corrupt on-disk asset is rejected rather
    /// than silently unpacked (task 25 — large-file streaming handling).
    pub async fn verify_path(&self, path: &Path) -> RcResult<()> {
        let size = std::fs::metadata(path).map_err(RcError::Io)?.len();
        if size != self.size {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.size.to_string(),
                actual: size.to_string(),
            });
        }
        let actual = sha1_path(path).await?;
        if hex_eq(&actual, &self.sha1) {
            Ok(())
        } else {
            Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.sha1.clone(),
                actual,
            })
        }
    }
}

/// All archive slices for one Java version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JreVersionEntry {
    /// The Java version (`jre17`, …).
    pub java_version: JavaVersion,
    /// The numeric major version (17, …).
    pub major: u32,
    /// FCL build number (contents of the `version` file).
    pub build: u32,
    /// Archive slices (one universal + one per supported ABI).
    pub archives: Vec<JreArchive>,
}

/// A single LWJGL artifact (a JAR or a `lib*.so` native) with the verification
/// metadata the supply layer needs to prove an on-device `app_runtime/lwjgl`
/// bundle matches the prebuilt one. Both SHA-1 and SHA-256 are recorded so the
/// verification gate ([`crate::launch::runtime_assets::AppRuntime::verify_lwjgl`])
/// can reject a corrupt *or* tampered artifact on first launch (task 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LwjglArtifact {
    /// File name inside the bundle (`lwjgl.jar`, `liblwjgl_opengl.so`, ...).
    pub file: String,
    /// Expected SHA-1 of the artifact (lowercase hex).
    pub sha1: String,
    /// Expected SHA-256 of the artifact (lowercase hex).
    pub sha256: String,
    /// Expected uncompressed size in bytes.
    pub size: u64,
}

impl LwjglArtifact {
    /// Verify `data` against the recorded size, SHA-1 and SHA-256.
    pub fn verify(&self, data: &[u8]) -> RcResult<()> {
        if data.len() as u64 != self.size {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.size.to_string(),
                actual: data.len().to_string(),
            });
        }
        if !hex_eq(&sha1_bytes(data), &self.sha1) {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.sha1.clone(),
                actual: sha1_bytes(data),
            });
        }
        if !hex_eq(&sha256_bytes(data), &self.sha256) {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.sha256.clone(),
                actual: sha256_bytes(data),
            });
        }
        Ok(())
    }

    /// Verify a file on disk against the recorded metadata without loading it
    /// fully into memory (the hash is streamed in fixed-size blocks).
    pub async fn verify_path(&self, path: &Path) -> RcResult<()> {
        let size = std::fs::metadata(path).map_err(RcError::Io)?.len();
        if size != self.size {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.size.to_string(),
                actual: size.to_string(),
            });
        }
        let actual_sha1 = sha1_path(path).await?;
        if !hex_eq(&actual_sha1, &self.sha1) {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.sha1.clone(),
                actual: actual_sha1,
            });
        }
        let actual_sha256 = sha256_path(path).await?;
        if !hex_eq(&actual_sha256, &self.sha256) {
            return Err(RcError::ChecksumMismatch {
                path: self.file.clone(),
                expected: self.sha256.clone(),
                actual: actual_sha256,
            });
        }
        Ok(())
    }
}

/// All artifacts (JARs + native `.so`) for one LWJGL bundle version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LwjglBundle {
    /// The LWJGL version (`"3.3.3"`, `"3.4.1"`).
    pub version: String,
    /// Java-side JARs (`lwjgl.jar`, `lwjgl-openal.jar`, ...).
    pub jars: Vec<LwjglArtifact>,
    /// ABI-specific native libraries (`liblwjgl.so`, `liblwjgl_opengl.so`, ...),
    /// taken from `natives/arm64-v8a/` (the only ABI RC targets).
    pub natives: Vec<LwjglArtifact>,
}

impl LwjglBundle {
    /// Look up an artifact by file name.
    pub fn find(&self, file: &str) -> Option<&LwjglArtifact> {
        self.jars
            .iter()
            .chain(self.natives.iter())
            .find(|a| a.file == file)
    }

    /// Build a bundle by scanning a `lwjgl/<version>/` directory laid out like
    /// FCL's `assets/app_runtime/lwjgl/<version>/`: every `*.jar` at the top
    /// level and every `*.so` in `natives/arm64-v8a/` (the single ABI RC ships).
    pub fn from_lwjgl_version_dir(version: &str, dir: &Path) -> RcResult<Self> {
        let version = version.to_string();
        let mut jars = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(RcError::Io)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .filter(|p| {
                p.extension()
                    .map(|e| e.eq_ignore_ascii_case("jar"))
                    .unwrap_or(false)
            })
            .collect();
        entries.sort();
        for jar in entries {
            jars.push(scan_lwjgl_artifact(&jar)?);
        }
        let natives_dir = dir.join("natives").join(Abi::Arm64V8a.as_android_abi());
        let mut natives = Vec::new();
        if natives_dir.is_dir() {
            let mut so_entries: Vec<_> = std::fs::read_dir(&natives_dir)
                .map_err(RcError::Io)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter(|p| {
                    p.extension()
                        .map(|e| e.eq_ignore_ascii_case("so"))
                        .unwrap_or(false)
                })
                .collect();
            so_entries.sort();
            for so in so_entries {
                natives.push(scan_lwjgl_artifact(&so)?);
            }
        }
        Ok(LwjglBundle {
            version,
            jars,
            natives,
        })
    }
}

/// The full JRE manifest.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JreManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Human-readable origin of the prebuilt packages.
    pub source: String,
    /// ISO-8601 generation timestamp.
    pub generated_at: String,
    /// Per-Java-version entries.
    pub versions: Vec<JreVersionEntry>,
    /// LWJGL runtime bundles (task 1). `None` when the manifest only describes
    /// JRE packages; CI regenerates it with `runtime/generate_jre_manifest.py`
    /// so both sections are always present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lwjgl: Option<Vec<LwjglBundle>>,
}

impl JreManifest {
    /// Parse a manifest from its JSON representation.
    pub fn from_json_str(json: &str) -> RcResult<Self> {
        serde_json::from_str(json).map_err(RcError::Json)
    }

    /// Look up the entry for a Java version.
    pub fn find(&self, version: JavaVersion) -> Option<&JreVersionEntry> {
        self.versions.iter().find(|e| e.java_version == version)
    }

    /// ABIs a given Java version can be installed for (those with a `bin` slice).
    pub fn supported_abis(&self, version: JavaVersion) -> Vec<Abi> {
        self.find(version)
            .map(|e| {
                e.archives
                    .iter()
                    .filter_map(|a| {
                        if a.kind == ArchiveKind::Bin {
                            a.abi
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Resolve the `(universal, bin)` archive pair for `(version, abi)`.
    pub fn archives_for(
        &self,
        version: JavaVersion,
        abi: Abi,
    ) -> RcResult<(JreArchive, JreArchive)> {
        let entry = self
            .find(version)
            .ok_or_else(|| RcError::Other(format!("no JRE prebuilt for {version}")))?;
        let universal = entry
            .archives
            .iter()
            .find(|a| a.kind == ArchiveKind::Universal)
            .cloned()
            .ok_or_else(|| RcError::Other(format!("missing universal.tar.xz for {version}")))?;
        let bin = entry
            .archives
            .iter()
            .find(|a| a.kind == ArchiveKind::Bin && a.abi == Some(abi))
            .cloned()
            .ok_or_else(|| RcError::Other(format!("no {abi} JRE prebuilt for {version}")))?;
        Ok((universal, bin))
    }

    /// All LWJGL bundles in the manifest, if any.
    pub fn lwjgl_bundles(&self) -> Option<&[LwjglBundle]> {
        self.lwjgl.as_deref()
    }

    /// Look up the LWJGL bundle for a version string (`"3.3.3"`).
    pub fn lwjgl_bundle(&self, version: &str) -> Option<&LwjglBundle> {
        self.lwjgl.as_ref()?.iter().find(|b| b.version == version)
    }

    /// Build a manifest by scanning a `java/` directory laid out like FCL's
    /// `assets/app_runtime/java/` (one `jre<major>/` subdir per version). This
    /// is what CI uses to re-derive the SHA-1/size of every archive and confirm
    /// the committed `jre_manifest.json` still matches the binaries.
    pub fn from_prebuilt_dir(dir: &Path) -> RcResult<Self> {
        let mut versions = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(RcError::Io)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        entries.sort();
        for jre_dir in entries {
            let name = jre_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let Some(version) = JavaVersion::from_jre_dir(name) else {
                continue;
            };
            let build = read_build_number(&jre_dir)?;
            let mut archives = Vec::new();
            // universal slice
            let uni = jre_dir.join("universal.tar.xz");
            if uni.exists() {
                archives.push(scan_archive(&uni, ArchiveKind::Universal, None)?);
            }
            // per-ABI slices
            for abi in Abi::all() {
                let bin = jre_dir.join(abi.bin_archive_name());
                if bin.exists() {
                    archives.push(scan_archive(&bin, ArchiveKind::Bin, Some(*abi))?);
                }
            }
            if archives.is_empty() {
                return Err(RcError::Other(format!(
                    "jre dir {name} contains no tar.xz archives"
                )));
            }
            versions.push(JreVersionEntry {
                java_version: version,
                major: version.major(),
                build,
                archives,
            });
        }
        versions.sort_by_key(|v| v.major);
        Ok(JreManifest {
            schema_version: 1,
            source: "scanned from prebuilt assets".to_string(),
            generated_at: String::new(),
            versions,
            lwjgl: None,
        })
    }

    /// Build a full manifest (JRE + LWJGL) by scanning an `app_runtime/` root
    /// laid out like FCL's `assets/app_runtime/` (a `java/` subdir and a
    /// `lwjgl/` subdir). Convenience wrapper around [`JreManifest::from_prebuilt_dir`]
    /// plus [`LwjglBundle::from_lwjgl_version_dir`] so CI can verify both
    /// sections of the committed `jre_manifest.json` against the binaries.
    pub fn from_app_runtime_root(root: &Path) -> RcResult<Self> {
        let java_dir = root.join("java");
        let mut versions = Vec::new();
        if java_dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&java_dir)
                .map_err(RcError::Io)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            entries.sort();
            for jre_dir in entries {
                let name = jre_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                let Some(version) = JavaVersion::from_jre_dir(name) else {
                    continue;
                };
                let build = read_build_number(&jre_dir)?;
                let mut archives = Vec::new();
                let uni = jre_dir.join("universal.tar.xz");
                if uni.exists() {
                    archives.push(scan_archive(&uni, ArchiveKind::Universal, None)?);
                }
                for abi in Abi::all() {
                    let bin = jre_dir.join(abi.bin_archive_name());
                    if bin.exists() {
                        archives.push(scan_archive(&bin, ArchiveKind::Bin, Some(*abi))?);
                    }
                }
                if archives.is_empty() {
                    return Err(RcError::Other(format!(
                        "jre dir {name} contains no tar.xz archives"
                    )));
                }
                versions.push(JreVersionEntry {
                    java_version: version,
                    major: version.major(),
                    build,
                    archives,
                });
            }
            versions.sort_by_key(|v| v.major);
        }
        let mut lwjgl = Vec::new();
        let lwjgl_dir = root.join("lwjgl");
        if lwjgl_dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&lwjgl_dir)
                .map_err(RcError::Io)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            entries.sort();
            for ver_dir in entries {
                let name = ver_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                lwjgl.push(LwjglBundle::from_lwjgl_version_dir(&name, &ver_dir)?);
            }
        }
        lwjgl.sort_by(|a, b| a.version.cmp(&b.version));
        Ok(JreManifest {
            schema_version: 1,
            source: "scanned from prebuilt assets".to_string(),
            generated_at: String::new(),
            versions,
            lwjgl: if lwjgl.is_empty() { None } else { Some(lwjgl) },
        })
    }
}

/// Read the FCL `version` file (a bare integer) from a `jre<major>/` directory.
fn read_build_number(jre_dir: &Path) -> RcResult<u32> {
    let p = jre_dir.join("version");
    let raw = std::fs::read_to_string(&p).map_err(RcError::Io)?;
    raw.trim()
        .parse::<u32>()
        .map_err(|_| RcError::Other(format!("invalid version file: {raw:?}")))
}

/// Compute a LWJGL artifact's verification metadata (SHA-1 + SHA-256 + size)
/// from its on-disk bytes.
fn scan_lwjgl_artifact(path: &Path) -> RcResult<LwjglArtifact> {
    let data = std::fs::read(path).map_err(RcError::Io)?;
    let size = data.len() as u64;
    let sha1 = sha1_bytes(&data);
    let sha256 = sha256_bytes(&data);
    Ok(LwjglArtifact {
        file: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        sha1,
        sha256,
        size,
    })
}

/// Compute an archive's verification metadata from its on-disk bytes.
fn scan_archive(path: &PathBuf, kind: ArchiveKind, abi: Option<Abi>) -> RcResult<JreArchive> {
    let data = std::fs::read(path).map_err(RcError::Io)?;
    let size = data.len() as u64;
    let sha1 = sha1_bytes(&data);
    Ok(JreArchive {
        kind,
        abi,
        file: path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        sha1,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_verify_accepts_matching_and_rejects_corrupt() {
        let a = JreArchive {
            kind: ArchiveKind::Universal,
            abi: None,
            file: "universal.tar.xz".into(),
            sha1: sha1_bytes(b"hello"),
            size: 5,
        };
        assert!(a.verify(b"hello").is_ok());
        assert!(a.verify(b"hello!").is_err());
        assert!(a.verify(b"world").is_err());
    }

    #[tokio::test]
    async fn archive_verify_path_accepts_matching_and_rejects_corrupt() {
        let dir = std::env::temp_dir().join(format!("rc-verify-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.tar.xz");
        std::fs::write(&p, b"hello").unwrap();
        let a = JreArchive {
            kind: ArchiveKind::Universal,
            abi: None,
            file: "a.tar.xz".into(),
            sha1: sha1_bytes(b"hello"),
            size: 5,
        };
        assert!(a.verify_path(&p).await.is_ok());
        // Wrong size / content on disk must be rejected.
        std::fs::write(&p, b"hello!").unwrap();
        assert!(a.verify_path(&p).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_roundtrips_through_json() {
        let m = JreManifest {
            schema_version: 1,
            source: "test".into(),
            generated_at: "now".into(),
            versions: vec![JreVersionEntry {
                java_version: JavaVersion::Java17,
                major: 17,
                build: 11,
                archives: vec![
                    JreArchive {
                        kind: ArchiveKind::Universal,
                        abi: None,
                        file: "universal.tar.xz".into(),
                        sha1: "deadbeef".into(),
                        size: 1,
                    },
                    JreArchive {
                        kind: ArchiveKind::Bin,
                        abi: Some(Abi::Arm64V8a),
                        file: "bin-arm64.tar.xz".into(),
                        sha1: "cafe".into(),
                        size: 2,
                    },
                ],
            }],
            lwjgl: None,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back = JreManifest::from_json_str(&json).unwrap();
        assert_eq!(back, m);
        let (u, b) = back
            .archives_for(JavaVersion::Java17, Abi::Arm64V8a)
            .unwrap();
        assert_eq!(u.kind, ArchiveKind::Universal);
        assert_eq!(b.abi, Some(Abi::Arm64V8a));
    }

    #[test]
    fn lwjgl_artifact_verify_checks_size_sha1_and_sha256() {
        let art = LwjglArtifact {
            file: "liblwjgl.so".into(),
            sha1: sha1_bytes(b"hello"),
            sha256: sha256_bytes(b"hello"),
            size: 5,
        };
        assert!(art.verify(b"hello").is_ok());
        assert!(art.verify(b"hello!").is_err());
        assert!(art.verify(b"world").is_err());
    }

    #[test]
    fn lwjgl_bundle_roundtrips_and_is_lookupable() {
        let m = JreManifest {
            schema_version: 1,
            source: "test".into(),
            generated_at: String::new(),
            versions: vec![],
            lwjgl: Some(vec![LwjglBundle {
                version: "3.3.3".into(),
                jars: vec![LwjglArtifact {
                    file: "lwjgl.jar".into(),
                    sha1: "aa".into(),
                    sha256: "bb".into(),
                    size: 1,
                }],
                natives: vec![LwjglArtifact {
                    file: "liblwjgl.so".into(),
                    sha1: "cc".into(),
                    sha256: "dd".into(),
                    size: 2,
                }],
            }]),
        };
        let json = serde_json::to_string(&m).unwrap();
        let back = JreManifest::from_json_str(&json).unwrap();
        assert_eq!(back, m);
        let b = back.lwjgl_bundle("3.3.3").unwrap();
        assert_eq!(b.version, "3.3.3");
        assert_eq!(b.find("liblwjgl.so").unwrap().sha256, "dd");
        assert!(back.lwjgl_bundle("9.9.9").is_none());
    }

    #[tokio::test]
    async fn lwjgl_artifact_verify_path_streams() {
        let dir = std::env::temp_dir().join(format!("rc-lw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pp = dir.join("liblwjgl.so");
        std::fs::write(&pp, b"hello").unwrap();
        let art = LwjglArtifact {
            file: "liblwjgl.so".into(),
            sha1: sha1_bytes(b"hello"),
            sha256: sha256_bytes(b"hello"),
            size: 5,
        };
        assert!(art.verify_path(&pp).await.is_ok());
        std::fs::write(&pp, b"hello!").unwrap();
        assert!(art.verify_path(&pp).await.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn from_app_runtime_root_scans_lwjgl() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("app_runtime");
        let jre = root.join("java").join("jre17");
        std::fs::create_dir_all(&jre).unwrap();
        std::fs::write(jre.join("version"), b"11").unwrap();
        std::fs::write(jre.join("universal.tar.xz"), b"uni").unwrap();
        std::fs::write(jre.join("bin-arm64.tar.xz"), b"bin").unwrap();
        let lw = root.join("lwjgl").join("3.3.3");
        std::fs::create_dir_all(lw.join("natives").join("arm64-v8a")).unwrap();
        std::fs::write(lw.join("lwjgl.jar"), b"jar").unwrap();
        std::fs::write(
            lw.join("natives").join("arm64-v8a").join("liblwjgl.so"),
            b"so",
        )
        .unwrap();
        let m = JreManifest::from_app_runtime_root(&root).unwrap();
        assert_eq!(m.versions.len(), 1);
        let b = m.lwjgl_bundle("3.3.3").unwrap();
        assert_eq!(b.jars.len(), 1);
        assert_eq!(b.natives.len(), 1);
    }
}
