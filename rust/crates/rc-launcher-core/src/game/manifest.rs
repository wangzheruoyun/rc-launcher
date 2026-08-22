//! Parsing of Mojang's top-level `version_manifest` (task 4).
//!
//! The manifest is the index of every published game version and lives at
//! <https://launchermeta.mojang.com/mc/game/version_manifest.json> (mirrored by
//! every China-mainland mirror). Each entry points at the per-version
//! `version.json` we resolve in [`crate::game::version`].

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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
