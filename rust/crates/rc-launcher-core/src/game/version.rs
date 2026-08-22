//! Per-version `version.json` parsing, inheritance resolution & async fetch
//! (task 4).
//!
//! A `version.json` is the full description of one game version: its main class,
//! launch arguments, the `client` jar, its `assetIndex`, the `libraries` list
//! and (for modded profiles) an `inheritsFrom` pointer to a parent version.
//!
//! [`ResolvedVersion`] is the *merged* view after walking the `inheritsFrom`
//! chain (child overrides parent), which is exactly what FCLCore/game and
//! FCLCore/util do when building a launchable profile.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};
use crate::game::library::Library;
use crate::net::MirrorProvider;

/// A reference to the assets index for a version (modern format).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub total_size: Option<u64>,
    pub url: String,
}

/// A single downloadable file (client jar, server jar, mappings, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadInfo {
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

/// The `downloads` block of a `version.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Downloads {
    #[serde(default)]
    pub client: Option<DownloadInfo>,
    #[serde(default)]
    pub server: Option<DownloadInfo>,
    #[serde(default)]
    pub client_mappings: Option<DownloadInfo>,
    #[serde(default)]
    pub server_mappings: Option<DownloadInfo>,
}

/// `java_version` requirement of a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaVersion {
    #[serde(default)]
    pub major_version: Option<u32>,
    #[serde(default)]
    pub name: Option<String>,
}

/// Game / JVM argument lists (new-style `arguments` block).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VersionArguments {
    #[serde(default)]
    pub game: Vec<serde_json::Value>,
    #[serde(default)]
    pub jvm: Vec<serde_json::Value>,
}

/// A parsed `version.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VersionJson {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    /// Parent version this one inherits from (Forge/Fabric/OptiFine profiles).
    #[serde(default, rename = "inheritsFrom")]
    pub inherits_from: Option<String>,
    #[serde(default, rename = "assetIndex")]
    pub asset_index: Option<AssetIndexRef>,
    /// Legacy assets id (pre-`assetIndex` format).
    #[serde(default)]
    pub assets: Option<String>,
    /// Legacy top-level `client`/`server` references.
    #[serde(default)]
    pub client: Option<DownloadInfo>,
    #[serde(default)]
    pub server: Option<DownloadInfo>,
    #[serde(default)]
    pub downloads: Downloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default, rename = "mainClass")]
    pub main_class: Option<String>,
    #[serde(default, rename = "minecraftArguments")]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub arguments: Option<VersionArguments>,
    #[serde(default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub logging: Option<serde_json::Value>,
    #[serde(default, rename = "minimumLauncherVersion")]
    pub minimum_launcher_version: Option<u32>,
}

/// The fully-merged view of a version after resolving `inheritsFrom`.
#[derive(Debug, Clone, Default)]
pub struct ResolvedVersion {
    pub id: String,
    pub kind: Option<String>,
    pub asset_index: Option<AssetIndexRef>,
    pub assets: Option<String>,
    pub downloads: Downloads,
    /// Libraries, deduplicated by Maven coordinate (child overrides parent).
    pub libraries: Vec<Library>,
    pub main_class: Option<String>,
    pub minecraft_arguments: Option<String>,
    pub arguments: Option<VersionArguments>,
    pub java_version: Option<JavaVersion>,
    pub logging: Option<serde_json::Value>,
}

impl VersionJson {
    /// Parse a `version.json` from a JSON string.
    pub fn parse(json: &str) -> RcResult<Self> {
        serde_json::from_str(json).map_err(RcError::Json)
    }
}

/// Walk an `inheritsFrom` chain (root first, leaf last) and merge the fields.
///
/// * Scalar fields: the leaf (instance) value wins; absent values fall back to
///   the nearest ancestor that defines them.
/// * `libraries`: concatenated parent-first; a library redefined by a child
///   replaces the parent's entry *in place* (so classpath order is stable).
pub fn merge_chain(chain: &[VersionJson]) -> ResolvedVersion {
    let mut resolved = ResolvedVersion::default();
    // Scalars: iterate leaf -> root so the leaf wins.
    for v in chain.iter().rev() {
        if resolved.id.is_empty() {
            resolved.id = v.id.clone().unwrap_or_default();
        }
        resolved.kind = resolved.kind.take().or_else(|| v.kind.clone());
        resolved.asset_index = resolved
            .asset_index
            .take()
            .or_else(|| v.asset_index.clone());
        resolved.assets = resolved.assets.take().or_else(|| v.assets.clone());
        resolved.downloads.client = resolved
            .downloads
            .client
            .take()
            .or_else(|| v.downloads.client.clone());
        resolved.downloads.server = resolved
            .downloads
            .server
            .take()
            .or_else(|| v.downloads.server.clone());
        resolved.downloads.client_mappings = resolved
            .downloads
            .client_mappings
            .take()
            .or_else(|| v.downloads.client_mappings.clone());
        resolved.downloads.server_mappings = resolved
            .downloads
            .server_mappings
            .take()
            .or_else(|| v.downloads.server_mappings.clone());
        resolved.main_class = resolved.main_class.take().or_else(|| v.main_class.clone());
        resolved.minecraft_arguments = resolved
            .minecraft_arguments
            .take()
            .or_else(|| v.minecraft_arguments.clone());
        resolved.arguments = resolved.arguments.take().or_else(|| v.arguments.clone());
        resolved.java_version = resolved
            .java_version
            .take()
            .or_else(|| v.java_version.clone());
        resolved.logging = resolved.logging.take().or_else(|| v.logging.clone());
        // legacy top-level client/server
        if resolved.downloads.client.is_none() {
            resolved.downloads.client = v.client.clone();
        }
        if resolved.downloads.server.is_none() {
            resolved.downloads.server = v.server.clone();
        }
    }
    // Libraries: parent first, child overrides in place.
    let mut libs: Vec<Library> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for v in chain {
        for lib in &v.libraries {
            if let Some(&idx) = seen.get(&lib.name) {
                libs[idx] = lib.clone();
            } else {
                seen.insert(lib.name.clone(), libs.len());
                libs.push(lib.clone());
            }
        }
    }
    resolved.libraries = libs;
    resolved
}

/// Resolve the inheritance chain for `start_id` by fetching each `version.json`
/// through `fetch_json` (which returns the parsed JSON for a given id).
///
/// Returns the merged [`ResolvedVersion`], or an error on a cycle / missing
/// parent / parse failure.
pub fn resolve_version_chain(
    start_id: &str,
    mut fetch_json: impl FnMut(&str) -> RcResult<VersionJson>,
) -> RcResult<ResolvedVersion> {
    let mut chain: Vec<VersionJson> = Vec::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut current = start_id.to_string();
    loop {
        if !visited.insert(current.clone()) {
            return Err(RcError::Other(format!(
                "inheritance cycle detected at version `{}`",
                current
            )));
        }
        let json = fetch_json(&current)?;
        chain.push(json.clone());
        match &json.inherits_from {
            Some(parent) => current = parent.clone(),
            None => break,
        }
    }
    chain.reverse(); // root first, leaf last
    Ok(merge_chain(&chain))
}

/// Fetch a `version.json` (or any JSON document) from `url`, transparently
/// retrying against the China-mainland mirrors in priority order.
pub async fn fetch_json_with_mirrors<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    mirror: &MirrorProvider,
    url: &str,
) -> RcResult<T> {
    let mut candidates: Vec<String> = mirror.rewrite_all(url);
    if candidates.is_empty() {
        candidates.push(url.to_string());
    }
    let mut last_err: Option<String> = None;
    for c in &candidates {
        match client.get(c).send().await {
            Ok(resp) if resp.status().is_success() => {
                let text = resp
                    .text()
                    .await
                    .map_err(|e| RcError::Network(e.to_string()))?;
                return serde_json::from_str(&text).map_err(RcError::Json);
            }
            Ok(resp) => {
                last_err = Some(format!("HTTP {}", resp.status()));
            }
            Err(e) => {
                last_err = Some(e.to_string());
            }
        }
    }
    Err(RcError::Other(format!(
        "failed to fetch {}: {:?}",
        url, last_err
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v1() -> VersionJson {
        serde_json::from_str(
            r#"{
                "id":"1.20.4",
                "type":"release",
                "assetIndex":{"id":"1.20","sha1":"abc","size":1,"url":"https://launchermeta.mojang.com/mc/assets/1.20.json"},
                "downloads":{"client":{"url":"https://piston-data.mojang.com/v1/objects/aa/client.jar","sha1":"aa","size":100}},
                "mainClass":"net.minecraft.client.main.Main",
                "libraries":[{"name":"com.mojang:patchy:1.1"}]
            }"#,
        )
        .unwrap()
    }

    fn forge() -> VersionJson {
        serde_json::from_str(
            r#"{
                "id":"1.20.4-forge-49.0.0",
                "inheritsFrom":"1.20.4",
                "mainClass":"net.minecraftforge.fml.common.launcher.FMLTweaker",
                "libraries":[
                    {"name":"net.minecraftforge:forge:1.20.4-49.0.0"},
                    {"name":"com.mojang:patchy:1.1","url":"https://custom.example/patchy.jar"}
                ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn merge_overrides_scalars_and_libraries() {
        let chain = vec![v1(), forge()];
        let merged = merge_chain(&chain);
        // child main class wins
        assert_eq!(
            merged.main_class.as_deref(),
            Some("net.minecraftforge.fml.common.launcher.FMLTweaker")
        );
        // asset index inherited from parent (child doesn't define one)
        assert_eq!(merged.asset_index.as_ref().unwrap().id, "1.20");
        // client download inherited from parent
        assert!(merged.downloads.client.is_some());
        // libraries: patchy overridden in place (custom url), forge added
        assert_eq!(merged.libraries.len(), 2);
        let patchy = merged
            .libraries
            .iter()
            .find(|l| l.name == "com.mojang:patchy:1.1")
            .unwrap();
        assert_eq!(
            patchy.url.as_deref(),
            Some("https://custom.example/patchy.jar")
        );
        assert!(merged
            .libraries
            .iter()
            .any(|l| l.name == "net.minecraftforge:forge:1.20.4-49.0.0"));
    }

    #[test]
    fn resolve_chain_uses_fetch_closure() {
        let mut db: HashMap<String, VersionJson> = HashMap::new();
        db.insert("1.20.4".into(), v1());
        db.insert("1.20.4-forge-49.0.0".into(), forge());
        let resolved = resolve_version_chain("1.20.4-forge-49.0.0", |id| {
            db.get(id)
                .cloned()
                .ok_or_else(|| RcError::Other(format!("missing {id}")))
        })
        .unwrap();
        assert_eq!(resolved.id, "1.20.4-forge-49.0.0");
        assert_eq!(resolved.libraries.len(), 2);
    }

    #[test]
    fn resolve_chain_detects_cycle() {
        let cyclic: VersionJson = serde_json::from_str(r#"{"id":"a","inheritsFrom":"b"}"#).unwrap();
        let b = serde_json::from_str(r#"{"id":"b","inheritsFrom":"a"}"#).unwrap();
        let mut db: HashMap<String, VersionJson> = HashMap::new();
        db.insert("a".into(), cyclic);
        db.insert("b".into(), b);
        let r = resolve_version_chain("a", |id| {
            db.get(id)
                .cloned()
                .ok_or_else(|| RcError::Other("x".into()))
        });
        assert!(r.is_err());
    }
}
