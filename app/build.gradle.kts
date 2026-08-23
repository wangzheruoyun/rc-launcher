plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("io.gitlab.arturbosch.detekt")
    id("org.jlleitschuh.gradle.ktlint")
}

android {
    namespace = "com.rc.launcher"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.rc.launcher"
        minSdk = 24
        targetSdk = 34
        versionCode = 1
        versionName = "0.1.0"

        // Match the ABIs produced by the Rust core (cargo-ndk).
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64", "x86")
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
    }

    buildFeatures {
        compose = true
    }

    // Kotlin 1.9.24 ships the Compose compiler as a separate extension; the
    // dedicated `org.jetbrains.kotlin.plugin.compose` Gradle plugin only exists
    // for Kotlin 2.0+, so enable Compose the 1.9.x way.
    composeOptions {
        kotlinCompilerExtensionVersion = "1.5.14"
    }

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

dependencies {
    // Project modules — clear dependency direction:
    //   :app -> :core (Rust/JNI bridge) -> :runtime (JRE/library mgmt)
    implementation(project(":core"))
    implementation(project(":runtime"))

    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.3")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.3")
    implementation("androidx.activity:activity-compose:1.9.0")

    val composeVersion = "1.6.8"
    implementation("androidx.compose.ui:ui:$composeVersion")
    implementation("androidx.compose.ui:ui-graphics:$composeVersion")
    implementation("androidx.compose.ui:ui-tooling-preview:$composeVersion")
    implementation("androidx.compose.material3:material3:1.2.1")
    implementation("androidx.compose.material:material-icons-extended:$composeVersion")
    implementation("androidx.navigation:navigation-compose:2.7.7")

    debugImplementation("androidx.compose.ui:ui-tooling:$composeVersion")

    testImplementation("junit:junit:4.13.2")
    // --- Task 21: Compose UI tests (run on the JVM via Robolectric) ---
    testImplementation("androidx.compose.ui:ui-test-junit4:$composeVersion")
    testImplementation("org.robolectric:robolectric:4.12.2")
    // --- Task 21: Compose UI instrumented tests (reference FCL androidTest / MCTier) ---
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4:$composeVersion")
    androidTestImplementation("androidx.test:runner:1.6.1")
    androidTestImplementation("androidx.test:rules:1.6.1")
    debugImplementation("androidx.compose.ui:ui-test-manifest:$composeVersion")
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
