//! Library model & rule evaluation (task 4).
//!
//! A `version.json` lists its dependencies as [`Library`] entries. Each library
//! carries:
//!
//! * a Maven coordinate (`group:artifact:version[:classifier][@ext]`),
//! * optional `downloads` (artifact + per-classifier artifacts),
//! * optional `natives` (which classifier to pull for each OS),
//! * `rules` gating inclusion by platform / feature.
//!
//! This module knows how to parse those entries, evaluate the rules against the
//! current [`Platform`](crate::game::platform::Platform) and turn a library into
//! one or more concrete download URLs (main jar + optional native jar),
//! constructing the canonical Maven URL when the manifest only supplies a base
//! repository.

use std::collections::HashMap;

use crate::game::platform::{Features, OsRule, Platform};

/// Allow / disallow action of a [`Rule`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    #[default]
    Allow,
    Disallow,
}

/// A single download rule (os and/or feature gated).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct Rule {
    pub action: Action,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

impl Rule {
    /// Does this rule's conditions match the platform and feature state? When
    /// both `os` and `features` are present they must *both* match.
    pub fn matches(&self, platform: &Platform, features: &Features) -> bool {
        if let Some(os) = &self.os {
            if !os.matches(platform) {
                return false;
            }
        }
        if let Some(feats) = &self.features {
            for (k, want) in feats {
                let have = features.get(k).copied().unwrap_or(false);
                if have != *want {
                    return false;
                }
            }
        }
        true
    }
}

/// One downloadable artifact (the jar itself or a per-classifier native jar).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct Artifact {
    /// Maven relative path (group/artifact/version/...jar). Present when the
    /// artifact lives on the default or custom Maven repo.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Absolute download URL. When absent, build from [`Artifact::path`] and the
    /// owning library's repository base.
    #[serde(default)]
    pub url: Option<String>,
}

/// `downloads` block of a library.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: HashMap<String, Artifact>,
}

/// `extract` block: paths to skip when unpacking a native jar.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct ExtractRule {
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// A single dependency of a Minecraft version.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
pub struct Library {
    /// Maven coordinate `group:artifact:version[:classifier][@ext]`.
    pub name: String,
    /// Custom Maven repository base (overrides `libraries.minecraft.net`).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub downloads: LibraryDownloads,
    /// `os_name -> classifier` map selecting the native jar for each platform.
    #[serde(default)]
    pub natives: HashMap<String, String>,
    #[serde(default)]
    pub extract: Option<ExtractRule>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// A legacy `clientreq`/`serverreq` marker (older formats).
    #[serde(default, rename = "clientreq")]
    pub client_req: Option<bool>,
    #[serde(default)]
    pub serverreq: Option<bool>,
}

impl Library {
    /// Split the Maven coordinate into its parts.
    ///
    /// Returns `(group, artifact, version, classifier, extension)`.
    pub fn parse_maven(&self) -> (String, String, String, Option<String>, String) {
        let mut coord = self.name.as_str();
        let mut ext = "jar".to_string();
        if let Some((head, tail)) = coord.rsplit_once('@') {
            coord = head;
            ext = tail.to_string();
        }
        let mut it = coord.split(':');
        let group = it.next().unwrap_or("").to_string();
        let artifact = it.next().unwrap_or("").to_string();
        let version = it.next().unwrap_or("").to_string();
        let classifier = it.next().map(|c| c.to_string());
        (group, artifact, version, classifier, ext)
    }

    /// Maven relative path for this library, optionally with a classifier.
    pub fn maven_path(&self, classifier: Option<&str>) -> String {
        let (group, artifact, version, _cls, ext) = self.parse_maven();
        let group_path = group.replace('.', "/");
        match classifier {
            Some(c) => format!(
                "{}/{}/{}/{}-{}-{}.{}",
                group_path, artifact, version, artifact, version, c, ext
            ),
            None => format!(
                "{}/{}/{}/{}-{}.{}",
                group_path, artifact, version, artifact, version, ext
            ),
        }
    }

    /// The Maven repository base URL for this library.
    fn base_url(&self) -> String {
        self.url
            .clone()
            .unwrap_or_else(|| "https://libraries.minecraft.net".to_string())
    }

    /// Resolve the URL of the *main* jar (no classifier).
    ///
    /// Prefers an explicit `downloads.artifact.url`, then
    /// `downloads.artifact.path` joined onto the repository base, and finally a
    /// synthesised Maven URL from the coordinate. Returns `None` when the
    /// library is natives-only (no main jar to download).
    pub fn artifact_url(&self) -> Option<String> {
        if let Some(art) = &self.downloads.artifact {
            if let Some(u) = &art.url {
                return Some(u.clone());
            }
            if let Some(p) = &art.path {
                return Some(format!("{}/{}", self.base_url().trim_end_matches('/'), p));
            }
        }
        // No explicit artifact info: synthesise the Maven URL from the
        // coordinate. A classifier-only coordinate (e.g. `group:artifact:ver:
        // natives-linux`) has no main jar, so skip it; otherwise the main jar
        // lives at the default/custom repository.
        let (_g, _a, _v, classifier, _e) = self.parse_maven();
        if classifier.is_some() {
            return None;
        }
        Some(format!(
            "{}/{}",
            self.base_url().trim_end_matches('/'),
            self.maven_path(None)
        ))
    }

    /// Resolve the URL of a native jar for `classifier` (e.g. `natives-linux`).
    pub fn classifier_url(&self, classifier: &str) -> Option<String> {
        if let Some(art) = self.downloads.classifiers.get(classifier) {
            if let Some(u) = &art.url {
                return Some(u.clone());
            }
            if let Some(p) = &art.path {
                return Some(format!("{}/{}", self.base_url().trim_end_matches('/'), p));
            }
        }
        Some(format!(
            "{}/{}",
            self.base_url().trim_end_matches('/'),
            self.maven_path(Some(classifier))
        ))
    }

    /// The native classifier to download for the given platform, per `natives`.
    pub fn native_classifier(&self, platform: &Platform) -> Option<String> {
        let key = match platform.os {
            crate::game::platform::OsName::Linux => "linux",
            crate::game::platform::OsName::Windows => "windows",
            crate::game::platform::OsName::Osx => "osx",
        };
        self.natives.get(key).cloned()
    }

    /// Should this library (and its artifacts) be included for the platform?
    ///
    /// No rules => always included. With rules => excluded unless the last
    /// matching rule's action is `allow`.
    pub fn is_allowed(&self, platform: &Platform, features: &Features) -> bool {
        if self.rules.is_empty() {
            return true;
        }
        let mut allowed = false;
        for rule in &self.rules {
            if rule.matches(platform, features) {
                allowed = rule.action == Action::Allow;
            }
        }
        allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::platform::{Arch, OsName, Platform};

    fn plat(os: OsName, arch: Arch) -> Platform {
        Platform {
            os,
            arch,
            os_version: String::new(),
        }
    }

    #[test]
    fn parse_maven_coordinate() {
        let lib: Library = serde_json::from_str(r#"{"name":"com.mojang:patchy:1.1"}"#).unwrap();
        assert_eq!(lib.parse_maven().0, "com.mojang");
        assert_eq!(lib.parse_maven().1, "patchy");
        assert_eq!(lib.parse_maven().2, "1.1");
        assert_eq!(lib.maven_path(None), "com/mojang/patchy/1.1/patchy-1.1.jar");
    }

    #[test]
    fn parse_maven_with_classifier_and_ext() {
        let lib: Library =
            serde_json::from_str(r#"{"name":"org.lwjgl:lwjgl:3.3.1:natives-linux@jar"}"#).unwrap();
        let (g, a, v, c, e) = lib.parse_maven();
        assert_eq!(
            (g.as_str(), a.as_str(), v.as_str(), c.as_deref(), e.as_str()),
            ("org.lwjgl", "lwjgl", "3.3.1", Some("natives-linux"), "jar")
        );
        assert_eq!(
            lib.maven_path(Some("natives-linux")),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-linux.jar"
        );
    }

    #[test]
    fn artifact_url_default_repo() {
        let lib: Library = serde_json::from_str(r#"{"name":"com.mojang:patchy:1.1"}"#).unwrap();
        assert_eq!(
            lib.artifact_url(),
            Some("https://libraries.minecraft.net/com/mojang/patchy/1.1/patchy-1.1.jar".into())
        );
    }

    #[test]
    fn artifact_url_explicit_and_custom_repo() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"net.fabricmc:fabric-loader:0.15.0","url":"https://maven.fabricmc.net/","downloads":{"artifact":{"url":"https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.0/fabric-loader-0.15.0.jar"}}}"#,
        )
        .unwrap();
        assert_eq!(
            lib.artifact_url(),
            Some("https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.0/fabric-loader-0.15.0.jar".into())
        );
    }

    #[test]
    fn natives_selection_and_url() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"org.lwjgl:lwjgl-glfw:3.3.1","natives":{"linux":"natives-linux","windows":"natives-windows","osx":"natives-osx"}}"#,
        )
        .unwrap();
        let linux = plat(OsName::Linux, Arch::X86_64);
        assert_eq!(
            lib.native_classifier(&linux).as_deref(),
            Some("natives-linux")
        );
        let url = lib.classifier_url("natives-linux").unwrap();
        assert_eq!(
            url,
            "https://libraries.minecraft.net/org/lwjgl/lwjgl-glfw/3.3.1/lwjgl-glfw-3.3.1-natives-linux.jar"
        );

        let win = plat(OsName::Windows, Arch::X86_64);
        assert_eq!(
            lib.native_classifier(&win).as_deref(),
            Some("natives-windows")
        );
    }

    #[test]
    fn rule_allow_linux_disallow_osx() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"x:y:1","rules":[{"action":"allow","os":{"name":"linux"}},{"action":"disallow","os":{"name":"osx"}}]}"#,
        )
        .unwrap();
        let linux = plat(OsName::Linux, Arch::X86_64);
        let osx = plat(OsName::Osx, Arch::X86_64);
        assert!(lib.is_allowed(&linux, &Features::new()));
        assert!(!lib.is_allowed(&osx, &Features::new()));
    }

    #[test]
    fn no_rules_means_included() {
        let lib: Library = serde_json::from_str(r#"{"name":"x:y:1"}"#).unwrap();
        assert!(lib.is_allowed(&plat(OsName::Linux, Arch::Arm64), &Features::new()));
        assert!(lib.is_allowed(&plat(OsName::Windows, Arch::X86_64), &Features::new()));
    }

    #[test]
    fn disallow_default_when_no_rule_matches() {
        // Rule only allows windows; on linux nothing matches -> excluded.
        let lib: Library = serde_json::from_str(
            r#"{"name":"x:y:1","rules":[{"action":"allow","os":{"name":"windows"}}]}"#,
        )
        .unwrap();
        assert!(!lib.is_allowed(&plat(OsName::Linux, Arch::Arm64), &Features::new()));
    }

    #[test]
    fn classifier_only_coordinate_has_no_main_artifact() {
        // A coordinate that already carries a classifier (natives-linux) has no
        // separate main jar to download.
        let lib: Library =
            serde_json::from_str(r#"{"name":"org.lwjgl:lwjgl:3.3.1:natives-linux"}"#).unwrap();
        assert!(lib.artifact_url().is_none());
        // but the classifier URL is still constructible
        assert!(lib.classifier_url("natives-linux").is_some());
    }

    #[test]
    fn windows_native_selected_on_windows() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"org.lwjgl:lwjgl-glfw:3.3.1","natives":{"linux":"natives-linux","windows":"natives-windows","osx":"natives-osx"}}"#,
        )
        .unwrap();
        let win = plat(OsName::Windows, Arch::X86_64);
        assert_eq!(
            lib.native_classifier(&win).as_deref(),
            Some("natives-windows")
        );
        let url = lib.classifier_url("natives-windows").unwrap();
        assert!(url.contains("natives-windows.jar"));
        // linux classifier must NOT be selected on windows
        assert!(!lib
            .native_classifier(&win)
            .map(|c| c.contains("linux"))
            .unwrap_or(false));
    }

    #[test]
    fn custom_repo_artifact_url() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"net.fabricmc:fabric-loader:0.15.0","url":"https://maven.fabricmc.net/"}"#,
        )
        .unwrap();
        let url = lib.artifact_url().unwrap();
        assert!(url.starts_with("https://maven.fabricmc.net/"));
        assert!(url.ends_with("fabric-loader-0.15.0.jar"));
    }

    #[test]
    fn feature_rule_gates_download() {
        let lib: Library = serde_json::from_str(
            r#"{"name":"x:y:1","rules":[{"action":"allow","features":{"is_demo_user":true}}]}"#,
        )
        .unwrap();
        let p = plat(OsName::Linux, Arch::Arm64);
        assert!(!lib.is_allowed(&p, &Features::new()));
        let mut f = Features::new();
        f.insert("is_demo_user".into(), true);
        assert!(lib.is_allowed(&p, &f));
    }
}
