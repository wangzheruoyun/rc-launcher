package com.rc.launcher.runtime

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Unit test for the JRE layout logic in [JreManager].
 * Lives in :runtime so the filesystem contract is verified without any native
 * or UI dependencies (see task 1 — clear dependency direction & testable units).
 */
class JreManagerTest {

    @Test
    fun jreRootUsesVersionAndAbi() {
        val root = JreManager.jreRoot(File("/data"), 17, "arm64-v8a")
        assertEquals(File("/data/jre/java17/arm64-v8a"), root)
    }

    @Test
    fun notProvisionedWithoutReleaseFile() {
        val dir = File.createTempFile("jre", "").apply { delete(); mkdirs() }
        try {
            assertFalse(JreManager.isProvisioned(dir))
            File(dir, "release").createNewFile()
            assertTrue(JreManager.isProvisioned(dir))
        } finally {
            dir.deleteRecursively()
        }
    }
}
