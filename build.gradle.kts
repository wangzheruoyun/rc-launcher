// Top-level build file for RC Launcher.
// Modules: :app (Compose UI), :core (Rust/JNI bridge + runtime resources),
//          :runtime (JRE / library management).
//
// All plugin + dependency versions are centralised in gradle/libs.versions.toml
// (the version catalog) so the multi-module project stays coherent and there is
// a single source of truth for every version (Task 1 — scaffolding).
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.android.library) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.detekt) apply false
    alias(libs.plugins.ktlint) apply false
}
