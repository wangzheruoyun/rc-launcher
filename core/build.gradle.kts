plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.detekt)
    alias(libs.plugins.ktlint)
}

android {
    namespace = "com.rc.launcher.core"
    compileSdk = 34

    defaultConfig {
        minSdk = 24
        consumerProguardFiles("consumer-rules.pro")

        ndk {
            abiFilters += listOf("arm64-v8a")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }


    // The Rust core (.so) produced by cargo-ndk lands here and is packaged
    // into the consuming APK automatically via the AAR.
    sourceSets {
        getByName("main") {
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }

    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }
}

kotlinOptions {
    jvmTarget = "17"
}

dependencies {
    implementation(project(":runtime"))
    implementation(libs.androidx.core.ktx)
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
