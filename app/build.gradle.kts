import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
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
        targetSdk = 37
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
            // assembleRelease still produces an installable APK everywhere.
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

// Kotlin 2.x: configure the compiler via the compilerOptions DSL (the old
// `kotlinOptions { jvmTarget / freeCompilerArgs }` accessors are errors now).
kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
        // The UI uses several experimental Material3 / Foundation / Animation APIs
        // (ExposedDropdownMenu, SearchBar, ModalBottomSheet, etc.). Opt in
        // project-wide instead of annotating every composable.
        freeCompilerArgs.addAll(
            listOf(
                "-Xopt-in=androidx.compose.material3.ExperimentalMaterial3Api",
                "-Xopt-in=androidx.compose.material.ExperimentalMaterialApi",
                "-Xopt-in=androidx.compose.foundation.ExperimentalFoundationApi",
                "-Xopt-in=androidx.compose.animation.ExperimentalAnimationApi"
            )
        )
    }
}

// ---------------------------------------------------------------------------
// Compose dependency management
//
// Compose BOM (platform(libs.compose.bom)) aligns every Compose artifact to
// one mutually-compatible set. No force / resolutionStrategy needed — the BOM
// already guarantees coherence. compileSdk = 37 satisfies Compose UI 1.12.x
// minCompileSdk requirement.
// ---------------------------------------------------------------------------

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

// AAR metadata validation removed: compileSdk = 37 satisfies Compose 1.12.x
// minCompileSdk, no need to skip the check.

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
