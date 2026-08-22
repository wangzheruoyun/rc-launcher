//! Assets index parsing (task 4).
//!
//! The `assetIndex` of a version points at a JSON document listing every
//! resource object (`<sha1>` -> size). Those objects are downloaded from
//! `https://resources.download.minecraft.net/<sha1[0..2]>/<sha1>` and stored
//! under `assets/objects/<sha1[0..2]>/<sha1>`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A single resource object in the assets index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetObject {
    /// SHA-1 hash of the object (also its storage key).
    pub hash: String,
    pub size: u64,
}

/// A parsed assets index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetsIndex {
    #[serde(default)]
    pub id: Option<String>,
    pub objects: HashMap<String, AssetObject>,
    /// Legacy flag: map objects directly into `resources/` instead of
    /// `assets/objects` (very old versions, 1.5.x and earlier).
    #[serde(default)]
    pub map_to_resources: Option<bool>,
}

impl AssetsIndex {
    pub fn parse(json: &str) -> crate::error::RcResult<Self> {
        serde_json::from_str(json).map_err(crate::error::RcError::Json)
    }

    /// Number of objects in the index.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "id":"1.20",
        "objects":{
            "minecraft/sounds/ambient/weather.ogg":{"hash":"0123456789abcdef0123456789abcdef01234567","size":4096},
            "minecraft/font/unicode_page_00.png":{"hash":"fedcba9876543210fedcba9876543210fedcba98","size":2048}
        }
    }"#;

    #[test]
    fn parse_assets_index() {
        let idx = AssetsIndex::parse(SAMPLE).unwrap();
        assert_eq!(idx.id.as_deref(), Some("1.20"));
        assert_eq!(idx.len(), 2);
        let first = idx
            .objects
            .get("minecraft/sounds/ambient/weather.ogg")
            .unwrap();
        assert_eq!(first.hash, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(first.size, 4096);
    }

    #[test]
    fn empty_index() {
        let idx: AssetsIndex = serde_json::from_str(r#"{"objects":{}}"#).unwrap();
        assert!(idx.is_empty());
    }
}
