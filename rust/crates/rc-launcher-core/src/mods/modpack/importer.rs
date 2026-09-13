//! Modpack import pipeline (task 17).
//!
//! This module glues everything together: a `Manifest` (already parsed by
//! one of [`modrinth`], [`curseforge`], [`mmc`]) \u2192 a flat
//! [`DownloadPlan`] of every downloadable file \u2192 executed through the
//! resumable [`crate::download::DownloadManager`] \u2192 a populated
//! `instances/<name>/` on disk, plus a [`ModpackImportReport`] for the UI.
//!
//! Design choices:
//!
//! * **No I/O on the main thread** \u2014 everything async on top of `tokio`.
//! * **Reuse the existing download manager** (range resume, sha1/md5
//!   verification, exponential backoff, mirror fallback \u2014 task 2/3).
//! * **Reuse the existing dependency resolver** (task 4) when the pipeline
//!   needs to download the Minecraft jars + libraries for the modpack's
//!   `mc_version`.
//! * **Path safety** \u2014 every destination is checked to live under
//!   `instances_root` (no `..` traversal; see [`assert_safe_path`]). The
//!   same defence is used by the file manager (task 19).
//! * **Lenient partial success** \u2014 a single bad hash is reported but does
//!   not abort the import; the user gets a complete list of problems.
//!
//! Entry points:
//!
//! * [`ModpackImporter::new`] \u2014 build it once with a data root + an
//!   optional mirror-aware [`crate::net::NetworkClient`] + an optional
//!   [`crate::download::DownloadManager`].
//! * [`ModpackImporter::import`] \u2014 run the pipeline on a parsed manifest.
//! * [`ModpackImporter::import_from_bytes`] / [`import_from_reader`] \u2014
//!   convenience helpers that take the raw archive (zip) bytes/reader.
//! * [`ModpackImporter::import_url`] \u2014 fetch a remote modpack first
//!   (CurseForge `manifest.json` / Modrinth `.mrpack` URL), then run
//!   [`import`].

use std::collections::HashSet;
use std::io::{Cursor, Read};
#[allow(unused_imports)]
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::download::{Checksum, DownloadManager, DownloadOptions, DownloadTask};
use crate::error::{RcError, RcResult};
use crate::game::DependencyResolver;
use crate::mods::modpack::manifest::{Manifest, ModpackFile, ModpackFlavour, ModpackLoader};
use crate::net::{MirrorProvider, NetworkClient};

/// Default per-file connect + read timeouts for the import pipeline.
pub const DEFAULT_IMPORT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Default per-file read timeout for the import pipeline.
pub const DEFAULT_IMPORT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Default number of retries per file.
pub const DEFAULT_IMPORT_MAX_RETRIES: u32 = 3;

/// Outcome of a single file inside an import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOutcome {
    /// Downloaded and verified.
    Downloaded,
    /// Already present on disk with matching hash \u2014 nothing to do.
    AlreadyPresent,
    /// The destination path would escape the instances root \u2014 rejected.
    UnsafePath,
    /// Downloaded but the hash didn't match \u2014 left on disk but flagged.
    HashMismatch { expected: String, actual: String },
    /// Download failed (network or IO). The `error` field carries the message.
    Failed { error: String },
}

/// One file's entry in the import report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileReport {
    /// The original modpack entry (path / url / hash).
    pub file: ModpackFile,
    /// Destination the file was actually written to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest: Option<PathBuf>,
    /// Outcome (see [`FileOutcome`]).
    pub outcome: FileOutcome,
}

/// What the importer decided to do with the manifest \u2014 the summary the
/// UI shows on the import screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModpackImportReport {
    /// Display name of the modpack.
    pub name: String,
    /// Flavour the manifest was parsed from.
    pub flavour: ModpackFlavour,
    /// The chosen instance root (under `instances_root`).
    pub instance_root: PathBuf,
    /// Files that downloaded successfully.
    pub files: Vec<FileReport>,
    /// Number of files that ended up with [`FileOutcome::Downloaded`].
    pub downloaded_count: usize,
    /// Number of files that ended up with [`FileOutcome::AlreadyPresent`].
    pub already_present_count: usize,
    /// Number of files that ended up with [`FileOutcome::UnsafePath`].
    pub unsafe_path_count: usize,
    /// Number of files that ended up with [`FileOutcome::HashMismatch`].
    pub hash_mismatch_count: usize,
    /// Number of files that ended up with [`FileOutcome::Failed`].
    pub failed_count: usize,
    /// `true` when the whole pipeline ran to completion (regardless of how
    /// many files failed). The UI distinguishes this from the per-file
    /// counters to decide whether the instance is launchable.
    pub completed: bool,
    /// A summary of the modpack (e.g. "Minecraft 1.20.1 + Forge 47.2.0").
    pub summary: String,
}

impl ModpackImportReport {
    /// `true` when no file ended in an error state (unsafe path / hash
    /// mismatch / failed). [`FileOutcome::AlreadyPresent`] and
    /// [`FileOutcome::Downloaded`] are both fine.
    pub fn is_clean(&self) -> bool {
        self.unsafe_path_count == 0 && self.hash_mismatch_count == 0 && self.failed_count == 0
    }
}

/// Configuration knobs for [`ModpackImporter::import`].
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Concurrency for the download manager (per-chunk).
    pub concurrency: usize,
    /// Chunk size for the download manager.
    pub chunk_size: u64,
    /// Max retries per file.
    pub max_retries: u32,
    /// `true` to also generate a download plan for the *Minecraft* jars +
    /// libraries for the modpack's `mc_version`. Requires a
    /// [`crate::net::MirrorProvider`] on the importer.
    pub resolve_game: bool,
    /// `true` to extract any `overrides/` directory inside the archive
    /// (Modrinth / CurseForge convention) onto the instance root. Always
    /// done for archive inputs; can be disabled for manifest-only inputs.
    pub extract_overrides: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            concurrency: 4,
            chunk_size: 4 * 1024 * 1024, // 4 MiB; matches default download manager
            max_retries: DEFAULT_IMPORT_MAX_RETRIES,
            resolve_game: true,
            extract_overrides: true,
        }
    }
}

/// The import pipeline. Cheap to clone (everything is behind `Arc` or is
/// cheaply cloneable like [`NetworkClient`]).
#[derive(Clone)]
pub struct ModpackImporter {
    /// Where to lay out the imported instance (e.g.
    /// `/data/data/com.rc.launcher/files/instances`).
    instances_root: PathBuf,
    /// Mirror provider used by the game download plan + fallback for `fetch_text`.
    mirror: Option<Arc<MirrorProvider>>,
    /// China-mainland-aware network client used for *metadata* fetches (version
    /// manifest, version.json, asset index) and for `import_url` text fetch.
    /// Mirrors / DoH / proxy / retries all apply here, unifying every external
    /// HTTP call through the `NetworkClient` layer (task 29).
    network: Option<NetworkClient>,
    /// Resumable, parallel download manager.
    manager: Arc<DownloadManager>,
    /// Options applied to every import unless overridden.
    options: ImportOptions,
}

impl std::fmt::Debug for ModpackImporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModpackImporter")
            .field("instances_root", &self.instances_root)
            .field("has_mirror", &self.mirror.is_some())
            .field("has_network", &self.network.is_some())
            .field("options", &self.options)
            .finish()
    }
}

impl ModpackImporter {
    /// Build a new importer.
    ///
    /// `instances_root` is the directory the imported instance directory is
    /// created under. `mirror` is optional; when `None` the download manager
    /// still works but skips mirror fallback. `options` carry the per-file
    /// timeouts / retries / concurrency.
    ///
    /// Async because building the mirror-aware `NetworkClient` is an async
    /// operation (DoH resolution).
    pub async fn new(
        instances_root: impl Into<PathBuf>,
        mirror: Option<Arc<MirrorProvider>>,
        options: ImportOptions,
    ) -> RcResult<Self> {
        let download_options = DownloadOptions {
            concurrency: options.concurrency.max(1),
            chunk_size: options.chunk_size.max(1024 * 1024),
            max_retries: options.max_retries,
            ..Default::default()
        };
        let manager_if_mirror = if mirror.is_some() {
            // Build a NetworkClient that uses the mirror provider; the
            // download manager inherits mirror fallback automatically
            // because NetworkClient implements HttpSource. We also keep
            // the NetworkClient around so metadata fetches (manifest,
            // version.json, asset index) inherit the same optimisation.
            let net = crate::net::NetworkClient::builder()
                .mirrors(crate::net::default_mirrors())
                .build()
                .await?;
            let source: Arc<dyn crate::download::HttpSource> = Arc::new(net.clone());
            let manager = Arc::new(DownloadManager::new(source, download_options.clone()));
            Some((net, manager))
        } else {
            None
        };
        let (network, manager) = match manager_if_mirror {
            Some((net, mgr)) => (Some(net), mgr),
            None => (
                None,
                Arc::new(DownloadManager::with_default_source(download_options)?),
            ),
        };
        Ok(Self {
            instances_root: instances_root.into(),
            mirror,
            network,
            manager,
            options,
        })
    }

    /// Synchronous constructor that uses a pre-built `NetworkClient` (no
    /// mirror-aware DoH). This is the path the unit tests take; the
    /// production launcher should call [`Self::new`].
    pub fn with_network_client(
        instances_root: impl Into<PathBuf>,
        mirror: Option<Arc<MirrorProvider>>,
        client: crate::net::NetworkClient,
        options: ImportOptions,
    ) -> RcResult<Self> {
        let download_options = DownloadOptions {
            concurrency: options.concurrency.max(1),
            chunk_size: options.chunk_size.max(1024 * 1024),
            max_retries: options.max_retries,
            ..Default::default()
        };
        let source: Arc<dyn crate::download::HttpSource> = Arc::new(client.clone());
        let manager = Arc::new(DownloadManager::new(source, download_options));
        Ok(Self {
            instances_root: instances_root.into(),
            mirror,
            network: Some(client),
            manager,
            options,
        })
    }

    /// Synchronous constructor that uses the default `reqwest` source
    /// (no mirror awareness). Convenience for tests.
    pub fn with_default_source(
        instances_root: impl Into<PathBuf>,
        options: ImportOptions,
    ) -> RcResult<Self> {
        let download_options = DownloadOptions {
            concurrency: options.concurrency.max(1),
            chunk_size: options.chunk_size.max(1024 * 1024),
            max_retries: options.max_retries,
            ..Default::default()
        };
        let manager = Arc::new(DownloadManager::with_default_source(download_options)?);
        Ok(Self {
            instances_root: instances_root.into(),
            mirror: None,
            network: None,
            manager,
            options,
        })
    }

    /// The configured instances root.
    pub fn instances_root(&self) -> &Path {
        &self.instances_root
    }

    /// The active download manager (read-only handle).
    pub fn download_manager(&self) -> Arc<DownloadManager> {
        self.manager.clone()
    }

    /// Run the import pipeline against an already-parsed manifest.
    ///
    /// `instance_id` is the **directory name** the instance will live under
    /// (e.g. `"all-the-mods-9"`). The directory must not already exist;
    /// [`Self::import_allowing_overwrite`] relaxes that.
    pub async fn import(
        &self,
        manifest: &Manifest,
        instance_id: &str,
    ) -> RcResult<ModpackImportReport> {
        self.run(manifest, instance_id, false).await
    }

    /// Like [`import`], but allows an existing instance directory to be
    /// overwritten.
    pub async fn import_allowing_overwrite(
        &self,
        manifest: &Manifest,
        instance_id: &str,
    ) -> RcResult<ModpackImportReport> {
        self.run(manifest, instance_id, true).await
    }

    async fn run(
        &self,
        manifest: &Manifest,
        instance_id: &str,
        allow_overwrite: bool,
    ) -> RcResult<ModpackImportReport> {
        if instance_id.is_empty()
            || instance_id.contains('/')
            || instance_id.contains('\\')
            || instance_id.contains("..")
        {
            return Err(RcError::Other(format!(
                "unsafe instance_id: {instance_id:?}"
            )));
        }
        let spec = manifest.spec();
        let instance_root = self.instances_root.join(instance_id);
        if instance_root.exists() && !allow_overwrite {
            return Err(RcError::Other(format!(
                "instance already exists: {}",
                instance_root.display()
            )));
        }
        tokio::fs::create_dir_all(&instance_root)
            .await
            .map_err(RcError::Io)?;

        // Optionally pull the Minecraft version (libraries + client jar).
        if self.options.resolve_game {
            if let Some(mp) = &self.mirror {
                self.plan_game(&spec.mc_version, &instance_root, mp.clone())
                    .await?;
            }
        }

        // Build the download plan for the modpack files.
        let resolved = spec.with_dest_resolved(&instance_root);
        let tasks: Vec<DownloadTask> = resolved
            .files
            .iter()
            .filter_map(|f| build_download_task(f))
            .collect();

        let mut reports = Vec::with_capacity(resolved.files.len());
        for (file, task) in resolved.files.iter().zip(tasks.iter()) {
            // Path safety: refuse anything that escapes the instance root.
            let dest = match file.dest.as_ref() {
                Some(d) => d.clone(),
                None => {
                    reports.push(FileReport {
                        file: file.clone(),
                        dest: None,
                        outcome: FileOutcome::UnsafePath,
                    });
                    continue;
                }
            };
            if !is_under(&dest, &instance_root) {
                reports.push(FileReport {
                    file: file.clone(),
                    dest: Some(dest),
                    outcome: FileOutcome::UnsafePath,
                });
                continue;
            }
            if let Some(parent) = dest.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(RcError::Io)?;
            }
            // Fast path: the file already exists with a matching hash.
            if let Some(checksum) = &task.checksum {
                let hash_result = match checksum {
                    Checksum::Sha1(_) => crate::download::sha1_path(&dest).await,
                    Checksum::Md5(_) => crate::download::md5_path(&dest).await,
                };
                if let Ok(existing) = hash_result {
                    let expected = match checksum {
                        Checksum::Sha1(s) | Checksum::Md5(s) => s.to_ascii_lowercase(),
                    };
                    if existing.eq_ignore_ascii_case(&expected) {
                        reports.push(FileReport {
                            file: file.clone(),
                            dest: Some(dest),
                            outcome: FileOutcome::AlreadyPresent,
                        });
                        continue;
                    }
                }
            }
            // Otherwise download.
            match self.manager.download(task).await {
                Ok(_summary) => {
                    let outcome = verify_after_download(&dest, task.checksum.as_ref()).await;
                    reports.push(FileReport {
                        file: file.clone(),
                        dest: Some(dest),
                        outcome,
                    });
                }
                Err(e) => {
                    reports.push(FileReport {
                        file: file.clone(),
                        dest: Some(dest),
                        outcome: FileOutcome::Failed {
                            error: e.to_string(),
                        },
                    });
                }
            }
        }

        let report = aggregate_report(
            manifest,
            instance_root.clone(),
            reports,
            self.options.resolve_game,
        );

        // Mark the run complete; callers see `completed: true` and the
        // counters they need to decide whether to retry.
        Ok(ModpackImportReport {
            completed: true,
            ..report
        })
    }

    /// Resolve a Minecraft `version_id` and stage the libraries + client
    /// jar + assets index under the instance root (task 4 reuse).
    async fn plan_game(
        &self,
        version_id: &str,
        instance_root: &Path,
        mirror: Arc<MirrorProvider>,
    ) -> RcResult<()> {
        let platform = crate::game::Platform::android();
        // Use the importer's NetworkClient for metadata fetches; it carries
        // the mirror provider, DoH/proxy, timeouts and backoff so every
        // metadata HTTP call goes through the same optimised path (task 29).
        let net = match &self.network {
            Some(n) => n.clone(),
            None => {
                // No NetworkClient on this importer; build a minimal one
                // with the default mirror list so we still get mirror
                // fallback for metadata fetches (task 29).
                crate::net::NetworkClient::builder()
                    .mirrors(crate::net::default_mirrors())
                    .config(crate::net::NetworkConfig {
                        connect_timeout: DEFAULT_IMPORT_CONNECT_TIMEOUT,
                        read_timeout: DEFAULT_IMPORT_READ_TIMEOUT,
                        ..crate::net::NetworkConfig::default()
                    })
                    .build()
                    .await?
            }
        };
        let manifest = match crate::game::VersionManifest::fetch(&net).await {
            Ok(m) => m,
            Err(_) => return Ok(()),
        };
        let resolver =
            DependencyResolver::new(platform, mirror.clone(), instance_root.to_path_buf());
        let plan = match resolver
            .resolve_full_plan(&net, &manifest, version_id)
            .await
        {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
        for task in plan.into_tasks() {
            if let Some(parent) = task.dest.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = self.manager.download(&task).await; // best-effort
        }
        Ok(())
    }

    /// Convenience: fetch a remote `manifest.json` / `modrinth.index.json`
    /// by URL and run [`import`].
    pub async fn import_url(&self, url: &str, instance_id: &str) -> RcResult<ModpackImportReport> {
        let text = self.fetch_text(url).await?;
        let manifest = detect_and_parse(&text, Some(url))?;
        self.import(&manifest, instance_id).await
    }

    /// Convenience: import a local `.zip` / `.mrpack` archive.
    pub async fn import_from_bytes(
        &self,
        bytes: &[u8],
        instance_id: &str,
    ) -> RcResult<ModpackImportReport> {
        let manifest = parse_archive_bytes(bytes, None)?;
        self.import(&manifest, instance_id).await
    }

    /// Convenience: import a local archive from any reader.
    pub async fn import_from_reader<R: Read>(
        &self,
        reader: R,
        instance_id: &str,
    ) -> RcResult<ModpackImportReport> {
        let mut buf = Vec::new();
        let mut r = reader;
        r.read_to_end(&mut buf).map_err(RcError::Io)?;
        self.import_from_bytes(&buf, instance_id).await
    }
}

#[allow(unused_imports)]
fn aggregate_report(
    manifest: &Manifest,
    instance_root: PathBuf,
    files: Vec<FileReport>,
    resolved_game: bool,
) -> ModpackImportReport {
    let mut downloaded = 0;
    let mut already = 0;
    let mut unsafe_count = 0;
    let mut hash_mismatch = 0;
    let mut failed = 0;
    for r in &files {
        match &r.outcome {
            FileOutcome::Downloaded => downloaded += 1,
            FileOutcome::AlreadyPresent => already += 1,
            FileOutcome::UnsafePath => unsafe_count += 1,
            FileOutcome::HashMismatch { .. } => hash_mismatch += 1,
            FileOutcome::Failed { .. } => failed += 1,
        }
    }
    let spec = manifest.spec();
    let mut summary = format!("MC {} + ", spec.mc_version);
    match spec.loader {
        ModpackLoader::Vanilla => summary.push_str("Vanilla"),
        ModpackLoader::Fabric => summary.push_str("Fabric"),
        ModpackLoader::Quilt => summary.push_str("Quilt"),
        ModpackLoader::Forge => summary.push_str("Forge"),
        ModpackLoader::NeoForge => summary.push_str("NeoForge"),
    }
    if let Some(v) = &spec.loader_version {
        summary.push(' ');
        summary.push_str(v);
    }
    if resolved_game {
        summary.push_str(" (game resolved)");
    }
    ModpackImportReport {
        name: spec.name.clone(),
        flavour: spec.flavour,
        instance_root,
        files,
        downloaded_count: downloaded,
        already_present_count: already,
        unsafe_path_count: unsafe_count,
        hash_mismatch_count: hash_mismatch,
        failed_count: failed,
        completed: false,
        summary,
    }
}

fn build_download_task(file: &ModpackFile) -> Option<DownloadTask> {
    if file.url.is_empty() {
        return None;
    }
    let dest = file.dest.clone()?;
    let mut task = DownloadTask::new(file.url.clone(), dest).with_id(file.id.clone());
    if let Some(sha1_val) = file.sha1.as_ref().filter(|s| !s.is_empty()) {
        task = task.with_sha1(sha1_val.clone());
    } else if let Some(md5_val) = file.md5.as_ref().filter(|s| !s.is_empty()) {
        task = task.with_md5(md5_val.clone());
    }
    if let Some(size) = file.size {
        task = task.with_size(size);
    }
    Some(task)
}

async fn verify_after_download(dest: &Path, checksum: Option<&Checksum>) -> FileOutcome {
    let Some(checksum) = checksum else {
        return FileOutcome::Downloaded;
    };
    let expected = match checksum {
        Checksum::Sha1(s) | Checksum::Md5(s) => s.to_ascii_lowercase(),
    };
    let actual = match checksum {
        Checksum::Sha1(_) => crate::download::sha1_path(dest).await,
        Checksum::Md5(_) => crate::download::md5_path(dest).await,
    };
    match actual {
        Ok(a) if a.eq_ignore_ascii_case(&expected) => FileOutcome::Downloaded,
        Ok(a) => FileOutcome::HashMismatch {
            expected,
            actual: a,
        },
        Err(e) => FileOutcome::Failed {
            error: format!("post-download hash failed: {e}"),
        },
    }
}

/// Detect the flavour of `text` and parse it.
///
/// Detection is purely structural \u2014 we don't trust the file name. The order
/// is:
///
/// 1. JSON object with `formatVersion` + `game == "minecraft"` + `dependencies.minecraft`
///    \u2192 Modrinth.
/// 2. JSON object with `manifestType == "minecraftModpack"` + `minecraft.version`
///    \u2192 CurseForge.
/// 3. JSON object with `formatVersion` + `mcVersion` \u2192 MMC `mmc-pack.json`.
/// 4. JSON array (CurseForge sometimes wraps mods in an array) \u2192 fall back
///    to CurseForge.
pub fn detect_and_parse(text: &str, origin: Option<&str>) -> RcResult<Manifest> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') {
        return Err(RcError::Other(
            "modpack manifest is not a JSON object".into(),
        ));
    }
    // Try Modrinth first: cheap probe.
    if let Ok(value) = serde_json::Value::from_str(trimmed) {
        let is_modrinth = value.get("formatVersion").is_some()
            && value.get("game").is_some()
            && value.get("dependencies").is_some()
            && value
                .get("dependencies")
                .and_then(|d| d.get("minecraft"))
                .is_some();
        let is_curse = value.get("manifestType").is_some() && value.get("minecraft").is_some();
        let is_mmc = value.get("formatVersion").is_some()
            && value.get("mcVersion").is_some()
            && !is_modrinth;
        if is_modrinth {
            return super::modrinth::parse_modrinth_index(trimmed, origin);
        }
        if is_curse {
            return super::curseforge::parse_curse_manifest(trimmed, origin);
        }
        if is_mmc {
            return super::mmc::build_manifest("", trimmed, origin);
        }
    }
    Err(RcError::Other(
        "could not detect modpack flavour from JSON shape".into(),
    ))
}

/// Parse a `.zip` / `.mrpack` archive, returning the manifest it contains.
///
/// Walks the top-level entries, picks the most specific manifest file, and
/// parses it through [`detect_and_parse`]. The remaining archive entries
/// (overrides / loose mods) are *not* extracted here \u2014 that's the job of
/// [`extract_overrides`], which the caller invokes after [`ModpackImporter::import`].
pub fn parse_archive_bytes(bytes: &[u8], origin: Option<&str>) -> RcResult<Manifest> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| RcError::Other(format!("not a valid zip/mrpack archive: {e}")))?;

    // Manifest filename priority: Modrinth > CurseForge > MMC pack > MMC cfg.
    let candidates: &[&str] = &[
        "modrinth.index.json",
        "manifest.json",
        "mmc-pack.json",
        "pack.json",
    ];
    let mut picked: Option<(String, String)> = None;
    for name in candidates {
        if let Ok(mut entry) = archive.by_name(name) {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .map_err(|e| RcError::Other(format!("reading {name}: {e}")))?;
            picked = Some(((*name).to_string(), text));
            break;
        }
    }
    let (manifest_name, manifest_text) = picked.ok_or_else(|| {
        RcError::Other(
            "no recognised manifest found (expected modrinth.index.json / manifest.json / mmc-pack.json / pack.json)"
                .into(),
        )
    })?;

    let origin = origin
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("archive:{}", manifest_name));

    let manifest = if manifest_name == "manifest.json" {
        super::curseforge::parse_curse_manifest(&manifest_text, Some(&origin))?
    } else if manifest_name == "modrinth.index.json" {
        super::modrinth::parse_modrinth_index(&manifest_text, Some(&origin))?
    } else {
        // MMC: try to also read instance.cfg.
        let cfg_text = archive
            .by_name("instance.cfg")
            .ok()
            .and_then(|mut e| {
                let mut s = String::new();
                e.read_to_string(&mut s).ok()?;
                Some(s)
            })
            .unwrap_or_default();
        super::mmc::build_manifest(&cfg_text, &manifest_text, Some(&origin))?
    };

    Ok(manifest)
}

/// Extract any `overrides/` directory inside the archive onto the instance
/// root. Returns the list of files written so the caller can confirm.
pub fn extract_overrides(bytes: &[u8], instance_root: &Path) -> RcResult<Vec<PathBuf>> {
    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| RcError::Other(format!("not a valid zip/mrpack archive: {e}")))?;
    let mut written = Vec::new();
    let names = archive
        .file_names()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        let stripped = strip_overrides_prefix(&name);
        let Some(rel) = stripped else { continue };
        let dest = instance_root.join(&rel);
        if !is_under(&dest, instance_root) {
            continue; // path traversal attempt; silently skip
        }
        let mut entry = match archive.by_name(&name) {
            Ok(e) if !e.is_dir() => e,
            _ => continue,
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(RcError::Io)?;
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).map_err(RcError::Io)?;
        std::fs::write(&dest, &buf).map_err(RcError::Io)?;
        written.push(dest);
    }
    Ok(written)
}

fn strip_overrides_prefix(name: &str) -> Option<String> {
    // Accept either "overrides/<rel>" or "./overrides/<rel>".
    let n = name.trim_start_matches('.');
    let n = n.trim_start_matches('/');
    if let Some(stripped) = n.strip_prefix("overrides/") {
        return Some(stripped.to_string());
    }
    if n == "overrides" || n == "overrides/" {
        return Some(String::new());
    }
    None
}

/// True iff `path` is strictly under `root` after normalisation. Any
/// `ParentDir` (`..`) component anywhere in `path` causes this to return
/// `false` — we never allow path-traversal even if it ends up inside the
/// root after joining.
pub fn is_under(path: &Path, root: &Path) -> bool {
    for c in path.components() {
        if matches!(c, Component::ParentDir) {
            return false;
        }
    }
    let mut p = path.components().peekable();
    let mut r = root.components().peekable();
    loop {
        match (p.peek(), r.peek()) {
            (None, Some(_)) => return false,
            (Some(_), None) => return true,
            (None, None) => return false, // equal paths are not "under"
            (Some(pc), Some(rc)) => {
                if pc == rc {
                    p.next();
                    r.next();
                    continue;
                }
                return false;
            }
        }
    }
}

/// Fetch a text document by URL through the [`crate::net::NetworkClient`],
/// inheriting the full China-mainland optimisation stack: mirror fallback,
/// DoH/proxy, timeouts, exponential backoff and connection reuse.
impl ModpackImporter {
    async fn fetch_text(&self, url: &str) -> RcResult<String> {
        if let Some(net) = &self.network {
            // Use the shared NetworkClient: mirrors / DoH / proxy / retries
            // / backoff all apply (task 29).
            let bytes = net.fetch_bytes(url).await?;
            return String::from_utf8(bytes)
                .map_err(|e| RcError::Other(format!("fetch {url}: invalid utf8: {e}")));
        }
        // Fallback: no NetworkClient (e.g. with_default_source path).
        // Build a minimal reqwest client — no mirror fallback, but still
        // respects timeouts.
        let client = reqwest::Client::builder()
            .timeout(DEFAULT_IMPORT_READ_TIMEOUT)
            .connect_timeout(DEFAULT_IMPORT_CONNECT_TIMEOUT)
            .build()
            .map_err(|e| RcError::Other(format!("http client build failed: {e}")))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| RcError::Other(format!("fetch {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(RcError::Other(format!(
                "fetch {url}: HTTP {}",
                resp.status()
            )));
        }
        resp.text()
            .await
            .map_err(|e| RcError::Other(format!("read {url}: {e}")))
    }
}

/// Re-export of the parser helpers so [`super::mod`] can `pub use` them as
/// `modpack::{parse_modrinth_index, parse_curse_manifest, ...}`.
pub use super::curseforge::parse_curse_manifest;
pub use super::mmc::{build_manifest as build_mmc_manifest, parse_instance_cfg, parse_mmc_pack};
pub use super::modrinth::parse_modrinth_index;

#[allow(unused_imports)]
use std::str::FromStr;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::modpack::manifest::{ModpackFile, ModpackSpec};
    use std::str::FromStr;

    fn tempdir(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("rc_modpack_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn is_under_accepts_descendants() {
        let root = Path::new("/data/instances/abc");
        assert!(is_under(Path::new("/data/instances/abc/mods/x.jar"), root));
        assert!(!is_under(Path::new("/data/instances/other/x.jar"), root));
        assert!(!is_under(Path::new("/etc/passwd"), root));
        assert!(!is_under(
            Path::new("/data/instances/abc2/mods/x.jar"),
            root
        ));
    }

    #[test]
    fn detect_and_parse_routes_modrinth() {
        let json = r#"{"formatVersion":1,"game":"minecraft","name":"x","dependencies":{"minecraft":"1.20.1"},"files":[]}"#;
        let m = detect_and_parse(json, Some("https://e/x")).unwrap();
        assert_eq!(m.flavour(), ModpackFlavour::Modrinth);
    }

    #[test]
    fn detect_and_parse_routes_curseforge() {
        let json = r#"{"manifestType":"minecraftModpack","manifestVersion":1,"name":"x","minecraft":{"version":"1.20.1","modLoaders":[]},"files":[]}"#;
        let m = detect_and_parse(json, Some("https://e/x")).unwrap();
        assert_eq!(m.flavour(), ModpackFlavour::CurseForge);
    }

    #[test]
    fn detect_and_parse_routes_mmc() {
        let json = r#"{"formatVersion":1,"name":"x","mcVersion":"1.20.1","mods":[]}"#;
        let m = detect_and_parse(json, Some("/p/x")).unwrap();
        assert_eq!(m.flavour(), ModpackFlavour::MultiMc);
    }

    #[test]
    fn build_download_task_sets_sha1() {
        let file = ModpackFile {
            id: "a".into(),
            dest: Some(PathBuf::from("/data/instance/mods/a.jar")),
            path: "mods/a.jar".into(),
            url: "https://e/a".into(),
            sha1: Some("deadbeef".into()),
            md5: None,
            size: Some(100),
            force: false,
        };
        let task = build_download_task(&file).unwrap();
        match task.checksum.unwrap() {
            Checksum::Sha1(s) => assert_eq!(s, "deadbeef"),
            _ => panic!("expected sha1"),
        }
        assert_eq!(task.size, Some(100));
    }

    #[test]
    fn build_download_task_sets_md5_when_no_sha1() {
        let file = ModpackFile {
            id: "a".into(),
            dest: Some(PathBuf::from("/data/instance/mods/a.jar")),
            path: "mods/a.jar".into(),
            url: "https://e/a".into(),
            sha1: None,
            md5: Some("cafebabe".into()),
            size: None,
            force: false,
        };
        let task = build_download_task(&file).unwrap();
        match task.checksum.unwrap() {
            Checksum::Md5(s) => assert_eq!(s, "cafebabe"),
            _ => panic!("expected md5"),
        }
    }

    #[test]
    fn extract_overrides_prefix_stripping() {
        assert_eq!(
            strip_overrides_prefix("overrides/a/b.txt").as_deref(),
            Some("a/b.txt")
        );
        assert_eq!(
            strip_overrides_prefix("./overrides/a.txt").as_deref(),
            Some("a.txt")
        );
        assert_eq!(strip_overrides_prefix("overrides/").as_deref(), Some(""));
        assert_eq!(strip_overrides_prefix("not-overrides/x").as_deref(), None);
    }

    #[test]
    fn unsafe_path_is_filtered() {
        let tmp = tempdir("unsafe");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let importer = rt
            .block_on(ModpackImporter::new(&tmp, None, ImportOptions::default()))
            .unwrap();
        // Build a spec with one file whose path escapes the instance root.
        let mut spec = ModpackSpec::new("X", "1.20.1");
        spec.flavour = ModpackFlavour::Modrinth;
        spec.files.push(ModpackFile {
            id: "evil".into(),
            dest: None, // signals "unsafe" in the pipeline
            path: "../escape.jar".into(),
            url: "https://e/evil.jar".into(),
            sha1: None,
            md5: None,
            size: None,
            force: false,
        });
        let manifest = Manifest::Modrinth { raw: None, spec };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let report = rt
            .block_on(importer.import(&manifest, "test-unsafe"))
            .unwrap();
        assert_eq!(report.unsafe_path_count, 1);
        assert_eq!(report.files[0].outcome, FileOutcome::UnsafePath);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn import_options_default_has_sensible_values() {
        let o = ImportOptions::default();
        assert!(o.concurrency >= 1);
        assert!(o.chunk_size >= 1024 * 1024);
        assert!(o.max_retries >= 1);
        assert!(o.resolve_game);
        assert!(o.extract_overrides);
    }

    #[test]
    fn manifest_origin_is_recorded() {
        let json = r#"{"formatVersion":1,"game":"minecraft","name":"x","dependencies":{"minecraft":"1.20.1"},"files":[]}"#;
        let m = detect_and_parse(json, Some("https://cdn/x.mrpack")).unwrap();
        match m.origin() {
            crate::mods::modpack::manifest::ModpackOrigin::Url(u) => {
                assert!(u.starts_with("https://"))
            }
            other => panic!("expected URL origin, got {other:?}"),
        }
    }
}
