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

use crate::download::sha1_bytes;
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
}
