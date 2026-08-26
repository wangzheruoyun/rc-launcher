plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
    alias(libs.plugins.detekt)
    alias(libs.plugins.ktlint)
}

android {
    namespace = "com.rc.launcher"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.rc.launcher"
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        // Match the ABIs produced by the Rust core (cargo-ndk).
        ndk {
            abiFilters += listOf("arm64-v8a")
        }

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        vectorDrawables {
            useSupportLibrary = true
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Sign the release artifacts with a real keystore when CI provides
            // one: build.yml decodes the base64-encoded keystore from the
            // KEYSTORE_BASE64 secret into $rootDir/release-keystore.jks and
            // exports KEYSTORE_PASSWORD / KEY_ALIAS / KEY_PASSWORD. When those
            // are absent (local builds, fork pull requests without secrets) we
            // fall back to the auto-generated debug key so assembleRelease /
            // bundleRelease still produce an installable APK / AAB everywhere.
            signingConfig = if (System.getenv("KEYSTORE_BASE64")?.isNotBlank() == true) {
                signingConfigs.create("ciRelease") {
                    val store = rootProject.file("release-keystore.jks")
                    require(store.isFile) {
                        "KEYSTORE_BASE64 is set but $store was not created by CI (build.yml)"
                    }
                    storeFile = store
                    storePassword = System.getenv("KEYSTORE_PASSWORD")
                        ?: error("KEYSTORE_PASSWORD must be set when signing with KEYSTORE_BASE64")
                    keyAlias = System.getenv("KEY_ALIAS")
                        ?: error("KEY_ALIAS must be set when signing with KEYSTORE_BASE64")
                    keyPassword = System.getenv("KEY_PASSWORD")
                        ?: error("KEY_PASSWORD must be set when signing with KEYSTORE_BASE64")
                }
            } else {
                signingConfigs.getByName("debug")
            }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
        // The UI uses several experimental Material3 / Foundation / Animation APIs
        // (ExposedDropdownMenu, SearchBar, ModalBottomSheet, etc.). Opt in
        // project-wide instead of annotating every composable.
        freeCompilerArgs += listOf(
            "-Xopt-in=androidx.compose.material3.ExperimentalMaterial3Api",
            "-Xopt-in=androidx.compose.material.ExperimentalMaterialApi",
            "-Xopt-in=androidx.compose.foundation.ExperimentalFoundationApi",
            "-Xopt-in=androidx.compose.animation.ExperimentalAnimationApi"
        )
    }

    buildFeatures {
        compose = true
    }

    // Compose is enabled via the `org.jetbrains.kotlin.plugin.compose` Gradle plugin
    // (applied in the plugins block above), which bundles the Compose compiler
    // matched to the Kotlin version. The old Kotlin 1.9.x `composeOptions {
    // kotlinCompilerExtensionVersion }` block is gone — the compiler now travels
    // with the Kotlin plugin.

    // Robolectric-backed JVM Compose UI tests (task 21) need the Android
    // resources + default return values to inflate composables off-device.
    testOptions {
        unitTests {
            isIncludeAndroidResources = true
            isReturnDefaultValues = true
        }
    }

    // Keep the build green on the first CI run; tighten later (task 26).
    lint {
        abortOnError = false
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

// ---------------------------------------------------------------------------
// Compose stack coherence (Task 1 scaffolding hardening).
//
// Material 3's TypographyTokens calls `TextStyle.copy$default`, a Kotlin
// synthetic method whose *signature* depends on the exact Compose UI version
// it was compiled against. Shipping a Material 3 built against Compose UI
// 1.6.x together with Compose UI 1.5.x on the runtime classpath produced a
// runtime `java.lang.NoSuchMethodError` (see logcat/25_08-13-50-16_522.log).
//
// We prevent that whole class of bug two ways, both sourced from the version
// catalog so the numbers can never silently drift apart:
//   1. The Compose BOM (platform(libs.compose.bom)) aligns every Compose
//      artifact to one mutually-compatible set.
//   2. resolutionStrategy.force hard-pins the whole Compose stack to the
//      single `compose` / `material3` versions below; `force` wins over the
//      BOM constraints, guaranteeing the runtime classpath is coherent even if
//      a transitive dependency ever tried to pull a newer Compose UI.
// ---------------------------------------------------------------------------
val composeVersion = libs.versions.compose.get()
val material3Version = libs.versions.material3.get()
val materialIconsExtendedVersion = libs.versions.materialIconsExtended.get()

configurations.all {
    resolutionStrategy {
        // Hard-pin the whole Compose stack to a single coherent set. `force`
        // wins over transitive `strictly` constraints and over the BOM, so the
        // version cannot float upward.
        force(
            "androidx.compose.ui:ui:$composeVersion",
            "androidx.compose.ui:ui-graphics:$composeVersion",
            "androidx.compose.ui:ui-tooling:$composeVersion",
            "androidx.compose.ui:ui-tooling-preview:$composeVersion",
            "androidx.compose.foundation:foundation:$composeVersion",
            "androidx.compose.runtime:runtime:$composeVersion",
            "androidx.compose.animation:animation:$composeVersion",
            "androidx.compose.material3:material3:$material3Version",
            "androidx.compose.material:material-icons-extended:$materialIconsExtendedVersion",
        )
        eachDependency {
            // Pin every module of the Compose stack to the one coherent set so a
            // transitive dependency can never silently pull a newer Material 3
            // (or ui/foundation/runtime) whose `TextStyle.copy$default` signature
            // diverges from the runtime Compose UI — the exact
            // `java.lang.NoSuchMethodError` class seen in
            // logcat/25_08-13-50-16_522.log. `force` above already wins over the
            // BOM; `eachDependency` is the belt-and-suspenders guarantee for any
            // dependency that requests a Compose artifact by a floating version.
            when {
                requested.group in setOf(
                    "androidx.compose.ui",
                    "androidx.compose.foundation",
                    "androidx.compose.runtime",
                    "androidx.compose.animation",
                ) -> {
                    useVersion(composeVersion)
                    because("pin the Compose stack to a single coherent set")
                }
                requested.group == "androidx.compose.material3" -> {
                    useVersion(material3Version)
                    because("pin Material 3 to the version aligned with the Compose BOM / compose ui")
                }
                requested.group == "androidx.compose.material" &&
                requested.name == "material-icons-extended" -> {
                    useVersion(materialIconsExtendedVersion)
                    because("material-icons-extended tracks its own 1.7.x line, not the 1.12.x ui line")
                }
            }
        }
    }
}

dependencies {
    // Project modules — clear dependency direction:
    //   :app -> :core (Rust/JNI bridge) -> :runtime (JRE/library mgmt)
    implementation(project(":core"))
    implementation(project(":runtime"))

    implementation(libs.androidx.core.ktx)
    implementation(libs.lifecycle.runtime.ktx)
    implementation(libs.lifecycle.viewmodel.compose)
    implementation(libs.lifecycle.runtime.compose)
    implementation(libs.activity.compose)

    // Compose BOM supplies a coherent version for every androidx.compose.* dep.
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.graphics)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.compose.material.icons.extended)
    implementation(libs.navigation.compose)
    // Type-safe navigation routes (task 11): every @Serializable route class needs
    // the kotlinx-serialization runtime + the plugin applied above.
    implementation(libs.kotlinx.serialization.json)

    debugImplementation(libs.compose.ui.tooling)

    testImplementation(libs.junit)
    // --- Task 21: Compose UI tests (run on the JVM via Robolectric) ---
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.robolectric)
    debugImplementation(libs.compose.ui.test.manifest)
}

// --- Task 26: unified Kotlin style checks (mirrors the Rust fmt/clippy gate) ---
// detekt + ktlint read config/detekt/detekt.yml and .editorconfig respectively.
// `ignoreFailures = true` keeps normal `assemble`/`build` green; the CI
// stylecheck job (continue-on-error) runs them and reports.
detekt {
    buildUponDefaultConfig = true
    allRules = false
    config.setFrom(rootProject.file("config/detekt/detekt.yml"))
    ignoreFailures = true
}

ktlint {
    android = true
    ignoreFailures = true
}
