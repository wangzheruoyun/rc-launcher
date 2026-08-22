//! Classpath & native-library resolution (task 7).
//!
//! Turning a resolved `version.json` into a JVM classpath on Android needs three
//! things the desktop launchers do not:
//!
//! 1. **Rule filtering** — only libraries allowed for the target platform go on
//!    the classpath (shared with the task-4 download planner).
//! 2. **LWJGL substitution** — a vanilla `version.json` pins `org.lwjgl:*` jars
//!    whose bundled natives are Windows/Linux/macOS `.dll`/`.so`/`.dylib` builds.
//!    They cannot load on Android, so (exactly like FCL) they are dropped and
//!    replaced by the prebuilt bundle in `app_runtime/lwjgl/<version>/`, whose
//!    `natives/<abi>/*.so` are real Android libraries.
//! 3. **Duplicate collapsing** — modded profiles (Forge/Fabric) frequently
//!    declare two versions of the same Maven artifact; the JVM would load the
//!    first one on the classpath, so we keep the *highest* version to avoid
//!    `NoSuchMethodError` at runtime.
//!
//! The resulting [`Classpath`] also records the native jars that still need
//! extracting into `${natives_directory}` and which entries were substituted /
//! are missing on disk, so [`crate::launch::engine`] can fail with a precise,
//! actionable error instead of letting the JVM die.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{RcError, RcResult};
use crate::game::library::Library;
use crate::game::platform::{Features, Platform};
use crate::game::ResolvedVersion;
use crate::launch::awt::{AwtBackend, CacioBundle};
use crate::launch::options::LwjglVersion;
use crate::launch::runtime_assets::AppRuntime;
use crate::runtime::{Abi, JavaVersion};

/// Maven coordinate prefixes replaced by the prebuilt Android bundle.
///
/// * `org.lwjgl*` — LWJGL 2 (`org.lwjgl.lwjgl`) and LWJGL 3 (`org.lwjgl`).
/// * `net.java.jinput` / `net.java.jutils` — LWJGL 2 input stack (desktop only).
/// * `ca.weblite` — macOS Objective-C bridge, meaningless on Android.
pub const SUBSTITUTED_PREFIXES: &[&str] = &[
    "org.lwjgl:",
    "org.lwjgl.lwjgl:",
    "net.java.jinput:",
    "net.java.jutils:",
    "ca.weblite:",
];

/// How to assemble the classpath.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClasspathPolicy {
    /// Replace the manifest's LWJGL with the prebuilt Android bundle.
    pub substitute_lwjgl: bool,
    /// Append the client jar after the libraries (vanilla ordering). Mod loaders
    /// that patch classes need the loader jars first, which this ordering gives.
    pub client_jar_last: bool,
    /// Collapse duplicate `group:artifact` entries, keeping the highest version.
    pub collapse_duplicates: bool,
    /// Extra coordinate prefixes to drop (user / renderer plugin overrides).
    pub extra_excludes: Vec<String>,
}

impl Default for ClasspathPolicy {
    fn default() -> Self {
        Self {
            substitute_lwjgl: true,
            client_jar_last: true,
            collapse_duplicates: true,
            extra_excludes: Vec::new(),
        }
    }
}

/// The assembled classpath plus the metadata the engine needs to validate it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Classpath {
    /// Classpath entries in load order.
    pub entries: Vec<PathBuf>,
    /// Native jars that must be unpacked into `${natives_directory}`.
    pub native_jars: Vec<PathBuf>,
    /// Directories added to the native search path (`java.library.path`).
    pub native_dirs: Vec<PathBuf>,
    /// Coordinates dropped by LWJGL substitution (diagnostics).
    pub substituted: Vec<String>,
    /// Coordinates dropped because a higher version was present.
    pub collapsed: Vec<String>,
}

impl Classpath {
    /// Join with `separator` (`${classpath_separator}`, `:` on Android/Linux).
    pub fn join(&self, separator: &str) -> String {
        self.entries
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(separator)
    }

    /// Number of classpath entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is the classpath empty?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries that do not exist on disk (incomplete download / deleted file).
    pub fn missing(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .filter(|p| !p.is_file())
            .cloned()
            .collect()
    }

    /// Fail when any entry is missing, naming (at most) the first few.
    pub fn verify_present(&self) -> RcResult<()> {
        let missing = self.missing();
        if missing.is_empty() {
            return Ok(());
        }
        let shown: Vec<String> = missing
            .iter()
            .take(5)
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        Err(RcError::MissingFile(format!(
            "{} classpath entr{} missing (re-download the version): {}{}",
            missing.len(),
            if missing.len() == 1 {
                "y is"
            } else {
                "ies are"
            },
            shown.join(", "),
            if missing.len() > shown.len() {
                ", …"
            } else {
                ""
            }
        )))
    }
}

/// Builds a [`Classpath`] from a resolved version.
#[derive(Debug, Clone)]
pub struct ClasspathBuilder {
    /// `libraries/` root (task-4 download layout).
    pub libraries_dir: PathBuf,
    pub platform: Platform,
    pub features: Features,
    pub policy: ClasspathPolicy,
    /// Prebuilt Android runtime bundle (LWJGL / caciocavallo).
    pub app_runtime: Option<AppRuntime>,
    pub lwjgl_version: LwjglVersion,
    pub abi: Abi,
    pub java_version: JavaVersion,
    /// Put the caciocavallo AWT bridge jars on the classpath (task 18).
    pub use_cacio: bool,
}

impl ClasspathBuilder {
    /// A builder with the default policy.
    pub fn new(
        libraries_dir: impl Into<PathBuf>,
        platform: Platform,
        features: Features,
        abi: Abi,
        java_version: JavaVersion,
    ) -> Self {
        Self {
            libraries_dir: libraries_dir.into(),
            platform,
            features,
            policy: ClasspathPolicy::default(),
            app_runtime: None,
            lwjgl_version: LwjglVersion::default(),
            abi,
            java_version,
            use_cacio: true,
        }
    }

    /// Attach the prebuilt `app_runtime/` bundle.
    pub fn with_app_runtime(mut self, rt: AppRuntime, lwjgl: LwjglVersion) -> Self {
        self.app_runtime = Some(rt);
        self.lwjgl_version = lwjgl;
        self
    }

    /// Should `lib` be replaced by the prebuilt Android bundle?
    fn is_substituted(&self, lib: &Library) -> bool {
        let matches_builtin = SUBSTITUTED_PREFIXES.iter().any(|p| lib.name.starts_with(p));
        let matches_extra = self
            .policy
            .extra_excludes
            .iter()
            .any(|p| lib.name.starts_with(p.as_str()));
        (self.policy.substitute_lwjgl && matches_builtin) || matches_extra
    }

    /// Assemble the classpath for `resolved` with `client_jar`.
    pub fn build(&self, resolved: &ResolvedVersion, client_jar: &Path) -> RcResult<Classpath> {
        let mut cp = Classpath::default();
        // group:artifact -> (index into cp.entries, version)
        let mut seen: HashMap<String, (usize, String)> = HashMap::new();

        for lib in &resolved.libraries {
            if !lib.is_allowed(&self.platform, &self.features) {
                continue;
            }
            if self.is_substituted(lib) {
                cp.substituted.push(lib.name.clone());
                continue;
            }
            // Native jars are never classpath entries; they are unpacked into
            // ${natives_directory} instead.
            if let Some(classifier) = lib.native_classifier(&self.platform) {
                let path = self.libraries_dir.join(lib.maven_path(Some(&classifier)));
                cp.native_jars.push(path);
                // A library that declares `natives` for this platform is a
                // *natives-only* artifact unless it explicitly declares a main
                // jar in `downloads.artifact` (some Forge profiles do both).
                // `Library::artifact_url` synthesises a Maven URL for every
                // classifier-less coordinate, so it must NOT be used to decide
                // this: the synthesised main jar does not exist for LWJGL-2
                // style `*-platform` libraries and putting it on the classpath
                // would make `verify_present()` fail on a healthy install.
                if lib.downloads.artifact.is_none() {
                    continue;
                }
            }
            // A classifier-only coordinate has no main jar at all.
            let (group, artifact, version, classifier, _ext) = lib.parse_maven();
            if classifier.is_some() && lib.downloads.artifact.is_none() {
                continue;
            }
            let path = self.libraries_dir.join(lib.maven_path(None));
            let ga = format!("{}:{}", group, artifact);
            if self.policy.collapse_duplicates {
                if let Some((idx, prev_version)) = seen.get(&ga).cloned() {
                    if compare_maven_versions(&version, &prev_version)
                        == std::cmp::Ordering::Greater
                    {
                        cp.collapsed.push(format!(
                            "{}:{} (superseded by {})",
                            ga, prev_version, version
                        ));
                        cp.entries[idx] = path;
                        seen.insert(ga, (idx, version));
                    } else {
                        cp.collapsed.push(format!(
                            "{}:{} (superseded by {})",
                            ga, version, prev_version
                        ));
                    }
                    continue;
                }
                seen.insert(ga, (cp.entries.len(), version));
            }
            cp.entries.push(path);
        }

        // Prebuilt LWJGL bundle replaces the manifest's desktop jars.
        if let Some(rt) = &self.app_runtime {
            if self.policy.substitute_lwjgl {
                for jar in rt.lwjgl_jars(self.lwjgl_version)? {
                    cp.entries.push(jar);
                }
                let natives = rt.lwjgl_natives_dir(self.lwjgl_version, self.abi);
                if natives.is_dir() {
                    cp.native_dirs.push(natives);
                }
            }
            // caciocavallo AWT bridge (task 18): needed by any version that
            // touches AWT/Swing (Forge installers, the Mojang splash, …).
            // Only the *classpath*-role jars go here: the java agent is passed
            // with `-javaagent:` and `ResConfHack.jar` has to be prepended to the
            // boot classpath, both handled by `launch::awt`.
            if self.use_cacio {
                let backend = AwtBackend::for_java(self.java_version);
                for jar in CacioBundle::scan(rt, backend).classpath_jars() {
                    cp.entries.push(jar);
                }
            }
        }

        // Client jar.
        if self.policy.client_jar_last {
            cp.entries.push(client_jar.to_path_buf());
        } else {
            cp.entries.insert(0, client_jar.to_path_buf());
        }

        // Final safety net: identical paths must never appear twice.
        let mut deduped: Vec<PathBuf> = Vec::with_capacity(cp.entries.len());
        for e in cp.entries.into_iter() {
            if !deduped.contains(&e) {
                deduped.push(e);
            }
        }
        cp.entries = deduped;
        cp.native_jars.sort();
        cp.native_jars.dedup();
        Ok(cp)
    }
}

/// Compare two Maven versions (numeric-aware, `1.10 > 1.9`).
///
/// Segments are split on `.`, `-`, `_` and `+`; numeric segments compare
/// numerically, others lexicographically. A longer version wins when it is a
/// prefix of the other (`1.2.1 > 1.2`), except when the extra segment is a
/// pre-release marker, which sorts *below* the release (`1.2 > 1.2-beta`).
pub fn compare_maven_versions(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let split = |s: &str| -> Vec<String> {
        s.split(['.', '-', '_', '+'])
            .filter(|p| !p.is_empty())
            .map(|p| p.to_string())
            .collect()
    };
    let sa = split(a);
    let sb = split(b);
    let is_prerelease = |s: &str| {
        let l = s.to_ascii_lowercase();
        ["alpha", "beta", "rc", "snapshot", "pre", "m"]
            .iter()
            .any(|m| l == *m || l.starts_with(m))
            && s.parse::<u64>().is_err()
    };
    for i in 0..sa.len().max(sb.len()) {
        match (sa.get(i), sb.get(i)) {
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    // a number outranks a qualifier (1.0 > 1.0-beta)
                    (Ok(_), Err(_)) => Ordering::Greater,
                    (Err(_), Ok(_)) => Ordering::Less,
                    (Err(_), Err(_)) => x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            // extra segment: a pre-release marker sorts below the bare release
            (Some(x), None) => {
                return if is_prerelease(x) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
            (None, Some(y)) => {
                return if is_prerelease(y) {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            (None, None) => break,
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::platform::{Arch, OsName};

    fn android() -> Platform {
        Platform {
            os: OsName::Linux,
            arch: Arch::Arm64,
            os_version: String::new(),
        }
    }

    fn resolved(json: &str) -> ResolvedVersion {
        let v = crate::game::VersionJson::parse(json).unwrap();
        crate::game::version::merge_chain(&[v])
    }

    fn builder() -> ClasspathBuilder {
        ClasspathBuilder::new(
            "/mc/libraries",
            android(),
            Features::new(),
            Abi::Arm64V8a,
            JavaVersion::Java17,
        )
    }

    #[test]
    fn maven_version_comparison() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_maven_versions("1.10", "1.9"), Greater);
        assert_eq!(compare_maven_versions("1.2.1", "1.2"), Greater);
        assert_eq!(compare_maven_versions("2.0", "10.0"), Less);
        assert_eq!(compare_maven_versions("1.0", "1.0"), Equal);
        // pre-release sorts below the release
        assert_eq!(compare_maven_versions("1.0", "1.0-beta"), Greater);
        assert_eq!(compare_maven_versions("1.0-rc1", "1.0"), Less);
        // real-world Forge/ASM style versions
        assert_eq!(compare_maven_versions("9.6.1", "9.5"), Greater);
        assert_eq!(compare_maven_versions("3.3.3", "3.4.1"), Less);
    }

    #[test]
    fn lwjgl_is_substituted_by_the_prebuilt_bundle() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("app_runtime");
        let lwjgl = root.join("lwjgl").join("3.3.3");
        std::fs::create_dir_all(lwjgl.join("natives").join("arm64-v8a")).unwrap();
        std::fs::write(lwjgl.join("lwjgl.jar"), b"x").unwrap();
        std::fs::write(lwjgl.join("lwjgl-glfw.jar"), b"x").unwrap();

        let b = builder().with_app_runtime(AppRuntime::new(&root), LwjglVersion::V3_3_3);
        let v = resolved(
            r#"{"id":"1.20.4","libraries":[
                 {"name":"org.lwjgl:lwjgl:3.3.1"},
                 {"name":"org.lwjgl:lwjgl:3.3.1:natives-linux"},
                 {"name":"net.java.jinput:jinput:2.0.5"},
                 {"name":"com.mojang:patchy:1.1"}
               ]}"#,
        );
        let cp = b
            .build(&v, Path::new("/mc/versions/1.20.4/1.20.4.jar"))
            .unwrap();
        let joined = cp.join(":");
        // desktop LWJGL / jinput dropped …
        assert!(!joined.contains("org/lwjgl"), "{joined}");
        assert!(!joined.contains("jinput"), "{joined}");
        assert_eq!(cp.substituted.len(), 3);
        // … and the prebuilt bundle added, plus the normal library + client jar
        assert!(joined.contains("app_runtime/lwjgl/3.3.3/lwjgl.jar"));
        assert!(joined.contains("com/mojang/patchy/1.1/patchy-1.1.jar"));
        assert!(joined.ends_with("1.20.4.jar"));
        assert_eq!(
            cp.native_dirs,
            vec![lwjgl.join("natives").join("arm64-v8a")]
        );
    }

    #[test]
    fn substitution_can_be_disabled() {
        let mut b = builder();
        b.policy.substitute_lwjgl = false;
        let v = resolved(r#"{"id":"x","libraries":[{"name":"org.lwjgl:lwjgl:3.3.1"}]}"#);
        let cp = b.build(&v, Path::new("/mc/x.jar")).unwrap();
        assert!(cp
            .join(":")
            .contains("org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar"));
        assert!(cp.substituted.is_empty());
    }

    #[test]
    fn native_jars_are_collected_not_classpathed() {
        let b = builder();
        let v = resolved(
            r#"{"id":"x","libraries":[
                 {"name":"com.example:native:1.0",
                  "natives":{"linux":"natives-linux"},
                  "downloads":{"classifiers":{"natives-linux":{"path":"com/example/native/1.0/native-1.0-natives-linux.jar"}}}}
               ]}"#,
        );
        let cp = b.build(&v, Path::new("/mc/x.jar")).unwrap();
        assert_eq!(
            cp.native_jars,
            vec![PathBuf::from(
                "/mc/libraries/com/example/native/1.0/native-1.0-natives-linux.jar"
            )]
        );
        // natives-only library contributes no classpath entry (only client jar)
        assert_eq!(cp.entries, vec![PathBuf::from("/mc/x.jar")]);
    }

    #[test]
    fn duplicates_collapse_to_the_highest_version() {
        let b = builder();
        // Forge-style: two ASM versions declared, the newer one must survive.
        let v = resolved(
            r#"{"id":"x","libraries":[
                 {"name":"org.ow2.asm:asm:9.3"},
                 {"name":"org.ow2.asm:asm:9.6"},
                 {"name":"com.mojang:patchy:1.1"}
               ]}"#,
        );
        let cp = b.build(&v, Path::new("/mc/x.jar")).unwrap();
        let joined = cp.join(":");
        assert!(joined.contains("asm/9.6/asm-9.6.jar"), "{joined}");
        assert!(!joined.contains("asm-9.3.jar"), "{joined}");
        assert_eq!(cp.collapsed.len(), 1);
        // order is preserved: asm keeps the *first* slot it occupied
        assert!(cp.entries[0].to_string_lossy().contains("asm-9.6.jar"));

        // reversed declaration order gives the same winner
        let v2 = resolved(
            r#"{"id":"x","libraries":[
                 {"name":"org.ow2.asm:asm:9.6"},
                 {"name":"org.ow2.asm:asm:9.3"}]}"#,
        );
        let cp2 = b.build(&v2, Path::new("/mc/x.jar")).unwrap();
        assert!(cp2.join(":").contains("asm-9.6.jar"));
    }

    #[test]
    fn rules_filter_the_classpath() {
        let b = builder();
        let v = resolved(
            r#"{"id":"x","libraries":[
                 {"name":"only.windows:lib:1.0","rules":[{"action":"allow","os":{"name":"windows"}}]},
                 {"name":"any:lib:1.0"}]}"#,
        );
        let cp = b.build(&v, Path::new("/mc/x.jar")).unwrap();
        let joined = cp.join(":");
        assert!(!joined.contains("only/windows"), "{joined}");
        assert!(joined.contains("any/lib/1.0/lib-1.0.jar"));
    }

    #[test]
    fn client_jar_ordering_is_configurable() {
        let mut b = builder();
        b.policy.client_jar_last = false;
        let v = resolved(r#"{"id":"x","libraries":[{"name":"a:b:1"}]}"#);
        let cp = b.build(&v, Path::new("/mc/x.jar")).unwrap();
        assert_eq!(cp.entries[0], PathBuf::from("/mc/x.jar"));
    }

    #[test]
    fn missing_entries_are_reported_precisely() {
        let td = tempfile::tempdir().unwrap();
        let present = td.path().join("present.jar");
        std::fs::write(&present, b"x").unwrap();
        let cp = Classpath {
            entries: vec![present.clone(), td.path().join("gone.jar")],
            ..Default::default()
        };
        assert_eq!(cp.missing(), vec![td.path().join("gone.jar")]);
        let err = cp.verify_present().unwrap_err();
        assert!(err.to_string().contains("gone.jar"), "{err}");
        let ok = Classpath {
            entries: vec![present],
            ..Default::default()
        };
        assert!(ok.verify_present().is_ok());
    }

    #[test]
    fn cacio_jars_join_the_classpath_for_java17() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("app_runtime");
        std::fs::create_dir_all(root.join("caciocavallo17")).unwrap();
        std::fs::write(root.join("caciocavallo17").join("cacio-shared.jar"), b"x").unwrap();
        std::fs::write(root.join("caciocavallo17").join("cacio-agent.jar"), b"x").unwrap();
        std::fs::create_dir_all(root.join("lwjgl").join("3.3.3")).unwrap();
        std::fs::write(root.join("lwjgl").join("3.3.3").join("lwjgl.jar"), b"x").unwrap();

        let b = builder().with_app_runtime(AppRuntime::new(&root), LwjglVersion::V3_3_3);
        let v = resolved(r#"{"id":"x","libraries":[]}"#);
        let cp = b.build(&v, Path::new("/mc/x.jar")).unwrap();
        let joined = cp.join(":");
        assert!(joined.contains("cacio-shared.jar"), "{joined}");
        // the agent is passed with -javaagent, never on the classpath
        assert!(!joined.contains("cacio-agent.jar"), "{joined}");
    }

    #[test]
    fn identical_paths_are_deduped() {
        let b = builder();
        let v = resolved(r#"{"id":"x","libraries":[{"name":"a:b:1"},{"name":"a:b:1"}]}"#);
        let cp = b.build(&v, Path::new("/mc/x.jar")).unwrap();
        assert_eq!(cp.entries.len(), 2); // a:b:1 + client jar
    }
}
