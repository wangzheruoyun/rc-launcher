//! JRE / JDK supply: packaging & extraction for Android OpenJDK (task 6).
//!
//! FCL ships each Android OpenJDK build as a set of XZ-compressed tar archives
//! under `assets/app_runtime/java/jre<major>/`:
//!
//! * `universal.tar.xz` — the ABI-independent part (modules, `conf/`, `legal/`);
//! * `bin-<abi>.tar.xz` — the ABI-specific native binaries (`lib/*.so`,
//!   `lib/server/libjvm.so`, `bin/java`, `release`, …).
//!
//! A complete JRE home is built by unpacking `universal` then overlaying the
//! matching `bin-<abi>` slice. This module mirrors that layout and adds the
//! robustness expected of the core:
//!
//! * [`abi`] / [`java_version`] — typed models of the supported ABIs
//!   (`arm64-v8a` / `armeabi-v7a` / `x86` / `x86_64`) and Java versions
//!   (8 / 17 / 21 / 25).
//! * [`manifest`] — the [`manifest::JreManifest`] describing every archive with
//!   its SHA-1 + size, plus a `from_prebuilt_dir` scanner CI uses to verify the
//!   committed manifest still matches the binaries.
//! * [`source`] — [`source::JreSource`] yields archive bytes, either from a
//!   local `java/` directory ([`source::LocalDirSource`]) or over HTTP(S) via
//!   the resumable, mirror-aware [`crate::download::DownloadManager`]
//!   ([`source::RemoteJreSource`], tasks 2/3) so JRE downloads get HTTP `Range`
//!   resume + parallel shards + SHA-1 verification and degrade to a mirror
//!   when the primary host fails — 断点续传 + 镜像源 for task 6.
//! * [`extract`] — pure-Rust `.tar.xz` extraction with a path-traversal guard.
//! * [`manager`] — [`manager::RuntimeManager`] installs, verifies and tracks
//!   JRE homes with multi-version coexistence and on-demand release. Installs
//!   are crash-safe (hidden `.part` staging dir + atomic rename) and
//!   concurrency-safe (advisory file lock), so an interrupted or parallel
//!   install never leaves a half-built JRE behind.
//!
//! Every subsystem is a self-contained, unit-tested unit so the core stays
//! robust and testable independently of the Android UI. The end-to-end tests
//! run against the real prebuilt FCL JRE packages extracted from the FCL APK
//! (see `runtime/src/main/assets/app_runtime/java/`).

pub mod abi;
pub mod extract;
pub mod java_version;
pub mod manager;
pub mod manifest;
pub mod source;

pub use abi::Abi;
pub use java_version::JavaVersion;
pub use manager::{JreHome, RuntimeManager};
pub use manifest::{ArchiveKind, JreArchive, JreManifest, JreVersionEntry};
pub use source::{JreSource, LocalDirSource, RemoteJreSource};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_is_wired() {
        // Ensure the public re-exports resolve and the doc'd types exist.
        let _ = std::mem::size_of::<JreManifest>();
        let _ = std::mem::size_of::<RuntimeManager>();
        assert_eq!(Abi::Arm64V8a.as_fcl_suffix(), "arm64");
        assert_eq!(JavaVersion::Java17.as_jre_dir(), "jre17");
    }
}
