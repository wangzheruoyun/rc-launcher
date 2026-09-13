package com.rc.launcher.ui.viewmodel

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the download progress model (task 30).
 *
 * The [DownloadEntry] data class holds the per-download state surfaced on the
 * downloads screen. These tests verify the computed properties that drive the
 * progress bar, status badge, and byte-remaining label — all of which must be
 * correct for the UI to show accurate speed / status feedback.
 */
class DownloadsViewModelTest {

    @Test
    fun fractionIsZeroWhenTotalIsUnknown() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 42,
            total = null,
            speedBps = 0,
            status = "running",
        )
        assertEquals(0f, e.fraction, 0f)
        assertNull(e.bytesRemaining)
    }

    @Test
    fun fractionScalesCorrectlyWithTotal() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 250,
            total = 1000,
            speedBps = 512,
            status = "running",
        )
        assertEquals(0.25f, e.fraction, 0.001f)
        assertEquals(750L, e.bytesRemaining)
    }

    @Test
    fun fractionClampsAtOne() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 1200,
            total = 1000,
            speedBps = 0,
            status = "completed",
        )
        assertEquals(1.2f, e.fraction, 0.001f)
        assertTrue(e.isFinished)
    }

    @Test
    fun isFinishedIsTrueForCompleted() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 1000,
            total = 1000,
            speedBps = 0,
            status = "completed",
        )
        assertTrue(e.isFinished)
    }

    @Test
    fun isFinishedIsTrueForFailed() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 500,
            total = 1000,
            speedBps = 0,
            status = "failed",
            errorMessage = "checksum mismatch",
        )
        assertTrue(e.isFinished)
    }

    @Test
    fun isFinishedIsFalseForRunning() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 500,
            total = 1000,
            speedBps = 1024,
            status = "running",
        )
        assertFalse(e.isFinished)
    }

    @Test
    fun isFinishedIsFalseForPaused() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 500,
            total = 1000,
            speedBps = 0,
            status = "paused",
        )
        assertFalse(e.isFinished)
    }

    @Test
    fun fractionIsZeroWhenTotalIsZero() {
        val e = DownloadEntry(
            scope = "dl-1",
            label = "test.jar",
            downloaded = 0,
            total = 0,
            speedBps = 0,
            status = "completed",
        )
        assertEquals(0f, e.fraction, 0f)
    }
}
