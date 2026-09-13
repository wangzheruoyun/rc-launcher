package com.rc.launcher.ui.component

import com.rc.launcher.ui.i18n.RcStringKeys
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the task-21 in-game log viewer data layer: the
 * [GameLogLevel] classification, [LogFilterMode] filtering and
 * [GameLogLine] construction. These are pure-Kotlin (no Compose / Android)
 * so they run on the JVM test runner without Robolectric.
 */
class FloatingHudLogTest {

    // --- GameLogLevel.isVisible / labelKey -----------------------------------

    @Test
    fun logLevel_isVisibleHidesVerboseByDefault() {
        // TRACE and DEBUG are too noisy for a game HUD and must be hidden
        // unless the user explicitly turns them on.
        assertFalse(GameLogLevel.TRACE.isVisible)
        assertFalse(GameLogLevel.DEBUG.isVisible)
        assertTrue(GameLogLevel.INFO.isVisible)
        assertTrue(GameLogLevel.WARN.isVisible)
        assertTrue(GameLogLevel.ERROR.isVisible)
        assertTrue(GameLogLevel.FATAL.isVisible)
        assertTrue(GameLogLevel.STDOUT.isVisible)
        assertTrue(GameLogLevel.STDERR.isVisible)
        assertTrue(GameLogLevel.UNKNOWN.isVisible)
    }

    @Test
    fun logLevel_labelKeyMapsToRcStringKeys() {
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_TRACE, GameLogLevel.TRACE.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_DEBUG, GameLogLevel.DEBUG.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_INFO, GameLogLevel.INFO.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_WARN, GameLogLevel.WARN.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_ERROR, GameLogLevel.ERROR.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_FATAL, GameLogLevel.FATAL.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_STDOUT, GameLogLevel.STDOUT.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_STDERR, GameLogLevel.STDERR.labelKey)
        assertEquals(RcStringKeys.HUD_LOG_LEVEL_UNKNOWN, GameLogLevel.UNKNOWN.labelKey)
    }

    // --- LogFilterMode.matches (stream-based STDOUT / STDERR) ---------------

    @Test
    fun filter_allShowsEveryLine() {
        val stdoutInfo = logLine("hello", isError = false, level = GameLogLevel.INFO)
        val stderrWarn = logLine("boom", isError = true, level = GameLogLevel.WARN)
        assertTrue(LogFilterMode.ALL.matches(stdoutInfo))
        assertTrue(LogFilterMode.ALL.matches(stderrWarn))
    }

    @Test
    fun filter_stdoutShowsAllStdoutLinesIncludingLog4j() {
        // A log4j INFO line from stdout must appear under STDOUT, even
        // though its classified level is INFO (not STDOUT).
        val infoFromStdout = logLine("hello", isError = false, level = GameLogLevel.INFO)
        val debugFromStdout = logLine("dbg", isError = false, level = GameLogLevel.DEBUG)
        val rawStdout = logLine("raw", isError = false, level = GameLogLevel.STDOUT)
        assertTrue(LogFilterMode.STDOUT.matches(infoFromStdout))
        assertTrue(LogFilterMode.STDOUT.matches(debugFromStdout))
        assertTrue(LogFilterMode.STDOUT.matches(rawStdout))
        // stderr lines must NOT appear
        assertFalse(LogFilterMode.STDOUT.matches(logLine("err", isError = true, level = GameLogLevel.STDERR)))
    }

    @Test
    fun filter_stderrShowsAllStderrLinesIncludingLog4j() {
        // A log4j WARN/ERROR line from stderr must appear under STDERR.
        val warnFromStderr = logLine("warn", isError = true, level = GameLogLevel.WARN)
        val errorFromStderr = logLine("err", isError = true, level = GameLogLevel.ERROR)
        val rawStderr = logLine("raw", isError = true, level = GameLogLevel.STDERR)
        assertTrue(LogFilterMode.STDERR.matches(warnFromStderr))
        assertTrue(LogFilterMode.STDERR.matches(errorFromStderr))
        assertTrue(LogFilterMode.STDERR.matches(rawStderr))
        // stdout lines must NOT appear
        assertFalse(LogFilterMode.STDERR.matches(logLine("ok", isError = false, level = GameLogLevel.INFO)))
    }

    @Test
    fun filter_errorsOnlyShowsErrorAndFatal() {
        assertTrue(LogFilterMode.ERRORS.matches(logLine("", level = GameLogLevel.ERROR)))
        assertTrue(LogFilterMode.ERRORS.matches(logLine("", level = GameLogLevel.FATAL)))
        assertTrue(LogFilterMode.ERRORS.matches(logLine("", level = GameLogLevel.STDERR)))
        assertFalse(LogFilterMode.ERRORS.matches(logLine("", level = GameLogLevel.INFO)))
        assertFalse(LogFilterMode.ERRORS.matches(logLine("", level = GameLogLevel.WARN)))
    }

    @Test
    fun filter_warnShowsWarnErrorFatal() {
        assertTrue(LogFilterMode.WARN.matches(logLine("", level = GameLogLevel.WARN)))
        assertTrue(LogFilterMode.WARN.matches(logLine("", level = GameLogLevel.ERROR)))
        assertTrue(LogFilterMode.WARN.matches(logLine("", level = GameLogLevel.FATAL)))
        assertFalse(LogFilterMode.WARN.matches(logLine("", level = GameLogLevel.INFO)))
        assertFalse(LogFilterMode.WARN.matches(logLine("", level = GameLogLevel.DEBUG)))
    }

    // --- LogFilterMode.showsLevel --------------------------------------------

    @Test
    fun showsLevel_allReturnsTrueForEverything() {
        for (level in GameLogLevel.entries) {
            assertTrue(LogFilterMode.ALL.showsLevel(level))
        }
    }

    @Test
    fun showsLevel_stdoutMatchesStdoutLevels() {
        assertTrue(LogFilterMode.STDOUT.showsLevel(GameLogLevel.STDOUT))
        assertTrue(LogFilterMode.STDOUT.showsLevel(GameLogLevel.INFO))
        assertTrue(LogFilterMode.STDOUT.showsLevel(GameLogLevel.DEBUG))
        assertTrue(LogFilterMode.STDOUT.showsLevel(GameLogLevel.TRACE))
        assertTrue(LogFilterMode.STDOUT.showsLevel(GameLogLevel.UNKNOWN))
        assertFalse(LogFilterMode.STDOUT.showsLevel(GameLogLevel.STDERR))
        assertFalse(LogFilterMode.STDOUT.showsLevel(GameLogLevel.WARN))
        assertFalse(LogFilterMode.STDOUT.showsLevel(GameLogLevel.ERROR))
        assertFalse(LogFilterMode.STDOUT.showsLevel(GameLogLevel.FATAL))
    }

    @Test
    fun showsLevel_stderrMatchesStderrLevels() {
        assertTrue(LogFilterMode.STDERR.showsLevel(GameLogLevel.STDERR))
        assertTrue(LogFilterMode.STDERR.showsLevel(GameLogLevel.WARN))
        assertTrue(LogFilterMode.STDERR.showsLevel(GameLogLevel.ERROR))
        assertTrue(LogFilterMode.STDERR.showsLevel(GameLogLevel.FATAL))
        assertFalse(LogFilterMode.STDERR.showsLevel(GameLogLevel.STDOUT))
        assertFalse(LogFilterMode.STDERR.showsLevel(GameLogLevel.INFO))
    }

    // --- GameLogLine helpers -------------------------------------------------

    @Test
    fun gameLogLine_formattedTextPrefixesStderr() {
        val stderr = logLine("boom", isError = true, level = GameLogLevel.ERROR)
        assertEquals("[stderr] boom", stderr.formattedText)

        val stdout = logLine("hello", isError = false, level = GameLogLevel.INFO)
        assertEquals("hello", stdout.formattedText)
    }

    @Test
    fun gameLogLine_timeStringFormatsCorrectly() {
        val line = logLine("test", timestamp = 123456789L)
        assertEquals("08:23:45.179", line.timeString)

        val noTime = logLine("test", timestamp = 0L)
        assertEquals("", noTime.timeString)
    }

    // --- Test helpers -------------------------------------------------------

    private fun logLine(
        text: String,
        isError: Boolean = false,
        level: String? = null,
        logLevel: GameLogLevel = GameLogLevel.UNKNOWN,
        timestamp: Long = 0L,
        stream: String? = null,
    ) = GameLogLine(
        text = text,
        isError = isError,
        level = level,
        timestamp = timestamp,
        logLevel = logLevel,
        stream = stream,
    )
}
