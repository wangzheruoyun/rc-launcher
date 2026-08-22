//! Process environment for the game JVM (task 7).
//!
//! Launching a desktop JVM inside an Android app needs a hand-built
//! environment: the bundled JRE is not on the system loader path, the renderer
//! (`libgl4es_114.so`, `libOSMesa_8.so`, …) lives in the app's
//! `nativeLibraryDir`, and the GL translation layers are configured purely
//! through environment variables. This mirrors what FCL's `FCLauncher` /
//! `jre_launcher.c` sets up before `JNI_CreateJavaVM`.
//!
//! The two path lists are built once and shared:
//!
//! * [`library_path`] — every directory holding a `.so` the JVM may `dlopen`
//!   (JRE natives, extracted `${natives_directory}`, prebuilt LWJGL natives, the
//!   app's `nativeLibraryDir`, plus the system dirs for `libEGL`/`libGLESv2`).
//!   It feeds both `LD_LIBRARY_PATH` and `-Djava.library.path`.
//! * [`jre_lib_dirs`] — the JRE's own native layout, which differs between
//!   Java 8 (`lib/<arch>/{,server,jli}`) and Java 17+ (`lib/{,server}`); the
//!   split is taken from the real FCL packages (see
//!   `FCL_APK_RUNTIME_ASSETS_CATALOG.md`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::launch::options::LaunchOptions;
use crate::runtime::{Abi, JavaVersion};

/// Path list separator (`:` on Android/Linux — the only platform we launch on).
pub const PATH_SEP: &str = ":";

/// System directories that hold `libEGL.so` / `libGLESv2.so` / `libvulkan.so`.
///
/// 64-bit ABIs load from `lib64`, 32-bit ones from `lib`.
fn system_lib_dirs(abi: Abi) -> Vec<PathBuf> {
    let suffix = match abi {
        Abi::Arm64V8a | Abi::X86_64 => "lib64",
        Abi::ArmeabiV7a | Abi::X86 => "lib",
    };
    vec![
        PathBuf::from("/system").join(suffix),
        PathBuf::from("/vendor").join(suffix),
    ]
}

/// The JRE's own native library directories.
///
/// * Java 8:  `lib/<arch>/`, `lib/<arch>/server/`, `lib/<arch>/jli/`
/// * Java 17+: `lib/`, `lib/server/`
pub fn jre_lib_dirs(java_home: &Path, java: JavaVersion, abi: Abi) -> Vec<PathBuf> {
    let lib = java_home.join("lib");
    if java == JavaVersion::Java8 {
        let arch = java8_arch_dir(abi);
        let base = lib.join(arch);
        vec![base.join("server"), base.join("jli"), base, lib]
    } else {
        vec![lib.join("server"), lib]
    }
}

/// Java 8's `lib/<arch>` directory name for an Android ABI.
pub fn java8_arch_dir(abi: Abi) -> &'static str {
    match abi {
        Abi::Arm64V8a => "aarch64",
        Abi::ArmeabiV7a => "arm",
        Abi::X86 => "i386",
        Abi::X86_64 => "amd64",
    }
}

/// Every directory that must be searchable for native libraries, in load order.
///
/// `extra_dirs` are the renderer / LWJGL directories discovered by the classpath
/// builder ([`crate::launch::Classpath::native_dirs`]). Duplicates are collapsed
/// while preserving order, so a caller can pass overlapping lists safely.
pub fn library_path(
    options: &LaunchOptions,
    version_id: &str,
    extra_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let push = |d: PathBuf, dirs: &mut Vec<PathBuf>| {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    };

    // 1) extracted natives for this version (`${natives_directory}`)
    push(options.natives_dir_for(version_id), &mut dirs);
    // 2) prebuilt LWJGL natives + anything else the classpath builder found
    for d in extra_dirs {
        push(d.clone(), &mut dirs);
    }
    // 3) the app's own nativeLibraryDir (renderer, OpenAL, …)
    if let Some(nl) = &options.native_lib_dir {
        push(nl.clone(), &mut dirs);
    }
    // 4) the JRE's natives
    for d in jre_lib_dirs(&options.java_home, options.java_version, options.abi) {
        push(d, &mut dirs);
    }
    // 5) system GLES / Vulkan drivers
    for d in system_lib_dirs(options.abi) {
        push(d, &mut dirs);
    }
    dirs
}

/// Join a path list with [`PATH_SEP`].
pub fn join_paths(dirs: &[PathBuf]) -> String {
    dirs.iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(PATH_SEP)
}

/// The environment handed to the game process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchEnv {
    vars: BTreeMap<String, String>,
}

impl LaunchEnv {
    /// An empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set `key` = `value`.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    /// Value of `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(|s| s.as_str())
    }

    /// Number of variables.
    pub fn len(&self) -> usize {
        self.vars.len()
    }

    /// Is the environment empty?
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Iterate over `(key, value)` pairs (sorted, deterministic).
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.vars.iter()
    }

    /// The underlying map.
    pub fn as_map(&self) -> &BTreeMap<String, String> {
        &self.vars
    }

    /// Consume into the underlying map.
    pub fn into_map(self) -> BTreeMap<String, String> {
        self.vars
    }
}

/// Build the environment for a launch.
///
/// `native_dirs` come from the assembled classpath (prebuilt LWJGL natives …).
/// User [`LaunchOptions::env_overrides`] are applied **last** so they always win.
pub fn build_env(options: &LaunchOptions, version_id: &str, native_dirs: &[PathBuf]) -> LaunchEnv {
    let mut env = LaunchEnv::new();
    let lib_dirs = library_path(options, version_id, native_dirs);

    env.set("JAVA_HOME", options.java_home.to_string_lossy().to_string());
    env.set(
        "PATH",
        format!(
            "{}{}{}",
            options.java_home.join("bin").to_string_lossy(),
            PATH_SEP,
            ["/system/bin", "/system/xbin", "/vendor/bin"].join(PATH_SEP)
        ),
    );
    env.set("LD_LIBRARY_PATH", join_paths(&lib_dirs));
    // The game writes options.txt / saves / logs relative to $HOME on some
    // versions, so anchor it inside the instance directory.
    env.set("HOME", options.game_dir.to_string_lossy().to_string());
    env.set(
        "TMPDIR",
        options.data_root.join("tmp").to_string_lossy().to_string(),
    );
    // Shader / GLSL caches must live in app-writable storage.
    env.set(
        "MESA_GLSL_CACHE_DIR",
        options
            .data_root
            .join("cache")
            .to_string_lossy()
            .to_string(),
    );

    // Renderer selection: `POJAV_RENDERER` is the de-facto name understood by
    // the GL4ES / Mesa builds FCL and Zalith ship; `RC_RENDERER` is our own
    // (task 9 renderer plugins read it).
    env.set("POJAV_RENDERER", options.renderer.id());
    env.set("RC_RENDERER", options.renderer.id());
    for (k, v) in options.renderer.env() {
        env.set(k, v);
    }
    // Mesa/Gallium drivers (zink, virgl) are dlopen'd from nativeLibraryDir;
    // `POJAV_NATIVEDIR` is the name the GL4ES / Mesa builds themselves read
    // (verified against the FCL APK's own launcher strings).
    if let Some(nl) = &options.native_lib_dir {
        env.set("LIBGL_DRIVERS_PATH", nl.to_string_lossy().to_string());
        env.set("POJAV_NATIVEDIR", nl.to_string_lossy().to_string());
    }

    // task 17: GL4ES/ANGLE OpenGL->OpenGL ES translation backend selection,
    // plus the performance-tuning profile. These *complement* (never override)
    // the base renderer environment set above; the user can still override them
    // via `env_overrides` immediately below.
    for (k, v) in crate::launch::render::gl_translation_env(
        options.renderer,
        options.native_lib_dir.as_deref(),
    ) {
        env.set(k, v);
    }
    for (k, v) in options.perf_profile.env() {
        env.set(k, v);
    }

    // User overrides win (settings UI / power users).
    for (k, v) in &options.env_overrides {
        env.set(k.clone(), v.clone());
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::options::{AccountProfile, Renderer};

    fn opts() -> LaunchOptions {
        let mut o = LaunchOptions::new(
            "/data/mc/.minecraft",
            "/data/mc",
            "/data/jre17",
            JavaVersion::Java17,
            AccountProfile::offline("Steve", "uuid"),
        );
        o.native_lib_dir = Some(PathBuf::from("/data/app/lib/arm64"));
        o
    }

    #[test]
    fn jre_lib_dirs_follow_the_java_version_layout() {
        // Java 17+: lib/ + lib/server
        let d = jre_lib_dirs(Path::new("/jre17"), JavaVersion::Java17, Abi::Arm64V8a);
        assert_eq!(
            d,
            vec![
                PathBuf::from("/jre17/lib/server"),
                PathBuf::from("/jre17/lib")
            ]
        );
        // Java 8: lib/<arch>/{server,jli} + lib/<arch> + lib
        let d8 = jre_lib_dirs(Path::new("/jre8"), JavaVersion::Java8, Abi::Arm64V8a);
        assert_eq!(d8[0], PathBuf::from("/jre8/lib/aarch64/server"));
        assert_eq!(d8[1], PathBuf::from("/jre8/lib/aarch64/jli"));
        assert_eq!(d8[2], PathBuf::from("/jre8/lib/aarch64"));
        assert_eq!(d8[3], PathBuf::from("/jre8/lib"));
    }

    #[test]
    fn java8_arch_dirs_cover_every_abi() {
        assert_eq!(java8_arch_dir(Abi::Arm64V8a), "aarch64");
        assert_eq!(java8_arch_dir(Abi::ArmeabiV7a), "arm");
        assert_eq!(java8_arch_dir(Abi::X86), "i386");
        assert_eq!(java8_arch_dir(Abi::X86_64), "amd64");
    }

    #[test]
    fn library_path_is_ordered_and_deduplicated() {
        let o = opts();
        let extra = vec![
            PathBuf::from("/data/app_runtime/lwjgl/3.3.3/natives/arm64-v8a"),
            // duplicate of the app lib dir: must be collapsed
            PathBuf::from("/data/app/lib/arm64"),
        ];
        let p = library_path(&o, "1.20.4", &extra);
        assert_eq!(
            p[0],
            PathBuf::from("/data/mc/versions/1.20.4/natives-arm64-v8a")
        );
        assert_eq!(p[1], extra[0]);
        assert_eq!(p[2], PathBuf::from("/data/app/lib/arm64"));
        assert!(p.contains(&PathBuf::from("/data/jre17/lib/server")));
        assert!(p.contains(&PathBuf::from("/system/lib64")));
        // no duplicates
        let mut sorted = p.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), p.len());
    }

    #[test]
    fn system_dirs_follow_the_abi_word_size() {
        let mut o = opts();
        o.abi = Abi::ArmeabiV7a;
        let p = library_path(&o, "x", &[]);
        assert!(p.contains(&PathBuf::from("/system/lib")));
        assert!(!p.contains(&PathBuf::from("/system/lib64")));
    }

    #[test]
    fn env_has_the_pieces_the_jvm_needs() {
        let o = opts();
        let e = build_env(&o, "1.20.4", &[]);
        assert_eq!(e.get("JAVA_HOME"), Some("/data/jre17"));
        assert!(e.get("PATH").unwrap().starts_with("/data/jre17/bin:"));
        assert!(e
            .get("LD_LIBRARY_PATH")
            .unwrap()
            .contains("/data/jre17/lib/server"));
        assert_eq!(e.get("HOME"), Some("/data/mc/.minecraft"));
        assert_eq!(e.get("TMPDIR"), Some("/data/mc/tmp"));
        assert_eq!(e.get("POJAV_RENDERER"), Some("opengles2"));
        assert_eq!(e.get("RC_RENDERER"), Some("opengles2"));
        // GL4ES tuning is present
        assert_eq!(e.get("LIBGL_ES"), Some("2"));
        assert_eq!(e.get("LIBGL_DRIVERS_PATH"), Some("/data/app/lib/arm64"));
    }

    #[test]
    fn renderer_switch_changes_the_env() {
        let mut o = opts();
        o.renderer = Renderer::Zink;
        let e = build_env(&o, "x", &[]);
        assert_eq!(e.get("MESA_LOADER_DRIVER_OVERRIDE"), Some("zink"));
        assert_eq!(
            e.get("POJAV_RENDERER"),
            Some("opengles3_desktopgl_zink_kopper")
        );
        assert!(e.get("LIBGL_ES").is_none());
    }

    #[test]
    fn user_overrides_win() {
        let mut o = opts();
        o.env_overrides.insert("LIBGL_ES".into(), "3".into());
        o.env_overrides.insert("MY_FLAG".into(), "1".into());
        let e = build_env(&o, "x", &[]);
        assert_eq!(e.get("LIBGL_ES"), Some("3"));
        assert_eq!(e.get("MY_FLAG"), Some("1"));
    }

    #[test]
    fn env_iteration_is_deterministic() {
        let o = opts();
        let e = build_env(&o, "x", &[]);
        let keys: Vec<&String> = e.iter().map(|(k, _)| k).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert!(!e.is_empty() && e.len() == e.as_map().len());
    }

    #[test]
    fn joins_paths_with_the_platform_separator() {
        assert_eq!(
            join_paths(&[PathBuf::from("/a"), PathBuf::from("/b")]),
            "/a:/b"
        );
        assert_eq!(join_paths(&[]), "");
    }
}
