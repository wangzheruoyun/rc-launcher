//! Modpack import pipeline (task 17).
//!
//! This module is the **modpack import pipeline** described in task 17 of the
//! roadmap. It accepts a `.zip` / `.mrpack` archive (or a remote URL) and
//! turns it into a populated, launchable instance directory under
//! `instances_root`.
//!
//! Module layout:
//!
//! * [`manifest`] \u2014 the typed, flavour-agnostic model every parser produces.
//! * [`modrinth`] \u2014 Modrinth `modrinth.index.json` parser.
//! * [`curseforge`] \u2014 CurseForge `manifest.json` parser.
//! * [`mmc`] \u2014 MultiMC / Prism `instance.cfg` + `mmc-pack.json` parser.
//! * [`importer`] \u2014 the orchestrator: archive detection, dependency
//!   resolution, resumable downloads, override extraction, safety checks and
//!   a typed report for the Compose UI.
//!
//! ## Usage
//!
//! ```no_run
//! use std::sync::Arc;
//! use rc_launcher::mods::modpack::{ModpackImporter, ImportOptions};
//! use rc_launcher::net::MirrorProvider;
//!
//! # async fn demo() -> rc_launcher::error::RcResult<()> {
//! let mirror = Arc::new(MirrorProvider::builtin()?);
//! let importer = ModpackImporter::new(
//!     "/data/data/com.rc.launcher/files/instances",
//!     Some(mirror),
//!     ImportOptions::default(),
//! ).await?;
//! let report = importer.import_url(
//!     "https://cdn.modrinth.com/data/P7dR8mSH/versions/abc/atm9.mrpack",
//!     "all-the-mods-9",
//! ).await?;
//! println!("imported {} files ({} already present)",
//!     report.downloaded_count, report.already_present_count);
//! # Ok(()) }
//! ```

pub mod curseforge;
pub mod importer;
pub mod manifest;
pub mod mmc;
pub mod modrinth;

pub use curseforge::parse_curse_manifest;
pub use importer::{
    detect_and_parse, extract_overrides, parse_archive_bytes, FileOutcome, FileReport,
    ImportOptions, ModpackImportReport, ModpackImporter,
};
pub use manifest::{
    Manifest, ModpackFile, ModpackFlavour, ModpackLoader, ModpackOrigin, ModpackSpec,
};
pub use mmc::{build_manifest as build_mmc_manifest, parse_instance_cfg, parse_mmc_pack};
pub use modrinth::parse_modrinth_index;

/// Re-export of the underlying [`crate::download::DownloadManager`] so the
/// rest of the launcher can share the same connection pool with the modpack
/// importer (one client = one happy eyeballs path = one mirror selector).
pub use crate::download::DownloadManager as ImporterDownloadManager;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_api_compiles() {
        // Just ensure every public symbol is reachable from the module root.
        let _: Option<ModpackImporter> = None;
        let _: Option<ModpackSpec> = None;
        let _: Option<Manifest> = None;
        let _: Option<ModpackFlavour> = None;
        let _: Option<ModpackLoader> = None;
        let _: Option<ModpackImportReport> = None;
    }
}

#[cfg(test)]
mod integration_tests;
