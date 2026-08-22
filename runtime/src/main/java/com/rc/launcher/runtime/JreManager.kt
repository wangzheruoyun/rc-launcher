package com.rc.launcher.runtime

import java.io.File

/**
 * Manages JRE / native library provisioning on the Android filesystem.
 *
 * Scaffold only — detailed download / unzip / verification logic arrives in
 * later tasks (see task 6). Kept in its own module so :core and :app can share
 * filesystem layout without pulling in native code.
 */
object JreManager {

    /** Root directory for a given JRE version + ABI. */
    fun jreRoot(baseDir: File, javaVersion: Int, abi: String): File =
        File(baseDir, "jre/java$javaVersion/$abi")

    /** Whether a JRE has been extracted and looks valid. */
    fun isProvisioned(root: File): Boolean = File(root, "release").exists()
}
