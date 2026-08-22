//! End-to-end demonstration of the task-7 launch engine.
//!
//! ```bash
//! cd rust && cargo run --example launch_demo
//! ```
//!
//! It builds a throw-away "device" tree (a fake `app_runtime/` with the real FCL
//! file names, a fake `bin/java` shell script standing in for the JVM, a fake
//! `libraries/` + client jar), then drives the engine three times:
//!
//! 1. a healthy launch — shows the assembled command line and a clean exit,
//! 2. a crashing launch — shows the crash classification + localised advice,
//! 3. a hung launch that is stopped — shows SIGTERM → SIGKILL escalation and
//!    that an intentional stop is *not* reported as a crash.
//!
//! No JVM, no Android device and no network access required.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rc_launcher::game::version::{merge_chain, VersionJson};
use rc_launcher::game::ResolvedVersion;
use rc_launcher::launch::{AccountProfile, LaunchEngine, LaunchOptions, Renderer};
use rc_launcher::runtime::{Abi, JavaVersion};

/// `version.json` of a vanilla-shaped 1.20.4 profile.
fn version() -> ResolvedVersion {
    let json = serde_json::json!({
        "id": "1.20.4",
        "type": "release",
        "mainClass": "net.minecraft.client.main.Main",
        "javaVersion": { "majorVersion": 17, "component": "java-runtime-gamma" },
        "assetIndex": { "id": "12", "url": "https://example/12.json" },
        "libraries": [
            { "name": "com.mojang:patchy:1.3.9" },
            { "name": "org.lwjgl:lwjgl:3.3.3" },
            { "name": "com.mojang:text2speech:1.11.3",
              "natives": { "linux": "natives-linux" } }
        ],
        "arguments": {
            "jvm": ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"],
            "game": [
                "--username", "${auth_player_name}",
                "--version", "${version_name}",
                "--gameDir", "${game_directory}",
                "--assetsDir", "${assets_root}",
                "--accessToken", "${auth_access_token}",
                "--clientId", "${clientid}",
                "--userType", "${user_type}"
            ]
        }
    });
    let parsed: VersionJson = serde_json::from_value(json).expect("version.json");
    merge_chain(&[parsed])
}

/// Create the fake on-device tree; `java_script` becomes `jre17/bin/java`.
fn install(root: &Path, java_script: &str) -> LaunchOptions {
    let app_runtime = root.join("app_runtime");

    let jre = app_runtime.join("java").join("jre17");
    fs::create_dir_all(jre.join("bin")).unwrap();
    fs::create_dir_all(jre.join("lib").join("server")).unwrap();
    let java = jre.join("bin").join("java");
    fs::write(&java, java_script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&java, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let lwjgl = app_runtime.join("lwjgl").join("3.3.3");
    fs::create_dir_all(lwjgl.join("natives").join("arm64-v8a")).unwrap();
    for jar in [
        "lwjgl.jar",
        "lwjgl-3.3.3-merged-modules.jar",
        "lwjgl-openal.jar",
    ] {
        fs::write(lwjgl.join(jar), b"jar").unwrap();
    }
    let cacio = app_runtime.join("caciocavallo17");
    fs::create_dir_all(&cacio).unwrap();
    for jar in ["cacio-shared-1.19.1-SNAPSHOT.jar", "cacio-agent.jar"] {
        fs::write(cacio.join(jar), b"jar").unwrap();
    }

    let lib = root.join("libraries/com/mojang/patchy/1.3.9");
    fs::create_dir_all(&lib).unwrap();
    fs::write(lib.join("patchy-1.3.9.jar"), b"jar").unwrap();
    let vdir = root.join("versions").join("1.20.4");
    fs::create_dir_all(&vdir).unwrap();
    fs::write(vdir.join("1.20.4.jar"), b"jar").unwrap();
    let game_dir = root.join(".minecraft");
    fs::create_dir_all(&game_dir).unwrap();

    let mut o = LaunchOptions::new(
        &game_dir,
        root,
        &jre,
        JavaVersion::Java17,
        AccountProfile::microsoft("Steve", "0-0-0-0", "a-very-secret-access-token"),
    );
    o.app_runtime = Some(app_runtime);
    o.abi = Abi::Arm64V8a;
    o.renderer = Renderer::Gl4es;
    o.native_lib_dir = Some(root.join("nativeLibraryDir"));
    o.memory.max_mb = 2048;
    o
}

fn banner(title: &str) {
    println!(
        "\n=== {title} {}",
        "=".repeat(60usize.saturating_sub(title.len()))
    );
}

#[tokio::main]
async fn main() {
    let td = tempfile::tempdir().expect("tempdir");
    let root: PathBuf = td.path().to_path_buf();
    let version = version();

    // ---------------------------------------------------------------- 1. OK ---
    banner("1. healthy launch");
    let fake_jvm = "#!/bin/sh\n\
        echo \"[JVM] argv: $*\"\n\
        echo \"[JVM] cwd: $(pwd)  LD_LIBRARY_PATH=$LD_LIBRARY_PATH\"\n\
        echo '[main/INFO]: Setting user: Steve'\n\
        echo '[main/INFO]: Stopping!'\n\
        exit 0\n";
    let options = install(&root, fake_jvm);
    let engine = LaunchEngine::new(options.clone());

    // Preflight only: this is what the UI calls to preview / validate a launch.
    let prepared = engine.prepare(&version).expect("prepare");
    println!(
        "[classpath] {} entries, {} substituted (desktop LWJGL -> Android bundle), {} warning(s)",
        prepared.classpath.len(),
        prepared.classpath.substituted.len(),
        prepared.warnings.len()
    );

    // `launch_and_wait` streams the (redacted) launch header first, then the
    // game output — exactly what the log window shows.
    let exit = engine
        .launch_and_wait(&version, |line| println!("  {line}"))
        .await
        .expect("launch");
    println!("[result] {}", exit.summary());
    assert!(exit.is_success());
    // the access token appears in argv but never in the captured log
    assert!(!exit.log.to_text().contains("a-very-secret-access-token"));
    println!("[result] token redacted in the log: OK");

    // ------------------------------------------------------------- 2. crash ---
    banner("2. crashing launch (out of memory)");
    let crashing = "#!/bin/sh\n\
        echo '[main/INFO]: Setting user: Steve'\n\
        echo 'Exception in thread \"main\" java.lang.OutOfMemoryError: Java heap space' 1>&2\n\
        echo '\tat net.minecraft.client.main.Main.main(Main.java:205)' 1>&2\n\
        exit 1\n";
    let root2 = root.join("crash");
    fs::create_dir_all(&root2).unwrap();
    let engine = LaunchEngine::new(install(&root2, crashing));
    let exit = engine
        .launch_and_wait(&version, |_| {})
        .await
        .expect("launch");
    println!("[result]   {}", exit.summary());
    println!(
        "[verdict]  {} ({})",
        exit.crash.category.summary(),
        exit.crash.category.id()
    );
    println!("[evidence] {}", exit.crash.evidence.join("\n           "));
    println!("[advice]   {}", exit.crash.category.advice());
    println!("[建议]     {}", exit.crash.category.advice_zh());
    assert_eq!(
        exit.crash.category,
        rc_launcher::launch::CrashCategory::OutOfMemory
    );

    // -------------------------------------------------------------- 3. stop ---
    banner("3. stopping a hung game (SIGTERM -> SIGKILL)");
    let hung = "#!/bin/sh\ntrap '' TERM\necho '[JVM] ignoring SIGTERM'\nsleep 60\n";
    let root3 = root.join("hung");
    fs::create_dir_all(&root3).unwrap();
    let engine = LaunchEngine::new(install(&root3, hung));
    let (_prepared, mut process) = engine.launch(&version).expect("launch");
    println!("[pid] {} running: {}", process.pid(), process.is_running());
    tokio::time::sleep(Duration::from_millis(200)).await;
    engine
        .stop(&mut process, Duration::from_millis(300))
        .await
        .expect("stop");
    let exit = process.wait().await.expect("wait");
    println!("[result]  {}", exit.summary());
    println!(
        "[verdict] user stop, not a crash: {}",
        exit.crash.terminated_by_user()
    );
    assert!(exit.crash.terminated_by_user());

    println!("\nAll launch-engine demonstrations passed.");
}
