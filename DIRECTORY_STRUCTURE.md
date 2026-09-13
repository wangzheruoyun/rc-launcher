# Directory Structure of com.rc.launcher

This document describes the project directory structure, excluding build artifacts,
version control directory (.git), and generated snapshots.

```
📁 com.rc.launcher/
    ├── 📄 .editorconfig
    ├── 📁 .github/
    │   └── 📁 workflows/
    │       ├── 📄 build.yml
    │       ├── 📄 health.yml
    │       ├── 📄 release.yml
    │       └── 📄 stylecheck.yml
    ├── 📄 .gitignore
    ├── 📄 ANALYSIS.md
    ├── 📄 CONTRIBUTING.md
    ├── 📄 DIRECTORY_STRUCTURE.md
    ├── 📄 FCL_APK_RUNTIME_ASSETS_CATALOG.md
    ├── 📄 FCL_NATIVE_LIBRARIES.md
    ├── 📄 PLAYER_FEEDBACK_SUMMARY.md
    ├── 📄 README.md
    ├── 📄 README_zh-CN.md
    ├── 📄 SNAPSHOTS_INDEX.md
    ├── 📁 androidx/
    │   └── 📁 compose/
    │       └── 📁 runtime/
    ├── 📁 app/
    │   ├── 📄 build.gradle.kts
    │   └── 📁 src/
    │       ├── 📁 main/
    │       │   ├── 📄 AndroidManifest.xml
    │       │   ├── 📁 assets/
    │       │   │   └── 📄 microsoft_auth.html
    │       │   ├── 📁 java/
    │       │   │   └── 📁 com/
    │       │   │       └── 📁 rc/
    │       │   │           └── 📁 launcher/
    │       │   │               ├── 📄 MainActivity.kt
    │       │   │               ├── 📄 RcApplication.kt
    │       │   │               ├── 📁 core/
    │       │   │               │   └── 📄 ControlLayoutDownloaderBridge.kt
    │       │   │               └── 📁 ui/
    │       │   │                   ├── 📄 AdaptiveLayout.kt
    │       │   │                   ├── 📄 MainScreen.kt
    │       │   │                   ├── 📄 RcApp.kt
    │       │   │                   ├── 📁 awt/
    │       │   │                   │   ├── 📄 AwtAndroidKeys.kt
    │       │   │                   │   ├── 📄 AwtCanvasBridge.kt
    │       │   │                   │   ├── 📄 AwtControl.kt
    │       │   │                   │   ├── 📄 AwtGeometry.kt
    │       │   │                   │   ├── 📄 AwtInput.kt
    │       │   │                   │   ├── 📄 AwtMouse.kt
    │       │   │                   │   ├── 📄 AwtSessionInfo.kt
    │       │   │                   │   └── 📄 AwtWire.kt
    │       │   │                   ├── 📁 component/
    │       │   │                   │   ├── 📄 AwtCanvasSurface.kt
    │       │   │                   │   ├── 📄 ExpandableText.kt
    │       │   │                   │   ├── 📄 FloatingHud.kt
    │       │   │                   │   ├── 📄 InstanceCard.kt
    │       │   │                   │   └── 📄 ResourceSummary.kt
    │       │   │                   ├── 📁 i18n/
    │       │   │                   │   ├── 📄 AppLanguage.kt
    │       │   │                   │   ├── 📄 LanguageOption.kt
    │       │   │                   │   ├── 📄 LocaleEngine.kt
    │       │   │                   │   ├── 📄 LocaleStorage.kt
    │       │   │                   │   ├── 📄 RcLocaleContext.kt
    │       │   │                   │   ├── 📄 RcLocalization.kt
    │       │   │                   │   ├── 📄 RcStringKeys.kt
    │       │   │                   │   ├── 📄 RcStringResources.kt
    │       │   │                   │   ├── 📄 RcStrings.kt
    │       │   │                   │   ├── 📄 RcStringsLoader.kt
    │       │   │                   │   └── 📄 RcValueFormat.kt
    │       │   │                   ├── 📁 model/
    │       │   │                   │   ├── 📄 Account.kt
    │       │   │                   │   ├── 📄 AccountRepository.kt
    │       │   │                   │   ├── 📄 ControlLayout.kt
    │       │   │                   │   ├── 📄 ControlLayoutDownloader.kt
    │       │   │                   │   ├── 📄 ControlLayoutImporter.kt
    │       │   │                   │   ├── 📄 ControlLayoutLibrary.kt
    │       │   │                   │   ├── 📄 ControlLayoutPackage.kt
    │       │   │                   │   ├── 📄 ControlLayoutRepository.kt
    │       │   │                   │   ├── 📄 GameInstance.kt
    │       │   │                   │   ├── 📄 GamepadDatabase.kt
    │       │   │                   │   ├── 📄 InstallProfile.kt
    │       │   │                   │   ├── 📄 InstanceRepository.kt
    │       │   │                   │   ├── 📄 LauncherSettings.kt
    │       │   │                   │   ├── 📄 MirrorMeasurer.kt
    │       │   │                   │   ├── 📄 ResourcePackData.kt
    │       │   │                   │   ├── 📄 SettingsRepository.kt
    │       │   │                   │   ├── 📄 ShaderPackData.kt
    │       │   │                   │   ├── 📄 TutorialState.kt
    │       │   │                   │   ├── 📄 VersionCatalog.kt
    │       │   │                   │   ├── 📄 WorldData.kt
    │       │   │                   │   └── 📁 json/
    │       │   │                   │       └── 📄 MiniJson.kt
    │       │   │                   ├── 📁 navigation/
    │       │   │                   │   └── 📄 RcNavigation.kt
    │       │   │                   ├── 📁 resource/
    │       │   │                   │   ├── 📄 FpsTracker.kt
    │       │   │                   │   └── 📄 ResourceMonitor.kt
    │       │   │                   ├── 📁 screen/
    │       │   │                   │   ├── 📄 AccountsScreen.kt
    │       │   │                   │   ├── 📄 AwtScreen.kt
    │       │   │                   │   ├── 📄 ControlLayoutLibraryScreen.kt
    │       │   │                   │   ├── 📄 ControllerScreen.kt
    │       │   │                   │   ├── 📄 DownloadsScreen.kt
    │       │   │                   │   ├── 📄 FileManagerScreen.kt
    │       │   │                   │   ├── 📄 HomeScreen.kt
    │       │   │                   │   ├── 📄 InstallWizardScreen.kt
    │       │   │                   │   ├── 📄 InstanceDetailScreen.kt
    │       │   │                   │   ├── 📄 InstancesScreen.kt
    │       │   │                   │   ├── 📄 ModBrowserScreen.kt
    │       │   │                   │   ├── 📄 ModpackImportScreen.kt
    │       │   │                   │   ├── 📄 OnboardingScreen.kt
    │       │   │                   │   ├── 📄 ResourcePackManagerScreen.kt
    │       │   │                   │   ├── 📄 SettingsScreen.kt
    │       │   │                   │   ├── 📄 ShaderPackManagerScreen.kt
    │       │   │                   │   ├── 📄 TranslationSettingsScreen.kt
    │       │   │                   │   └── 📄 WorldManagerScreen.kt
    │       │   │                   ├── 📁 theme/
    │       │   │                   │   ├── 📄 BackgroundCache.kt
    │       │   │                   │   ├── 📄 BackgroundConfig.kt
    │       │   │                   │   ├── 📄 RcBackground.kt
    │       │   │                   │   ├── 📄 Theme.kt
    │       │   │                   │   ├── 📄 ThemeData.kt
    │       │   │                   │   ├── 📄 ThemeEngine.kt
    │       │   │                   │   ├── 📄 ThemeStorage.kt
    │       │   │                   │   └── 📄 ThemeViewModel.kt
    │       │   │                   ├── 📁 translate/
    │       │   │                   │   └── 📄 TranslationViewModel.kt
    │       │   │                   └── 📁 viewmodel/
    │       │   │                       ├── 📄 AccountViewModel.kt
    │       │   │                       ├── 📄 AwtSurfaceViewModel.kt
    │       │   │                       ├── 📄 ControlLayoutLibraryViewModel.kt
    │       │   │                       ├── 📄 ControlLayoutViewModel.kt
    │       │   │                       ├── 📄 DashboardViewModel.kt
    │       │   │                       ├── 📄 DownloadsViewModel.kt
    │       │   │                       ├── 📄 FileManagerViewModel.kt
    │       │   │                       ├── 📄 InstallViewModel.kt
    │       │   │                       ├── 📄 InstanceDetailViewModel.kt
    │       │   │                       ├── 📄 LocaleViewModel.kt
    │       │   │                       ├── 📄 MainViewModel.kt
    │       │   │                       ├── 📄 ModpackImportViewModel.kt
    │       │   │                       ├── 📄 ResourcePackManagerViewModel.kt
    │       │   │                       ├── 📄 SettingsViewModel.kt
    │       │   │                       ├── 📄 ShaderPackManagerViewModel.kt
    │       │   │                       ├── 📄 TutorialViewModel.kt
    │       │   │                       └── 📄 WorldManagerViewModel.kt
    │       │   └── 📁 res/
    │       │       ├── 📁 values/
    │       │       │   ├── 📄 strings.xml
    │       │       │   └── 📄 themes.xml
    │       │       ├── 📁 values-en/
    │       │       │   └── 📄 strings.xml
    │       │       ├── 📁 values-zh-rTW/
    │       │       │   └── 📄 strings.xml
    │       │       └── 📁 xml/
    │       │           └── 📄 locales_config.xml
    │       └── 📁 test/
    │           ├── 📁 java/
    │           │   └── 📁 com/
    │           │       └── 📁 rc/
    │           │           └── 📁 launcher/
    │           │               └── 📁 ui/
    │           │                   ├── 📄 AdaptiveLayoutParityTest.kt
    │           │                   ├── 📄 AdaptiveLayoutTest.kt
    │           │                   ├── 📁 awt/
    │           │                   │   ├── 📄 AwtAndroidKeysTest.kt
    │           │                   │   ├── 📄 AwtControlTest.kt
    │           │                   │   ├── 📄 AwtGeometryTest.kt
    │           │                   │   ├── 📄 AwtInputTest.kt
    │           │                   │   ├── 📄 AwtMouseTest.kt
    │           │                   │   ├── 📄 AwtSessionInfoTest.kt
    │           │                   │   └── 📄 AwtWireTest.kt
    │           │                   ├── 📁 component/
    │           │                   │   ├── 📄 FloatingHudLogTest.kt
    │           │                   │   └── 📄 InstanceCardTest.kt
    │           │                   ├── 📁 i18n/
    │           │                   │   ├── 📄 AppLanguageTest.kt
    │           │                   │   ├── 📄 CatalogueParityTest.kt
    │           │                   │   ├── 📄 CoreStringsSourceTest.kt
    │           │                   │   ├── 📄 LocaleEngineTest.kt
    │           │                   │   ├── 📄 RcStringsTest.kt
    │           │                   │   └── 📄 RcValueFormatParityTest.kt
    │           │                   ├── 📁 model/
    │           │                   │   ├── 📄 AccountModelTest.kt
    │           │                   │   ├── 📄 ControlLayoutDownloaderTest.kt
    │           │                   │   ├── 📄 ControlLayoutImporterTest.kt
    │           │                   │   ├── 📄 ControlLayoutRepositoryTest.kt
    │           │                   │   ├── 📄 ControlLayoutTest.kt
    │           │                   │   ├── 📄 GameInstanceTest.kt
    │           │                   │   ├── 📄 GamepadDatabaseTest.kt
    │           │                   │   ├── 📄 InputCalibrationTest.kt
    │           │                   │   ├── 📄 InstallProfileTest.kt
    │           │                   │   ├── 📄 LauncherSettingsTest.kt
    │           │                   │   ├── 📄 MiniJsonTest.kt
    │           │                   │   ├── 📄 OrientationModeTest.kt
    │           │                   │   └── 📄 TutorialStateTest.kt
    │           │                   ├── 📁 navigation/
    │           │                   │   ├── 📄 RcAppSkeletonTest.kt
    │           │                   │   └── 📄 RcNavigationTest.kt
    │           │                   ├── 📁 resource/
    │           │                   │   └── 📄 ResourceMathTest.kt
    │           │                   ├── 📁 theme/
    │           │                   │   └── 📄 ThemeLogicTest.kt
    │           │                   ├── 📁 translate/
    │           │                   │   └── 📄 TranslationViewModelTest.kt
    │           │                   └── 📁 viewmodel/
    │           │                       ├── 📄 AccountViewModelTest.kt
    │           │                       ├── 📄 AwtSurfaceViewModelTest.kt
    │           │                       ├── 📄 ControlLayoutViewModelTest.kt
    │           │                       ├── 📄 DashboardViewModelTest.kt
    │           │                       ├── 📄 DownloadsViewModelTest.kt
    │           │                       ├── 📄 InstallViewModelTest.kt
    │           │                       ├── 📄 InstanceDetailViewModelTest.kt
    │           │                       ├── 📄 LocaleViewModelTest.kt
    │           │                       ├── 📄 SettingsViewModelTest.kt
    │           │                       └── 📄 TutorialViewModelTest.kt
    │           └── 📁 resources/
    │               ├── 📄 display_layout_golden.tsv
    │               ├── 📄 display_orientation_golden.tsv
    │               └── 📄 i18n_format_golden.tsv
    ├── 📄 build.gradle.kts
    ├── 📄 clippy.toml
    ├── 📁 config/
    │   └── 📁 detekt/
    │       └── 📄 detekt.yml
    ├── 📁 core/
    │   ├── 📄 build.gradle.kts
    │   ├── 📄 consumer-rules.pro
    │   └── 📁 src/
    │       └── 📁 main/
    │           ├── 📄 AndroidManifest.xml
    │           └── 📁 java/
    │               └── 📁 com/
    │                   └── 📁 rc/
    │                       └── 📁 launcher/
    │                           └── 📁 core/
    │                               ├── 📄 RcEventBus.kt
    │                               └── 📄 RustBridge.kt
    ├── 📁 docs/
    │   ├── 📄 ARCHITECTURE.md
    │   ├── 📄 BUILD.md
    │   ├── 📄 CONTRIBUTING.md
    │   ├── 📄 MODULES.md
    │   ├── 📄 auth.md
    │   ├── 📄 awt.md
    │   ├── 📄 ffi_event_bus.md
    │   ├── 📄 health_audit.md
    │   ├── 📄 i18n.md
    │   ├── 📄 launch.md
    │   ├── 📄 orientation.md
    │   └── 📄 rendering.md
    ├── 📁 fcl_apk/
    │   └── 📄 FCL-release-1.3.2.8-arm64-v8a.apk
    ├── 📁 gradle/
    │   ├── 📄 libs.versions.toml
    │   └── 📁 wrapper/
    │       ├── 📄 gradle-wrapper.jar
    │       └── 📄 gradle-wrapper.properties
    ├── 📄 gradle.properties
    ├── 📄 gradlew
    ├── 📄 gradlew.bat
    ├── 📁 logcat/
    ├── 📁 mobile_gules_apk/
    │   └── 📄 MobileGlues_2.0.0.apk
    ├── 📁 rc_apk/
    │   └── 📄 app-release.apk
    ├── 📁 runtime/
    │   ├── 📄 build.gradle.kts
    │   ├── 📄 generate_jre_manifest.py
    │   └── 📁 src/
    │       ├── 📁 main/
    │       │   ├── 📄 AndroidManifest.xml
    │       │   ├── 📁 assets/
    │       │   │   ├── 📁 app_runtime/
    │       │   │   │   ├── 📁 caciocavallo/
    │       │   │   │   │   ├── 📄 ResConfHack.jar
    │       │   │   │   │   ├── 📄 cacio-androidnw-1.10-SNAPSHOT.jar
    │       │   │   │   │   ├── 📄 cacio-shared-1.10-SNAPSHOT.jar
    │       │   │   │   │   └── 📄 version
    │       │   │   │   ├── 📁 caciocavallo17/
    │       │   │   │   │   ├── 📄 cacio-agent.jar
    │       │   │   │   │   ├── 📄 cacio-shared-1.19.1-SNAPSHOT.jar
    │       │   │   │   │   ├── 📄 cacio-tta-1.19.1-SNAPSHOT.jar
    │       │   │   │   │   └── 📄 version
    │       │   │   │   ├── 📁 java/
    │       │   │   │   │   ├── 📁 jre17/
    │       │   │   │   │   │   ├── 📄 bin-arm64.tar.xz
    │       │   │   │   │   │   ├── 📄 universal.tar.xz
    │       │   │   │   │   │   └── 📄 version
    │       │   │   │   │   ├── 📁 jre21/
    │       │   │   │   │   │   ├── 📄 bin-arm64.tar.xz
    │       │   │   │   │   │   ├── 📄 universal.tar.xz
    │       │   │   │   │   │   └── 📄 version
    │       │   │   │   │   ├── 📁 jre25/
    │       │   │   │   │   │   ├── 📄 bin-arm64.tar.xz
    │       │   │   │   │   │   ├── 📄 universal.tar.xz
    │       │   │   │   │   │   └── 📄 version
    │       │   │   │   │   ├── 📁 jre8/
    │       │   │   │   │   │   ├── 📄 bin-arm64.tar.xz
    │       │   │   │   │   │   ├── 📄 universal.tar.xz
    │       │   │   │   │   │   └── 📄 version
    │       │   │   │   │   └── 📄 jre_manifest.json
    │       │   │   │   ├── 📁 jna/
    │       │   │   │   │   ├── 📄 jna-arm64.zip
    │       │   │   │   │   └── 📄 version
    │       │   │   │   └── 📁 lwjgl/
    │       │   │   │       ├── 📁 3.3.3/
    │       │   │   │       │   ├── 📄 lwjgl-3.3.3-merged-modules.jar
    │       │   │   │       │   ├── 📄 lwjgl-freetype.jar
    │       │   │   │       │   ├── 📄 lwjgl-lwjglx.jar
    │       │   │   │       │   ├── 📄 lwjgl-nanovg.jar
    │       │   │   │       │   ├── 📄 lwjgl-openal.jar
    │       │   │   │       │   ├── 📄 lwjgl-shaderc.jar
    │       │   │   │       │   ├── 📄 lwjgl-spvc.jar
    │       │   │   │       │   ├── 📄 lwjgl-stb.jar
    │       │   │   │       │   ├── 📄 lwjgl-tinyfd.jar
    │       │   │   │       │   ├── 📄 lwjgl-vma.jar
    │       │   │   │       │   ├── 📄 lwjgl-vulkan.jar
    │       │   │   │       │   ├── 📄 lwjgl.jar
    │       │   │   │       │   ├── 📁 natives/
    │       │   │   │       │   │   └── 📁 arm64-v8a/
    │       │   │   │       │   │       ├── 📄 libfreetype.so
    │       │   │   │       │   │       ├── 📄 liblwjgl.so
    │       │   │   │       │   │       ├── 📄 liblwjgl_nanovg.so
    │       │   │   │       │   │       ├── 📄 liblwjgl_opengl.so
    │       │   │   │       │   │       ├── 📄 liblwjgl_stb.so
    │       │   │   │       │   │       ├── 📄 liblwjgl_tinyfd.so
    │       │   │   │       │   │       ├── 📄 liblwjgl_vma.so
    │       │   │   │       │   │       └── 📄 libshaderc.so
    │       │   │   │       │   └── 📄 version
    │       │   │   │       └── 📁 3.4.1/
    │       │   │   │           ├── 📄 lwjgl-3.4.1-merged-modules.jar
    │       │   │   │           ├── 📄 lwjgl-freetype.jar
    │       │   │   │           ├── 📄 lwjgl-lwjglx.jar
    │       │   │   │           ├── 📄 lwjgl-nanovg.jar
    │       │   │   │           ├── 📄 lwjgl-openal.jar
    │       │   │   │           ├── 📄 lwjgl-sdl.jar
    │       │   │   │           ├── 📄 lwjgl-shaderc.jar
    │       │   │   │           ├── 📄 lwjgl-spng.jar
    │       │   │   │           ├── 📄 lwjgl-spvc.jar
    │       │   │   │           ├── 📄 lwjgl-stb.jar
    │       │   │   │           ├── 📄 lwjgl-tinyfd.jar
    │       │   │   │           ├── 📄 lwjgl-vma.jar
    │       │   │   │           ├── 📄 lwjgl-vulkan.jar
    │       │   │   │           ├── 📄 lwjgl.jar
    │       │   │   │           ├── 📁 natives/
    │       │   │   │           │   └── 📁 arm64-v8a/
    │       │   │   │           │       ├── 📄 libfreetype.so
    │       │   │   │           │       ├── 📄 liblwjgl.so
    │       │   │   │           │       ├── 📄 liblwjgl_nanovg.so
    │       │   │   │           │       ├── 📄 liblwjgl_opengl.so
    │       │   │   │           │       ├── 📄 liblwjgl_spng.so
    │       │   │   │           │       ├── 📄 liblwjgl_stb.so
    │       │   │   │           │       ├── 📄 liblwjgl_tinyfd.so
    │       │   │   │           │       ├── 📄 liblwjgl_vma.so
    │       │   │   │           │       └── 📄 libshaderc.so
    │       │   │   │           └── 📄 version
    │       │   │   ├── 📄 mod_data.txt
    │       │   │   └── 📄 modpack_data.txt
    │       │   ├── 📁 java/
    │       │   │   └── 📁 com/
    │       │   │       └── 📁 rc/
    │       │   │           └── 📁 launcher/
    │       │   │               └── 📁 runtime/
    │       │   │                   └── 📄 JreManager.kt
    │       │   └── 📁 jniLibs/
    │       │       └── 📁 arm64-v8a/
    │       │           ├── 📄 libawt_headless.so
    │       │           └── 📄 libawt_xawt.so
    │       └── 📁 test/
    │           └── 📁 java/
    │               └── 📁 com/
    │                   └── 📁 rc/
    │                       └── 📁 launcher/
    │                           └── 📁 runtime/
    │                               └── 📄 JreManagerTest.kt
    ├── 📁 rust/
    │   ├── 📄 Cargo.lock
    │   ├── 📄 Cargo.toml
    │   └── 📁 crates/
    │       └── 📁 rc-launcher-core/
    │           ├── 📄 Cargo.toml
    │           ├── 📄 cbindgen.toml
    │           ├── 📁 examples/
    │           │   ├── 📄 awt_demo.rs
    │           │   ├── 📄 display_layout_golden.rs
    │           │   ├── 📄 i18n_demo.rs
    │           │   ├── 📄 i18n_format_golden.rs
    │           │   ├── 📄 input_demo.rs
    │           │   ├── 📄 launch_demo.rs
    │           │   └── 📄 rotation_demo.rs
    │           ├── 📁 i18n/
    │           │   ├── 📄 en.properties
    │           │   ├── 📄 zh-CN.properties
    │           │   └── 📄 zh-Hant.properties
    │           ├── 📄 rc_launcher.h
    │           └── 📁 src/
    │               ├── 📁 auth/
    │               │   ├── 📄 callback.rs
    │               │   ├── 📄 manager.rs
    │               │   ├── 📄 microsoft.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📄 model.rs
    │               │   ├── 📄 offline.rs
    │               │   ├── 📄 store.rs
    │               │   ├── 📄 third_party.rs
    │               │   ├── 📄 transport.rs
    │               │   └── 📄 vault.rs
    │               ├── 📄 capi.rs
    │               ├── 📄 discord.rs
    │               ├── 📄 display.rs
    │               ├── 📁 download/
    │               │   ├── 📄 client.rs
    │               │   ├── 📄 hash.rs
    │               │   ├── 📄 manager.rs
    │               │   ├── 📄 mod.rs
    │               │   └── 📄 testing.rs
    │               ├── 📄 error.rs
    │               ├── 📄 event.rs
    │               ├── 📄 ffi.rs
    │               ├── 📄 fs_ops.rs
    │               ├── 📁 game/
    │               │   ├── 📄 assets.rs
    │               │   ├── 📄 library.rs
    │               │   ├── 📄 manifest.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📄 platform.rs
    │               │   ├── 📄 resolve.rs
    │               │   ├── 📄 version.rs
    │               │   ├── 📁 version_data/
    │               │   │   ├── 📄 unlisted-versions.json
    │               │   │   ├── 📄 version-alias.csv
    │               │   │   └── 📄 versions.txt
    │               │   ├── 📄 version_extra.rs
    │               │   └── 📄 version_list.rs
    │               ├── 📄 gamepad.rs
    │               ├── 📁 i18n/
    │               │   ├── 📄 catalog.rs
    │               │   ├── 📄 format.rs
    │               │   ├── 📄 language.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📄 number.rs
    │               │   └── 📄 pack.rs
    │               ├── 📄 integration_tests.rs
    │               ├── 📄 jobs.rs
    │               ├── 📁 launch/
    │               │   ├── 📄 args.rs
    │               │   ├── 📄 awt.rs
    │               │   ├── 📄 awt_host.rs
    │               │   ├── 📄 classpath.rs
    │               │   ├── 📄 command.rs
    │               │   ├── 📄 crash.rs
    │               │   ├── 📄 engine.rs
    │               │   ├── 📄 env.rs
    │               │   ├── 📄 fakefx.rs
    │               │   ├── 📄 input.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📄 options.rs
    │               │   ├── 📄 process.rs
    │               │   ├── 📄 render.rs
    │               │   └── 📄 runtime_assets.rs
    │               ├── 📄 lib.rs
    │               ├── 📁 mods/
    │               │   ├── 📄 catalog.rs
    │               │   ├── 📁 catalog_data/
    │               │   │   ├── 📄 featured_modpacks.tsv
    │               │   │   └── 📄 featured_mods.tsv
    │               │   ├── 📄 conflict.rs
    │               │   ├── 📄 constraint.rs
    │               │   ├── 📁 fcl_data/
    │               │   │   ├── 📄 mod_data.txt
    │               │   │   └── 📄 modpack_data.txt
    │               │   ├── 📄 loader.rs
    │               │   ├── 📄 metadata.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📁 modpack/
    │               │   │   ├── 📄 curseforge.rs
    │               │   │   ├── 📄 importer.rs
    │               │   │   ├── 📄 integration_tests.rs
    │               │   │   ├── 📄 manifest.rs
    │               │   │   ├── 📄 mmc.rs
    │               │   │   ├── 📄 mod.rs
    │               │   │   └── 📄 modrinth.rs
    │               │   ├── 📄 resource_pack.rs
    │               │   └── 📄 shader.rs
    │               ├── 📁 net/
    │               │   ├── 📄 client.rs
    │               │   ├── 📄 dns.rs
    │               │   ├── 📄 mirror.rs
    │               │   ├── 📄 mod.rs
    │               │   └── 📄 proxy.rs
    │               ├── 📁 plugins/
    │               │   ├── 📄 fcl_apk.rs
    │               │   ├── 📄 mobile_glues.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📄 native_lib.rs
    │               │   ├── 📄 renderer.rs
    │               │   └── 📄 validation.rs
    │               ├── 📁 robust/
    │               │   ├── 📄 cache.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📄 reporter.rs
    │               │   └── 📄 retry.rs
    │               ├── 📁 runtime/
    │               │   ├── 📄 abi.rs
    │               │   ├── 📄 extract.rs
    │               │   ├── 📄 java_version.rs
    │               │   ├── 📄 manager.rs
    │               │   ├── 📄 manifest.rs
    │               │   ├── 📄 mod.rs
    │               │   └── 📄 source.rs
    │               ├── 📁 translate/
    │               │   ├── 📄 cache.rs
    │               │   ├── 📄 dictionary.rs
    │               │   ├── 📄 mod.rs
    │               │   ├── 📄 model.rs
    │               │   └── 📄 service.rs
    │               └── 📁 util/
    │                   ├── 📄 bufpool.rs
    │                   └── 📄 mod.rs
    ├── 📄 rust-toolchain.toml
    ├── 📄 rustfmt.toml
    ├── 📁 scripts/
    │   ├── 📁 __pycache__/
    │   │   ├── 📄 check_i18n.cpython-312.pyc
    │   │   ├── 📄 gen_android_strings.cpython-312.pyc
    │   │   └── 📄 i18n_common.cpython-312.pyc
    │   ├── 📄 check_awt_wire.py
    │   ├── 📄 check_i18n.py
    │   ├── 📄 check_layout_parity.py
    │   ├── 📄 gen_android_strings.py
    │   ├── 📄 health_audit.py
    │   └── 📄 i18n_common.py
    ├── 📄 settings.gradle.kts
    ├── 📄 task_list.txt
    └── 📄 task_list_2.txt
```
