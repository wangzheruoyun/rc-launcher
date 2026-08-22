//! Version manifest & dependency resolution (task 4).
//!
//! This module turns Mojang's version metadata into a concrete, downloadable
//! plan optimised for the China mainland:
//!
//! * [`manifest`] parses the top-level `version_manifest` (every published
//!   version + its `version.json` URL).
//! * [`version`] parses each `version.json` (client jar, asset index, libraries,
//!   main class, launch arguments) and resolves the `inheritsFrom` chain — the
//!   same job FCLCore/game and FCLCore/util do when building a launchable
//!   profile.
//! * [`library`] models a dependency and evaluates the `rules` that gate it by
//!   OS / architecture / feature.
//! * [`assets`] parses the assets index (every resource object + its SHA-1).
//! * [`platform`] detects the target platform and evaluates OS rules (incl. a
//!   tiny anchored-regex engine for `os.version`).
//! * [`resolve`] is the orchestrator: it merges the resolved version into a
//!   deduplicated, platform-filtered, rule-matched [`resolve::DownloadPlan`] and
//!   rewrites every URL through the [`crate::net::mirror`] mirrors so the
//!   download manager (task 2) inherits mirror fallback automatically.
//!
//! Every subsystem is a self-contained, unit-tested unit so the core stays
//! robust and testable independently of the Android UI.

pub mod assets;
pub mod library;
pub mod manifest;
pub mod platform;
pub mod resolve;
pub mod version;

pub use assets::{AssetObject, AssetsIndex};
pub use library::{Action, Artifact, ExtractRule, Library, LibraryDownloads, Rule};
pub use manifest::{Latest, VersionEntry, VersionManifest, VERSION_MANIFEST_URL};
pub use platform::{Arch, Features, OsName, OsRule, Platform};
pub use resolve::{ArtifactKind, DependencyResolver, DownloadItem, DownloadPlan};
pub use version::{
    AssetIndexRef, DownloadInfo, Downloads, JavaVersion, ResolvedVersion, VersionArguments,
    VersionJson,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_is_wired() {
        // Ensure the public re-exports resolve and the doc'd types exist.
        let _ = std::mem::size_of::<DownloadPlan>();
        let _ = std::mem::size_of::<ResolvedVersion>();
        assert_eq!(
            VERSION_MANIFEST_URL,
            "https://launchermeta.mojang.com/mc/game/version_manifest.json"
        );
    }
}
