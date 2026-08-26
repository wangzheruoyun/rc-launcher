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

impl ResolvedVersion {
    /// The asset index this version needs, preferring the modern `assetIndex`
    /// block and falling back to the legacy `assets` string id (which maps to
    /// `https://launchermeta.mojang.com/mc/assets/<id>/<id>.json`).
    ///
    /// Returning a synthesised [`AssetIndexRef`] for legacy versions keeps the
    /// resolver pipeline uniform: the caller can always fetch *something* and
    /// build the asset-object plan, instead of special-casing old releases.
    pub fn asset_index_ref(&self) -> Option<AssetIndexRef> {
        if let Some(ai) = &self.asset_index {
            return Some(ai.clone());
        }
        self.assets.as_ref().map(|id| AssetIndexRef {
            id: id.clone(),
            sha1: None,
            size: None,
            total_size: None,
            url: format!(
                "https://launchermeta.mojang.com/mc/assets/{}/{}.json",
                id, id
            ),
        })
    }

    /// The minimum Java major version required to run this version, inferred
    /// from the `java_version` block so the JRE provider (task 6) can pick a
    /// compatible runtime. Modern releases declare `major_version` directly
    /// (e.g. 17 for 1.18+); older ones reference a component name
    /// (`java-runtime-alpha` -> 17, `jre-legacy` -> 8). Unknown versions
    /// conservatively assume Java 8 (the historical default).
    pub fn required_java_major(&self) -> u32 {
        if let Some(jv) = &self.java_version {
            if let Some(m) = jv.major_version {
                if m > 0 {
                    return m;
                }
            }
            if let Some(name) = &jv.name {
                return match name.as_str() {
                    "jre-legacy" | "jre-8" | "java-8" => 8,
                    "java-16" | "java-runtime-delta" | "java-runtime-beta" => 16,
                    "java-17" | "java-runtime-alpha" => 17,
                    "java-21" => 21,
                    "java-25" | "java-runtime-gamma" => 25,
                    _ => 8,
                };
            }
        }
        8
    }
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
///
/// Robustness: a mirror may answer with HTTP 200 but a *non-JSON* body — a captive
/// portal, a CDN error page, a half-cached HTML stub — especially on the
/// flaky domestic networks this launcher targets. A naïve implementation would
/// parse-fail and abort; instead we treat a successful-but-unparseable response
/// the same as a transport failure and **fall through to the next candidate**
/// (the next mirror, or the origin). We only give up after every candidate has
/// been tried, reporting the last error seen. This is the single most important
/// resilience property for mirror-based downloads.
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
        let resp = match client.get(c).send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(format!("request to {c} failed: {e}"));
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = Some(format!("HTTP {} from {c}", resp.status()));
            continue;
        }
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                last_err = Some(format!("body read from {c} failed: {e}"));
                continue;
            }
        };
        match serde_json::from_str(&text) {
            Ok(v) => return Ok(v),
            Err(e) => {
                // 200 OK but invalid JSON: a polluted mirror, not a real result.
                // Try the next candidate instead of surfacing a parse error.
                last_err = Some(format!("parse from {c} failed ({} bytes): {e}", text.len()));
                continue;
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

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    use crate::net::MirrorProvider;

    /// Minimal blocking HTTP/1.0 test server (no extra deps). `handler` returns
    /// `(status_code, body)` for a request, given its path.
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

    #[test]
    fn required_java_major_inferred() {
        // No java_version block -> conservative Java 8 default.
        let mut rv = ResolvedVersion::default();
        assert_eq!(rv.required_java_major(), 8);

        // Explicit major_version wins.
        rv.java_version = Some(JavaVersion {
            major_version: Some(21),
            name: None,
        });
        assert_eq!(rv.required_java_major(), 21);

        // Component name mapping.
        rv.java_version = Some(JavaVersion {
            major_version: None,
            name: Some("java-runtime-alpha".into()),
        });
        assert_eq!(rv.required_java_major(), 17);

        rv.java_version = Some(JavaVersion {
            major_version: None,
            name: Some("jre-legacy".into()),
        });
        assert_eq!(rv.required_java_major(), 8);

        rv.java_version = Some(JavaVersion {
            major_version: Some(0),
            name: Some("java-21".into()),
        });
        // major_version == 0 is treated as "unknown", fall back to name.
        assert_eq!(rv.required_java_major(), 21);
    }

    #[test]
    fn asset_index_ref_legacy_fallback() {
        // Modern block is preferred.
        let mut rv = ResolvedVersion::default();
        rv.asset_index = Some(AssetIndexRef {
            id: "1.20".into(),
            sha1: Some("abc".into()),
            size: Some(1),
            total_size: None,
            url: "https://launchermeta.mojang.com/mc/assets/1.20.json".into(),
        });
        let ai = rv.asset_index_ref().unwrap();
        assert_eq!(ai.id, "1.20");
        assert_eq!(
            ai.url,
            "https://launchermeta.mojang.com/mc/assets/1.20.json"
        );

        // Legacy `assets` string synthesises the standard index URL.
        let mut rv2 = ResolvedVersion::default();
        rv2.assets = Some("1.12".into());
        let ai2 = rv2.asset_index_ref().unwrap();
        assert_eq!(ai2.id, "1.12");
        assert_eq!(
            ai2.url,
            "https://launchermeta.mojang.com/mc/assets/1.12/1.12.json"
        );

        // Neither -> none.
        assert!(ResolvedVersion::default().asset_index_ref().is_none());
    }

    #[tokio::test]
    async fn fetch_json_falls_through_polluted_mirror() {
        // One mirror (best) returns an HTML error page; the other returns valid
        // JSON. The resolver must fall through the polluted mirror to the good
        // one instead of surfacing a parse error.
        let (base, _h) = start_json_server(|path| {
            if path.contains("/polluted/") {
                (200, "<html>502 Bad Gateway</html>".to_string())
            } else {
                (200, "{\"hello\":\"world\"}".to_string())
            }
        });
        let provider = MirrorProvider::new(vec![
            crate::net::MirrorSource::new("polluted", "Polluted", &base)
                .with_path_prefix("polluted"),
            crate::net::MirrorSource::new("good", "Good", &base).with_path_prefix("good"),
        ]);
        provider.set_best("polluted");

        let client = reqwest::Client::new();
        let url = "https://launchermeta.mojang.com/mc/game/test.json";
        let val: serde_json::Value = fetch_json_with_mirrors(&client, &provider, url)
            .await
            .unwrap();
        assert_eq!(val["hello"], "world");
    }

    #[tokio::test]
    async fn fetch_json_errors_when_all_polluted() {
        // Both candidates return HTML -> ultimate failure (not a silent ok).
        let (base, _h) = start_json_server(|_path| (200, "<html>error</html>".to_string()));
        let provider = MirrorProvider::new(vec![crate::net::MirrorSource::new(
            "polluted", "Polluted", &base,
        )]);
        let client = reqwest::Client::new();
        let url = "https://launchermeta.mojang.com/mc/game/test.json";
        let res: RcResult<serde_json::Value> =
            fetch_json_with_mirrors(&client, &provider, url).await;
        assert!(res.is_err());
    }
}
