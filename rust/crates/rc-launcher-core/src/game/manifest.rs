//! Parsing of Mojang's top-level `version_manifest` (task 4).
//!
//! The manifest is the index of every published game version and lives at
//! <https://launchermeta.mojang.com/mc/game/version_manifest.json> (mirrored by
//! every China-mainland mirror). Each entry points at the per-version
//! `version.json` we resolve in [`crate::game::version`].

use crate::error::RcResult;
use crate::game::version::fetch_json_with_mirrors;
use crate::net::MirrorProvider;

/// Canonical Mojang version-manifest URL. Mirrors rewrite the host onto their
/// own path-preserving CDN (see [`crate::net::mirror`]), so the same URL works
/// on the origin and on BMCLAPI/MCBBS/Aliyun.
pub const VERSION_MANIFEST_URL: &str =
    "https://launchermeta.mojang.com/mc/game/version_manifest.json";

/// The full version manifest.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct VersionManifest {
    pub latest: Latest,
    pub versions: Vec<VersionEntry>,
}

/// `latest.release` / `latest.snapshot` pointers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Latest {
    pub release: String,
    pub snapshot: String,
}

/// One entry in the manifest: an id, its type and the URL of its `version.json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct VersionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    /// URL of the per-version `version.json` (on `piston-meta.mojang.com`).
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub release_time: Option<String>,
}

impl VersionManifest {
    /// Find an entry by its exact version id (e.g. `"1.20.4"`).
    pub fn find(&self, id: &str) -> Option<&VersionEntry> {
        self.versions.iter().find(|v| v.id == id)
    }

    /// The [`VersionEntry`] for `latest.release`.
    pub fn latest_release(&self) -> Option<&VersionEntry> {
        self.find(&self.latest.release)
    }

    /// The [`VersionEntry`] for `latest.snapshot`.
    pub fn latest_snapshot(&self) -> Option<&VersionEntry> {
        self.find(&self.latest.snapshot)
    }

    /// Convenience: the URL of a version's `version.json` by id.
    pub fn url_of(&self, id: &str) -> Option<&str> {
        self.find(id).map(|e| e.url.as_str())
    }

    /// Fetch and parse the live version manifest from the network, transparently
    /// retrying against the China-mainland mirrors. This is the entry point of
    /// the dependency-resolution pipeline (task 4): callers then resolve a
    /// specific version and build a download plan from it.
    pub async fn fetch(client: &reqwest::Client, mirror: &MirrorProvider) -> RcResult<Self> {
        fetch_json_with_mirrors(client, mirror, VERSION_MANIFEST_URL).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    use crate::net::MirrorProvider;

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

    const SAMPLE: &str = r#"{
        "latest": { "release": "1.20.4", "snapshot": "24w03a" },
        "versions": [
            { "id": "1.20.4", "type": "release", "url": "https://piston-meta.mojang.com/v1/packages/abc/1.20.4.json", "sha1": "deadbeef" },
            { "id": "24w03a", "type": "snapshot", "url": "https://piston-meta.mojang.com/v1/packages/def/24w03a.json" }
        ]
    }"#;

    #[test]
    fn parse_manifest() {
        let m: VersionManifest = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(m.latest.release, "1.20.4");
        assert_eq!(m.latest.snapshot, "24w03a");
        assert_eq!(m.versions.len(), 2);
        assert_eq!(m.versions[0].id, "1.20.4");
        assert_eq!(m.versions[0].kind, "release");
        assert_eq!(
            m.versions[0].url,
            "https://piston-meta.mojang.com/v1/packages/abc/1.20.4.json"
        );
        assert_eq!(m.versions[0].sha1.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn find_entries() {
        let m: VersionManifest = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(m.find("1.20.4").unwrap().kind, "release");
        assert_eq!(m.latest_release().unwrap().id, "1.20.4");
        assert_eq!(m.latest_snapshot().unwrap().id, "24w03a");
        assert_eq!(
            m.url_of("24w03a"),
            Some("https://piston-meta.mojang.com/v1/packages/def/24w03a.json")
        );
        assert!(m.find("nope").is_none());
    }

    #[tokio::test]
    async fn fetch_uses_mirror_and_parses_manifest() {
        // Mirror the canonical manifest host onto a local server and confirm
        // VersionManifest::fetch downloads + parses through the mirror.
        let manifest_json = r#"{
            "latest": { "release": "1.20.4", "snapshot": "24w03a" },
            "versions": [
                { "id": "1.20.4", "type": "release", "url": "https://piston-meta.mojang.com/v1/packages/abc/1.20.4.json" }
            ]
        }"#;
        let (base, _h) = start_json_server(move |_path| (200, manifest_json.to_string()));
        let provider =
            MirrorProvider::new(vec![crate::net::MirrorSource::new("local", "Local", &base)]);
        let client = reqwest::Client::new();
        let m = VersionManifest::fetch(&client, &provider).await.unwrap();
        assert_eq!(m.latest.release, "1.20.4");
        assert_eq!(m.versions.len(), 1);
        assert_eq!(m.versions[0].id, "1.20.4");
    }
}
