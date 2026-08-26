//! Dependency resolution & download-plan generation (task 4).
//!
//! [`DependencyResolver`] turns a resolved [`ResolvedVersion`] (after inheritance
//! merging) into a [`DownloadPlan`]: a deduplicated, platform-filtered and
//! rule-matched list of [`DownloadItem`]s — the client jar, the asset index, the
//! libraries (main + native jars) and the logging client — each rewritten
//! through the China-mainland [`MirrorProvider`] so the download manager inherits
//! mirror fallback for free.
//!
//! Asset *objects* are added separately (via [`DependencyResolver::plan_assets`])
//! once the asset index has been fetched, because their URLs can only be derived
//! from the index contents.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::download::{Checksum, DownloadTask};
use crate::error::{RcError, RcResult};
use crate::game::assets::AssetsIndex;
use crate::game::manifest::VersionManifest;
use crate::game::platform::{Features, Platform};
use crate::game::version::{fetch_json_with_mirrors, ResolvedVersion, VersionJson};
use crate::net::MirrorProvider;

/// What kind of artifact a [`DownloadItem`] represents (for progress grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactKind {
    /// The Minecraft client jar (`versions/<id>/<id>.jar`).
    Client,
    /// A library or native jar under `libraries/`.
    Library,
    /// The assets index JSON under `assets/indexes/`.
    AssetIndex,
    /// An individual asset object under `assets/objects/`.
    AssetObject,
    /// The logging client configuration file.
    Logging,
}

/// One concrete file to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadItem {
    /// Stable id for progress events (e.g. `library/com.mojang:patchy:1.1`).
    pub id: String,
    /// Canonical (Mojang CDN) URL — what the download manager sends to the
    /// network layer, which then applies mirror fallback.
    pub url: String,
    /// Mirror-rewritten candidate URLs (preferred first), for UI/diagnostics.
    pub mirrors: Vec<String>,
    /// Destination path (absolute on the device, rooted at the data directory).
    pub dest: PathBuf,
    pub checksum: Option<Checksum>,
    pub size: Option<u64>,
    pub kind: ArtifactKind,
}

/// A deduplicated collection of [`DownloadItem`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadPlan {
    pub items: Vec<DownloadItem>,
}

impl DownloadPlan {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Insert an item, deduplicating by destination path. When an item with the
    /// same destination already exists, the more complete metadata (checksum,
    /// size, mirrors) is merged in — this is how shared libraries/assets across
    /// multiple versions collapse into a single download.
    pub fn add(&mut self, item: DownloadItem) {
        if let Some(existing) = self.items.iter_mut().find(|i| i.dest == item.dest) {
            if existing.checksum.is_none() {
                existing.checksum = item.checksum.clone();
            }
            if existing.size.is_none() {
                existing.size = item.size;
            }
            if existing.url.is_empty() {
                existing.url = item.url.clone();
            }
            for m in &item.mirrors {
                if !existing.mirrors.contains(m) {
                    existing.mirrors.push(m.clone());
                }
            }
        } else {
            self.items.push(item);
        }
    }

    /// Explicit deduplication pass (idempotent).
    pub fn dedup(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(self.items.len());
        for item in self.items.drain(..) {
            if seen.insert(item.dest.clone()) {
                out.push(item);
            }
        }
        self.items = out;
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Sum of known content sizes (bytes).
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().filter_map(|i| i.size).sum()
    }

    /// Count items of a given kind.
    pub fn count_kind(&self, kind: ArtifactKind) -> usize {
        self.items.iter().filter(|i| i.kind == kind).count()
    }

    /// Convert the plan into download tasks. Each task uses the *canonical* URL
    /// so the [`crate::download::HttpSource`] (which rewrites through mirrors)
    /// performs fallback automatically.
    pub fn into_tasks(self) -> Vec<DownloadTask> {
        self.items
            .into_iter()
            .map(|i| {
                let mut task = DownloadTask::new(i.url, i.dest).with_id(i.id);
                if let Some(c) = i.checksum {
                    task.checksum = Some(c);
                }
                if let Some(s) = i.size {
                    task = task.with_size(s);
                }
                task
            })
            .collect()
    }
}

/// Drives version dependency resolution and download-plan generation.
pub struct DependencyResolver {
    pub platform: Platform,
    pub mirror: Arc<MirrorProvider>,
    /// Launcher data root (absolute); items are placed under `versions/`,
    /// `libraries/`, `assets/` relative to it.
    pub root: PathBuf,
}

impl DependencyResolver {
    pub fn new(platform: Platform, mirror: Arc<MirrorProvider>, root: impl Into<PathBuf>) -> Self {
        Self {
            platform,
            mirror,
            root: root.into(),
        }
    }

    /// Build the version-level plan (client jar, asset index, libraries, logging
    /// client) from an already-resolved version. Does **not** include asset
    /// objects — add those via [`Self::plan_assets`] after fetching the index.
    pub fn build_plan(&self, resolved: &ResolvedVersion) -> DownloadPlan {
        let mut plan = DownloadPlan::new();
        let root = &self.root;
        let platform = &self.platform;

        // 1) Client jar.
        if let Some(client) = &resolved.downloads.client {
            let dest = root
                .join("versions")
                .join(&resolved.id)
                .join(format!("{}.jar", resolved.id));
            plan.add(self.item(
                &format!("client/{}", resolved.id),
                &client.url,
                dest,
                client.sha1.clone(),
                client.size,
                ArtifactKind::Client,
            ));
        }

        // 2) Asset index (modern `assetIndex` block, or the legacy `assets` string).
        if let Some(ai) = resolved.asset_index_ref().as_ref() {
            let dest = root
                .join("assets")
                .join("indexes")
                .join(format!("{}.json", ai.id));
            plan.add(self.item(
                &format!("assetindex/{}", ai.id),
                &ai.url,
                dest,
                ai.sha1.clone(),
                ai.size,
                ArtifactKind::AssetIndex,
            ));
        }

        // 3) Libraries (rule-filtered) + natives.
        for lib in &resolved.libraries {
            if !lib.is_allowed(platform, &Features::new()) {
                continue;
            }
            if let Some(url) = lib.artifact_url() {
                let dest = root.join("libraries").join(lib.maven_path(None));
                plan.add(self.item(
                    &format!("library/{}", lib.name),
                    &url,
                    dest,
                    lib.downloads.artifact.as_ref().and_then(|a| a.sha1.clone()),
                    lib.downloads.artifact.as_ref().and_then(|a| a.size),
                    ArtifactKind::Library,
                ));
            }
            if let Some(classifier) = lib.native_classifier(platform) {
                if let Some(url) = lib.classifier_url(&classifier) {
                    let dest = root
                        .join("libraries")
                        .join(lib.maven_path(Some(&classifier)));
                    let sha1 = lib
                        .downloads
                        .classifiers
                        .get(&classifier)
                        .and_then(|a| a.sha1.clone());
                    let size = lib
                        .downloads
                        .classifiers
                        .get(&classifier)
                        .and_then(|a| a.size);
                    plan.add(self.item(
                        &format!("native/{}/{}", lib.name, classifier),
                        &url,
                        dest,
                        sha1,
                        size,
                        ArtifactKind::Library,
                    ));
                }
            }
        }

        // 4) Logging client file (if present).
        if let Some(logging) = &resolved.logging {
            if let Some(client) = logging.get("client").and_then(|c| c.get("file")) {
                if let (Some(url), Some(id)) = (
                    client.get("url").and_then(|v| v.as_str()),
                    client.get("id").and_then(|v| v.as_str()),
                ) {
                    let sha1 = client
                        .get("sha1")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let size = client.get("size").and_then(|v| v.as_u64());
                    let dest = root
                        .join("assets")
                        .join("log_configs")
                        .join(format!("{}.xml", id));
                    plan.add(self.item(
                        &format!("logging/{}", id),
                        url,
                        dest,
                        sha1,
                        size,
                        ArtifactKind::Logging,
                    ));
                }
            }
        }

        plan.dedup();
        plan
    }

    /// Build the asset-object download items from a fetched [`AssetsIndex`].
    ///
    /// Modern indexes store every object under `assets/objects/<hh>/<hash>` and
    /// the object is fetched by its SHA-1 from the Mojang CDN. Very old versions
    /// (the `map_to_resources` flag set) instead lay objects out under
    /// `assets/resources/<logical-name>` using their *original* path; we still
    /// download them by hash, so only the destination differs.
    pub fn plan_assets(&self, index: &AssetsIndex) -> Vec<DownloadItem> {
        let mut items = Vec::new();
        let root = &self.root;
        let legacy = index.map_to_resources.unwrap_or(false);
        for (name, obj) in &index.objects {
            let hash = &obj.hash;
            let prefix = match hash.get(..2) {
                Some(p) => p,
                None => continue,
            };
            let url = format!(
                "https://resources.download.minecraft.net/{}/{}",
                prefix, hash
            );
            let (dest, id) = if legacy {
                (
                    root.join("assets").join("resources").join(name),
                    format!("asset/legacy/{}", name),
                )
            } else {
                (
                    root.join("assets").join("objects").join(prefix).join(hash),
                    format!("asset/{}", name),
                )
            };
            items.push(self.item(
                &id,
                &url,
                dest,
                Some(hash.clone()),
                Some(obj.size),
                ArtifactKind::AssetObject,
            ));
        }
        items
    }

    /// Convenience: build the full plan (version + asset objects) given the
    /// resolved version and its (already fetched) assets index.
    pub fn build_full_plan(
        &self,
        resolved: &ResolvedVersion,
        assets: &AssetsIndex,
    ) -> DownloadPlan {
        let mut plan = self.build_plan(resolved);
        for item in self.plan_assets(assets) {
            plan.add(item);
        }
        plan.dedup();
        plan
    }

    fn item(
        &self,
        id: &str,
        url: &str,
        dest: PathBuf,
        sha1: Option<String>,
        size: Option<u64>,
        kind: ArtifactKind,
    ) -> DownloadItem {
        let mirrors = self.mirror.rewrite_all(url);
        DownloadItem {
            id: id.to_string(),
            url: url.to_string(),
            mirrors,
            dest,
            checksum: sha1.map(Checksum::Sha1),
            size,
            kind,
        }
    }

    // ----- async helpers (network integration) -----

    /// Resolve a version (walking `inheritsFrom`) and return the merged view.
    pub async fn resolve_version(
        &self,
        client: &reqwest::Client,
        manifest: &VersionManifest,
        version_id: &str,
    ) -> RcResult<ResolvedVersion> {
        let mut chain: Vec<VersionJson> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut current = version_id.to_string();
        loop {
            if !visited.insert(current.clone()) {
                return Err(RcError::Other(format!(
                    "inheritance cycle detected at version `{}`",
                    current
                )));
            }
            let entry = manifest.find(&current).ok_or_else(|| {
                RcError::Other(format!("version `{}` not found in manifest", current))
            })?;
            let json: VersionJson =
                fetch_json_with_mirrors(client, &self.mirror, &entry.url).await?;
            chain.push(json);
            match &chain.last().unwrap().inherits_from {
                Some(parent) => current = parent.clone(),
                None => break,
            }
        }
        chain.reverse();
        Ok(crate::game::version::merge_chain(&chain))
    }

    /// Fetch an assets index given its reference URL.
    pub async fn fetch_assets_index(
        &self,
        client: &reqwest::Client,
        url: &str,
    ) -> RcResult<AssetsIndex> {
        fetch_json_with_mirrors(client, &self.mirror, url).await
    }

    /// Resolve a version (walking `inheritsFrom`), fetch its asset index and
    /// return the complete, deduplicated [`DownloadPlan`] — client jar, asset
    /// index, libraries (+ natives), logging client **and** every asset object.
    ///
    /// This is the one-call convenience that turns a version id into a
    /// ready-to-download plan: it wraps [`Self::resolve_version`] +
    /// [`Self::fetch_assets_index`] + [`Self::build_full_plan`] into a single
    /// mirror-aware pipeline. Every URL inherits transparent mirror fallback
    /// from the [`crate::net::MirrorProvider`] held by this resolver.
    pub async fn resolve_full_plan(
        &self,
        client: &reqwest::Client,
        manifest: &VersionManifest,
        version_id: &str,
    ) -> RcResult<DownloadPlan> {
        let resolved = self.resolve_version(client, manifest, version_id).await?;
        let mut plan = self.build_plan(&resolved);
        if let Some(ai) = resolved.asset_index_ref() {
            let idx = self.fetch_assets_index(client, &ai.url).await?;
            for item in self.plan_assets(&idx) {
                plan.add(item);
            }
        }
        plan.dedup();
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::game::manifest::Latest;
    use crate::game::platform::{Arch, OsName, Platform};
    use crate::game::version::{AssetIndexRef, DownloadInfo, Downloads};
    use crate::net::{MirrorProvider, MirrorSource};

    /// Minimal blocking HTTP/1.0 test server; `handler` returns `(status, body)`.
    fn start_json_server(
        handler: impl Fn(&str) -> (u16, String) + Send + Sync + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let handler = Arc::new(handler);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 16384];
                let n = match stream.read(&mut buf) {
                    Ok(n) if n > 0 => n,
                    _ => continue,
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let (status, body) = handler(&path);
                let resp = format!(
                    "HTTP/1.0 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
            }
        });
        (format!("http://{addr}"), handle)
    }

    fn resolver() -> DependencyResolver {
        let platform = Platform {
            os: OsName::Linux,
            arch: Arch::Arm64,
            os_version: String::new(),
        };
        // Use a mirror so rewrite_all produces real candidate URLs.
        let mirror = MirrorProvider::new(vec![MirrorSource::new(
            "bmclapi",
            "BMCLAPI",
            "https://bmclapi2.bangbang93.com",
        )]);
        mirror.set_best("bmclapi");
        DependencyResolver::new(platform, Arc::new(mirror), "/data")
    }

    fn resolved() -> ResolvedVersion {
        let mut r = ResolvedVersion::default();
        r.id = "1.20.4".into();
        r.asset_index = Some(AssetIndexRef {
            id: "1.20".into(),
            sha1: Some("idxsha".into()),
            size: Some(10),
            total_size: None,
            url: "https://launchermeta.mojang.com/mc/assets/1.20.json".into(),
        });
        r.downloads = Downloads {
            client: Some(DownloadInfo {
                url: "https://piston-data.mojang.com/v1/objects/aa/client.jar".into(),
                sha1: Some("clientsha".into()),
                size: Some(100),
            }),
            ..Default::default()
        };
        // A normal library + a linux-only native library.
        r.libraries = vec![
            serde_json::from_str(r#"{"name":"com.mojang:patchy:1.1"}"#).unwrap(),
            serde_json::from_str(
                r#"{"name":"org.lwjgl:lwjgl-glfw:3.3.1","natives":{"linux":"natives-linux","windows":"natives-windows","osx":"natives-osx"}}"#,
            )
            .unwrap(),
            // osx-only library must be filtered out on linux.
            serde_json::from_str(
                r#"{"name":"com.example:maconly:1.0","rules":[{"action":"allow","os":{"name":"osx"}}]}"#,
            )
            .unwrap(),
        ];
        r
    }

    #[test]
    fn plan_has_client_assetindex_and_filtered_libraries() {
        let r = resolver();
        let plan = r.build_plan(&resolved());
        // client jar
        assert!(plan.items.iter().any(|i| i.kind == ArtifactKind::Client));
        // asset index
        assert!(plan
            .items
            .iter()
            .any(|i| i.kind == ArtifactKind::AssetIndex));
        // patchy + lwjgl-glfw main + lwjgl-glfw native-linux = 3 libraries
        let libs: Vec<_> = plan
            .items
            .iter()
            .filter(|i| i.kind == ArtifactKind::Library)
            .collect();
        assert_eq!(
            libs.len(),
            3,
            "expected patchy, lwjgl main and lwjgl native"
        );
        // maconly filtered out
        assert!(!plan.items.iter().any(|i| i.id.contains("maconly")));
        // native classifier resolved to linux
        assert!(plan.items.iter().any(|i| i.id.contains("natives-linux")));
        // mac native absent
        assert!(!plan.items.iter().any(|i| i.id.contains("natives-windows")));
    }

    #[test]
    fn plan_paths_and_mirrors() {
        let r = resolver();
        let plan = r.build_plan(&resolved());
        let client = plan
            .items
            .iter()
            .find(|i| i.kind == ArtifactKind::Client)
            .unwrap();
        assert_eq!(client.dest, Path::new("/data/versions/1.20.4/1.20.4.jar"));
        assert_eq!(client.checksum, Some(Checksum::Sha1("clientsha".into())));
        assert_eq!(client.size, Some(100));
        // mirror rewrite applied
        assert!(client
            .mirrors
            .iter()
            .any(|u| u.contains("bmclapi2.bangbang93.com")));
        // canonical url preserved
        assert!(client.url.contains("piston-data.mojang.com"));
    }

    #[test]
    fn asset_objects_planned() {
        let r = resolver();
        let idx: AssetsIndex = serde_json::from_str(
            r#"{"objects":{"a/b.ogg":{"hash":"0123456789abcdef0123456789abcdef01234567","size":42}}}"#,
        )
        .unwrap();
        let items = r.plan_assets(&idx);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.kind, ArtifactKind::AssetObject);
        assert_eq!(
            it.dest,
            Path::new("/data/assets/objects/01/0123456789abcdef0123456789abcdef01234567")
        );
        assert!(it.url.contains(
            "resources.download.minecraft.net/01/0123456789abcdef0123456789abcdef01234567"
        ));
    }

    #[test]
    fn dedup_collapses_shared_libraries() {
        let r = resolver();
        let res = resolved();
        let mut plan = r.build_plan(&res);
        // forcibly add a duplicate library item with same dest
        let dup = plan
            .items
            .iter()
            .find(|i| i.kind == ArtifactKind::Library)
            .cloned()
            .unwrap();
        let before = plan.len();
        plan.add(dup);
        assert_eq!(plan.len(), before, "duplicate dest should be merged");
    }

    #[test]
    fn full_plan_merges_version_and_assets_with_dedup() {
        let r = resolver();
        let mut res = resolved();
        // second library to make the plan non-trivial
        res.libraries.push(
            serde_json::from_str(r#"{"name":"com.google.code.findbugs:jsr305:3.0.2"}"#).unwrap(),
        );
        // Two asset objects that share the SAME hash must collapse to one file.
        let same_hash = "0123456789abcdef0123456789abcdef01234567";
        let idx: AssetsIndex = serde_json::from_str(&format!(
            r#"{{"objects":{{
                "minecraft/sounds/a.ogg":{{"hash":"{}","size":10}},
                "minecraft/sounds/b.ogg":{{"hash":"{}","size":20}}
            }}}}"#,
            same_hash, same_hash
        ))
        .unwrap();
        let plan = r.build_full_plan(&res, &idx);
        // version-level items: client + asset index + 3 libs (patchy, lwjgl main,
        // lwjgl native-linux, jsr305) = 1 + 1 + 4 = 6
        assert_eq!(plan.count_kind(ArtifactKind::Client), 1);
        assert_eq!(plan.count_kind(ArtifactKind::AssetIndex), 1);
        assert_eq!(plan.count_kind(ArtifactKind::Library), 4);
        // assets: two logical objects but one unique hash -> one physical file
        assert_eq!(plan.count_kind(ArtifactKind::AssetObject), 1);
        assert_eq!(plan.len(), 1 + 1 + 4 + 1);
        // every item carries a canonical url AND at least one mirror candidate
        for it in &plan.items {
            assert!(!it.url.is_empty(), "missing canonical url for {}", it.id);
            assert!(!it.mirrors.is_empty(), "missing mirror for {}", it.id);
        }
    }

    #[test]
    fn into_tasks_uses_canonical_url() {
        let r = resolver();
        let plan = r.build_plan(&resolved());
        let tasks = plan.into_tasks();
        let client = tasks
            .iter()
            .find(|t| t.dest.ends_with("1.20.4.jar"))
            .unwrap();
        assert!(client.url.contains("piston-data.mojang.com"));
        assert_eq!(client.checksum, Some(Checksum::Sha1("clientsha".into())));
    }

    #[test]
    fn legacy_asset_index_included_in_plan() {
        // A version that only declares the legacy `assets` string (no modern
        // `assetIndex` block) must still produce an asset-index download item
        // with the synthesised legacy URL.
        let r = resolver();
        let mut res = resolved();
        res.asset_index = None;
        res.assets = Some("1.12".into());
        let plan = r.build_plan(&res);
        let ai = plan
            .items
            .iter()
            .find(|i| i.kind == ArtifactKind::AssetIndex)
            .unwrap();
        assert_eq!(ai.id, "assetindex/1.12");
        assert_eq!(
            ai.url,
            "https://launchermeta.mojang.com/mc/assets/1.12/1.12.json"
        );
    }

    #[test]
    fn map_to_resources_uses_legacy_layout() {
        // Legacy `map_to_resources` indexes store objects under assets/resources/<name>.
        let r = resolver();
        let idx: AssetsIndex = serde_json::from_str(
            r#"{"map_to_resources":true,"objects":{"minecraft/sounds/a.ogg":{"hash":"0123456789abcdef0123456789abcdef01234567","size":42}}}"#,
        )
        .unwrap();
        let items = r.plan_assets(&idx);
        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.kind, ArtifactKind::AssetObject);
        assert_eq!(
            it.dest,
            Path::new("/data/assets/resources/minecraft/sounds/a.ogg")
        );
        assert!(it.url.contains("resources.download.minecraft.net"));
    }

    #[test]
    fn modern_assets_use_objects_layout() {
        let r = resolver();
        let idx: AssetsIndex = serde_json::from_str(
            r#"{"objects":{"minecraft/sounds/a.ogg":{"hash":"0123456789abcdef0123456789abcdef01234567","size":42}}}"#,
        )
        .unwrap();
        let items = r.plan_assets(&idx);
        assert_eq!(
            items[0].dest,
            Path::new("/data/assets/objects/01/0123456789abcdef0123456789abcdef01234567")
        );
    }

    #[tokio::test]
    async fn resolve_full_plan_end_to_end() {
        // Drive the whole task-4 pipeline against a local mirror: fetch the
        // manifest entry's version.json, walk (no inheritance here) and fetch
        // the asset index, then build a complete deduplicated plan.
        let version_json = r#"{
            "id":"1.20.4",
            "assetIndex":{"id":"1.20","sha1":"idxsha","size":10,"url":"https://launchermeta.mojang.com/mc/assets/1.20.json"},
            "downloads":{"client":{"url":"https://piston-data.mojang.com/v1/objects/aa/client.jar","sha1":"clientsha","size":100}},
            "mainClass":"net.minecraft.client.main.Main",
            "libraries":[{"name":"com.mojang:patchy:1.1"}]
        }"#;
        let asset_index_json = r#"{"objects":{"minecraft/sounds/a.ogg":{"hash":"0123456789abcdef0123456789abcdef01234567","size":42}}}"#;
        let (base, _h) = start_json_server(move |path| match path {
            p if p.ends_with("/1.20.4.json") => (200, version_json.to_string()),
            p if p.ends_with("/1.20.json") => (200, asset_index_json.to_string()),
            _ => (404, "not found".to_string()),
        });
        let provider = MirrorProvider::new(vec![MirrorSource::new("local", "Local", &base)]);
        let client = reqwest::Client::new();

        let manifest = VersionManifest {
            latest: Latest {
                release: String::new(),
                snapshot: String::new(),
            },
            versions: vec![crate::game::manifest::VersionEntry {
                id: "1.20.4".into(),
                kind: "release".into(),
                url: "https://piston-meta.mojang.com/v1/packages/abc/1.20.4.json".into(),
                sha1: None,
                time: None,
                release_time: None,
            }],
        };
        let r = DependencyResolver::new(Platform::android(), Arc::new(provider), "/data");
        let plan = r
            .resolve_full_plan(&client, &manifest, "1.20.4")
            .await
            .unwrap();

        // client + asset index + patchy library + 1 asset object = 4
        assert_eq!(plan.count_kind(ArtifactKind::Client), 1);
        assert_eq!(plan.count_kind(ArtifactKind::AssetIndex), 1);
        assert_eq!(plan.count_kind(ArtifactKind::Library), 1);
        assert_eq!(plan.count_kind(ArtifactKind::AssetObject), 1);
        assert_eq!(plan.len(), 4);
        // Every item carries a canonical url plus at least one mirror candidate.
        for it in &plan.items {
            assert!(!it.url.is_empty(), "empty url for {}", it.id);
            assert!(!it.mirrors.is_empty(), "no mirror for {}", it.id);
        }
    }
}
