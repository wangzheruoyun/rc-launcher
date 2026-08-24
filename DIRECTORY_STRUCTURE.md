# com.rc.launcher — 目录结构 (Directory Structure)

> 自动生成的目录树（已排除 `rust/target`、`.git`、`.gradle`、各模块 `build`、`jniLibs` 等生成/大目录）。

```
com.rc.launcher/
├── .github
│   └── workflows
│       ├── build.yml
│       ├── health.yml
│       ├── release.yml
│       └── stylecheck.yml
├── app
│   ├── src
│   │   ├── androidTest
│   │   │   └── java
│   │   │       └── com
│   │   │           └── rc
│   │   │               └── launcher
│   │   │                   ├── ui
│   │   │                   │   └── ComposeNavigationTest.kt
│   │   │                   └── ExampleInstrumentedTest.kt
│   │   ├── main
│   │   │   ├── java
│   │   │   │   └── com
│   │   │   │       └── rc
│   │   │   │           └── launcher
│   │   │   │               ├── ui
│   │   │   │               │   ├── awt
│   │   │   │               │   │   ├── AwtAndroidKeys.kt
│   │   │   │               │   │   ├── AwtCanvasBridge.kt
│   │   │   │               │   │   ├── AwtGeometry.kt
│   │   │   │               │   │   ├── AwtInput.kt
│   │   │   │               │   │   ├── AwtSessionInfo.kt
│   │   │   │               │   │   └── AwtWire.kt
│   │   │   │               │   ├── component
│   │   │   │               │   │   ├── AwtCanvasSurface.kt
│   │   │   │               │   │   ├── FloatingHud.kt
│   │   │   │               │   │   ├── InstanceCard.kt
│   │   │   │               │   │   └── ResourceSummary.kt
│   │   │   │               │   ├── i18n
│   │   │   │               │   │   ├── AppLanguage.kt
│   │   │   │               │   │   ├── LocaleEngine.kt
│   │   │   │               │   │   ├── LocaleStorage.kt
│   │   │   │               │   │   ├── RcLocaleContext.kt
│   │   │   │               │   │   ├── RcLocalization.kt
│   │   │   │               │   │   ├── RcStringKeys.kt
│   │   │   │               │   │   ├── RcStringResources.kt
│   │   │   │               │   │   ├── RcStrings.kt
│   │   │   │               │   │   └── RcStringsLoader.kt
│   │   │   │               │   ├── model
│   │   │   │               │   │   ├── json
│   │   │   │               │   │   │   └── MiniJson.kt
│   │   │   │               │   │   ├── Account.kt
│   │   │   │               │   │   ├── AccountRepository.kt
│   │   │   │               │   │   ├── ControlLayout.kt
│   │   │   │               │   │   ├── ControlLayoutRepository.kt
│   │   │   │               │   │   ├── GameInstance.kt
│   │   │   │               │   │   ├── InstallProfile.kt
│   │   │   │               │   │   ├── InstanceRepository.kt
│   │   │   │               │   │   ├── LauncherSettings.kt
│   │   │   │               │   │   └── SettingsRepository.kt
│   │   │   │               │   ├── navigation
│   │   │   │               │   │   └── RcNavigation.kt
│   │   │   │               │   ├── resource
│   │   │   │               │   │   ├── FpsTracker.kt
│   │   │   │               │   │   └── ResourceMonitor.kt
│   │   │   │               │   ├── screen
│   │   │   │               │   │   ├── AccountsScreen.kt
│   │   │   │               │   │   ├── AwtScreen.kt
│   │   │   │               │   │   ├── ControllerScreen.kt
│   │   │   │               │   │   ├── DownloadsScreen.kt
│   │   │   │               │   │   ├── HomeScreen.kt
│   │   │   │               │   │   ├── InstallWizardScreen.kt
│   │   │   │               │   │   ├── InstanceDetailScreen.kt
│   │   │   │               │   │   ├── InstancesScreen.kt
│   │   │   │               │   │   └── SettingsScreen.kt
│   │   │   │               │   ├── theme
│   │   │   │               │   │   ├── Theme.kt
│   │   │   │               │   │   ├── ThemeData.kt
│   │   │   │               │   │   ├── ThemeEngine.kt
│   │   │   │               │   │   ├── ThemeStorage.kt
│   │   │   │               │   │   └── ThemeViewModel.kt
│   │   │   │               │   ├── viewmodel
│   │   │   │               │   │   ├── AccountViewModel.kt
│   │   │   │               │   │   ├── AwtSurfaceViewModel.kt
│   │   │   │               │   │   ├── ControlLayoutViewModel.kt
│   │   │   │               │   │   ├── DashboardViewModel.kt
│   │   │   │               │   │   ├── InstallViewModel.kt
│   │   │   │               │   │   ├── InstanceDetailViewModel.kt
│   │   │   │               │   │   ├── LocaleViewModel.kt
│   │   │   │               │   │   ├── MainViewModel.kt
│   │   │   │               │   │   └── SettingsViewModel.kt
│   │   │   │               │   ├── MainScreen.kt
│   │   │   │               │   └── RcApp.kt
│   │   │   │               ├── MainActivity.kt
│   │   │   │               └── RcApplication.kt
│   │   │   ├── res
│   │   │   │   ├── values
│   │   │   │   │   ├── strings.xml
│   │   │   │   │   └── themes.xml
│   │   │   │   ├── values-en
│   │   │   │   │   └── strings.xml
│   │   │   │   ├── values-zh-rTW
│   │   │   │   │   └── strings.xml
│   │   │   │   └── xml
│   │   │   │       └── locales_config.xml
│   │   │   └── AndroidManifest.xml
│   │   └── test
│   │       └── java
│   │           └── com
│   │               └── rc
│   │                   └── launcher
│   │                       └── ui
│   │                           ├── awt
│   │                           │   ├── AwtAndroidKeysTest.kt
│   │                           │   ├── AwtGeometryTest.kt
│   │                           │   ├── AwtInputTest.kt
│   │                           │   ├── AwtSessionInfoTest.kt
│   │                           │   └── AwtWireTest.kt
│   │                           ├── component
│   │                           │   └── InstanceCardTest.kt
│   │                           ├── i18n
│   │                           │   ├── AppLanguageTest.kt
│   │                           │   ├── CatalogueParityTest.kt
│   │                           │   ├── CoreStringsSourceTest.kt
│   │                           │   ├── LocaleEngineTest.kt
│   │                           │   └── RcStringsTest.kt
│   │                           ├── model
│   │                           │   ├── AccountModelTest.kt
│   │                           │   ├── ControlLayoutRepositoryTest.kt
│   │                           │   ├── ControlLayoutTest.kt
│   │                           │   ├── GameInstanceTest.kt
│   │                           │   ├── InstallProfileTest.kt
│   │                           │   ├── LauncherSettingsTest.kt
│   │                           │   └── MiniJsonTest.kt
│   │                           ├── resource
│   │                           │   └── ResourceMathTest.kt
│   │                           ├── theme
│   │                           │   └── ThemeLogicTest.kt
│   │                           └── viewmodel
│   │                               ├── AccountViewModelTest.kt
│   │                               ├── AwtSurfaceViewModelTest.kt
│   │                               ├── ControlLayoutViewModelTest.kt
│   │                               ├── InstallViewModelTest.kt
│   │                               ├── InstanceDetailViewModelTest.kt
│   │                               ├── LocaleViewModelTest.kt
│   │                               └── SettingsViewModelTest.kt
│   └── build.gradle.kts
├── config
│   └── detekt
│       └── detekt.yml
├── core
│   ├── src
│   │   └── main
│   │       ├── java
│   │       │   └── com
│   │       │       └── rc
│   │       │           └── launcher
│   │       │               └── core
│   │       │                   ├── RcEventBus.kt
│   │       │                   └── RustBridge.kt
│   │       └── AndroidManifest.xml
│   ├── build.gradle.kts
│   └── consumer-rules.pro
├── docs
│   ├── ARCHITECTURE.md
│   ├── BUILD.md
│   ├── CONTRIBUTING.md
│   ├── MODULES.md
│   ├── auth.md
│   ├── awt.md
│   ├── ffi_event_bus.md
│   ├── health_audit.md
│   ├── i18n.md
│   ├── launch.md
│   └── rendering.md
├── gradle
│   └── wrapper
│       └── gradle-wrapper.properties
├── runtime
│   ├── src
│   │   ├── main
│   │   │   ├── assets
│   │   │   │   └── app_runtime
│   │   │   │       └── java
│   │   │   │           ├── jre17
│   │   │   │           │   ├── bin-arm64.tar.xz
│   │   │   │           │   ├── universal.tar.xz
│   │   │   │           │   └── version
│   │   │   │           ├── jre21
│   │   │   │           │   ├── bin-arm64.tar.xz
│   │   │   │           │   ├── universal.tar.xz
│   │   │   │           │   └── version
│   │   │   │           ├── jre25
│   │   │   │           │   ├── bin-arm64.tar.xz
│   │   │   │           │   ├── universal.tar.xz
│   │   │   │           │   └── version
│   │   │   │           ├── jre8
│   │   │   │           │   ├── bin-arm64.tar.xz
│   │   │   │           │   ├── universal.tar.xz
│   │   │   │           │   └── version
│   │   │   │           └── jre_manifest.json
│   │   │   ├── java
│   │   │   │   └── com
│   │   │   │       └── rc
│   │   │   │           └── launcher
│   │   │   │               └── runtime
│   │   │   │                   └── JreManager.kt
│   │   │   └── AndroidManifest.xml
│   │   └── test
│   │       └── java
│   │           └── com
│   │               └── rc
│   │                   └── launcher
│   │                       └── runtime
│   │                           └── JreManagerTest.kt
│   ├── build.gradle.kts
│   └── generate_jre_manifest.py
├── rust
│   ├── crates
│   │   └── rc-launcher-core
│   │       ├── examples
│   │       │   ├── awt_demo.rs
│   │       │   ├── i18n_demo.rs
│   │       │   └── launch_demo.rs
│   │       ├── i18n
│   │       │   ├── en.properties
│   │       │   ├── zh-CN.properties
│   │       │   └── zh-Hant.properties
│   │       ├── src
│   │       │   ├── auth
│   │       │   │   ├── manager.rs
│   │       │   │   ├── microsoft.rs
│   │       │   │   ├── mod.rs
│   │       │   │   ├── model.rs
│   │       │   │   ├── offline.rs
│   │       │   │   ├── store.rs
│   │       │   │   ├── transport.rs
│   │       │   │   └── vault.rs
│   │       │   ├── download
│   │       │   │   ├── client.rs
│   │       │   │   ├── hash.rs
│   │       │   │   ├── manager.rs
│   │       │   │   ├── mod.rs
│   │       │   │   └── testing.rs
│   │       │   ├── game
│   │       │   │   ├── assets.rs
│   │       │   │   ├── library.rs
│   │       │   │   ├── manifest.rs
│   │       │   │   ├── mod.rs
│   │       │   │   ├── platform.rs
│   │       │   │   ├── resolve.rs
│   │       │   │   └── version.rs
│   │       │   ├── i18n
│   │       │   │   ├── catalog.rs
│   │       │   │   ├── format.rs
│   │       │   │   ├── language.rs
│   │       │   │   └── mod.rs
│   │       │   ├── launch
│   │       │   │   ├── args.rs
│   │       │   │   ├── awt.rs
│   │       │   │   ├── awt_host.rs
│   │       │   │   ├── classpath.rs
│   │       │   │   ├── command.rs
│   │       │   │   ├── crash.rs
│   │       │   │   ├── engine.rs
│   │       │   │   ├── env.rs
│   │       │   │   ├── fakefx.rs
│   │       │   │   ├── mod.rs
│   │       │   │   ├── options.rs
│   │       │   │   ├── process.rs
│   │       │   │   ├── render.rs
│   │       │   │   └── runtime_assets.rs
│   │       │   ├── mods
│   │       │   │   ├── conflict.rs
│   │       │   │   ├── constraint.rs
│   │       │   │   ├── loader.rs
│   │       │   │   ├── metadata.rs
│   │       │   │   ├── mod.rs
│   │       │   │   ├── resource_pack.rs
│   │       │   │   └── shader.rs
│   │       │   ├── net
│   │       │   │   ├── client.rs
│   │       │   │   ├── dns.rs
│   │       │   │   ├── mirror.rs
│   │       │   │   ├── mod.rs
│   │       │   │   └── proxy.rs
│   │       │   ├── plugins
│   │       │   │   ├── fcl_apk.rs
│   │       │   │   ├── mod.rs
│   │       │   │   ├── native_lib.rs
│   │       │   │   ├── renderer.rs
│   │       │   │   └── validation.rs
│   │       │   ├── robust
│   │       │   │   ├── cache.rs
│   │       │   │   ├── mod.rs
│   │       │   │   ├── reporter.rs
│   │       │   │   └── retry.rs
│   │       │   ├── runtime
│   │       │   │   ├── abi.rs
│   │       │   │   ├── extract.rs
│   │       │   │   ├── java_version.rs
│   │       │   │   ├── manager.rs
│   │       │   │   ├── manifest.rs
│   │       │   │   ├── mod.rs
│   │       │   │   └── source.rs
│   │       │   ├── util
│   │       │   │   ├── bufpool.rs
│   │       │   │   └── mod.rs
│   │       │   ├── capi.rs
│   │       │   ├── error.rs
│   │       │   ├── event.rs
│   │       │   ├── ffi.rs
│   │       │   ├── integration_tests.rs
│   │       │   ├── jobs.rs
│   │       │   └── lib.rs
│   │       ├── Cargo.toml
│   │       ├── cbindgen.toml
│   │       └── rc_launcher.h
│   ├── Cargo.lock
│   └── Cargo.toml
├── scripts
│   ├── check_i18n.py
│   ├── gen_android_strings.py
│   ├── health_audit.py
│   └── i18n_common.py
├── snapshots
│   ├── FCL-Team__Android-Easytier-Build.txt
│   ├── FCL-Team__Android-OpenJDK-Build.txt
│   ├── FCL-Team__EnchantNet.txt
│   ├── FCL-Team__EnchantNetCore.txt
│   ├── FCL-Team__FCL-Controllers.txt
│   ├── FCL-Team__FCL-Docs.txt
│   ├── FCL-Team__FCL-Repo.txt
│   ├── FCL-Team__FCL-Team.github.io.txt
│   ├── FCL-Team__FCLDriverPlugin.txt
│   ├── FCL-Team__FCLRendererPlugin.txt
│   ├── FCL-Team__FoldCraftLauncher.txt
│   ├── FCL-Team__Holy-GL4ES.txt
│   ├── FCL-Team__LWJGL-Pojav.txt
│   ├── FCL-Team__NG-GL4ES.txt
│   ├── FCL-Team__OpenAL.txt
│   ├── FCL-Team__angle-gles.txt
│   ├── FCL-Team__caciocavallo-FCL.txt
│   ├── FCL-Team__caciocavallo11-FCL.txt
│   ├── FCL-Team__caciocavallo17-FCL.txt
│   ├── FCL-Team__lwjgl-fcl.txt
│   ├── FCL-Team__lwjgl3-fcl.txt
│   ├── FCL-Team__lwjgl3.txt
│   ├── FCL-Team__mesa.txt
│   ├── FCL-Team__zstd-jni-DH.txt
│   ├── ZalithLauncher__LWJGL-AAMC.txt
│   ├── ZalithLauncher__NativeLibPlugin.txt
│   ├── ZalithLauncher__OptiFineRenamer.txt
│   ├── ZalithLauncher__RendererPlugin-v2.txt
│   ├── ZalithLauncher__RendererPlugin.txt
│   ├── ZalithLauncher__SDL.txt
│   ├── ZalithLauncher__VerifiedPluginLoad.txt
│   ├── ZalithLauncher__Zalith-Info.txt
│   ├── ZalithLauncher__ZalithJars.txt
│   ├── ZalithLauncher__ZalithLauncher.txt
│   ├── ZalithLauncher__ZalithLauncher2.txt
│   ├── ZalithLauncher__ZalithRendererPlugin.txt
│   ├── ZalithLauncher__ZalithWebsite.txt
│   ├── ZalithLauncher__lwjgl3.txt
│   ├── ZalithLauncher__zalithdocs.txt
│   ├── cuberite__cuberite.txt
│   └── pmh1314520__MCTier.txt
├── .editorconfig
├── .gitignore
├── ANALYSIS.md
├── CONTRIBUTING.md
├── DIRECTORY_STRUCTURE.md
├── FCL_APK_RUNTIME_ASSETS_CATALOG.md
├── FCL_NATIVE_LIBRARIES.md
├── README.md
├── SNAPSHOTS_INDEX.md
├── build.gradle.kts
├── clippy.toml
├── gradle.properties
├── gradlew
├── gradlew.bat
├── rust-toolchain.toml
├── rustfmt.toml
├── settings.gradle.kts
└── task_list.txt
```
