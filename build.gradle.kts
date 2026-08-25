// Top-level build file for RC Launcher.
// Modules: :app (Compose UI), :core (Rust/JNI bridge + runtime resources),
//          :runtime (JRE / library management).
plugins {
    id("com.android.application") version "8.5.2" apply false
    id("com.android.library") version "8.5.2" apply false
    id("org.jetbrains.kotlin.android") version "1.9.20" apply false
    id("io.gitlab.arturbosch.detekt") version "1.23.6" apply false
    id("org.jlleitschuh.gradle.ktlint") version "12.1.1" apply false
}
