package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.component.FloatingHudConfig
import com.rc.launcher.ui.component.InputMode
import com.rc.launcher.ui.model.InstanceRepository
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * Unit tests for the home-dashboard state machine (task 12): the one-tap launch
 * lifecycle, the floating-HUD visibility toggle, and the "最近游玩" recording.
 *
 * The [DashboardViewModel] is built through its test-only constructor with an
 * injected [LaunchExecutor] and a [CoroutineScope] backed by
 * [Dispatchers.Unconfined], so the asynchronous launch flow runs deterministically
 * on the JVM (no Android main looper / Robolectric needed). The fake executor
 * mirrors [SimulatedLaunchExecutor]'s gate: `run` parks until `cancel`, so a
 * launch settles on [LaunchState.Running] and only returns to [LaunchState.Idle]
 * when stopped.
 */
class DashboardViewModelTest {

    /** Deterministic executor: `run` parks on a gate until [cancel] completes it. */
    private class GateExecutor(private val prepareFails: Boolean = false) : LaunchExecutor {
        private var gate: CompletableDeferred<Unit>? = null
        var prepareCalls = 0
            private set
        var cancelled = false
            private set

        override suspend fun prepare(instance: GameInstance): Result<Unit> {
            prepareCalls++
            return if (prepareFails) {
                Result.failure(RuntimeException("preflight boom"))
            } else {
                Result.success(Unit)
            }
        }

        override suspend fun run(instance: GameInstance): Result<Unit> {
            gate = CompletableDeferred()
            gate!!.await() // stays "Running" until cancel()
            return Result.success(Unit)
        }

        override fun cancel() {
            cancelled = true
            gate?.complete(Unit)
        }
    }

    private val scope = CoroutineScope(Dispatchers.Unconfined + SupervisorJob())

    @Before
    fun setUp() {
        InstanceRepository.replaceAll(
            listOf(
                GameInstance(id = "a", name = "Alpha", version = "1.20.1"),
                GameInstance(id = "b", name = "Beta", version = "1.19.2"),
            ),
        )
    }

    private fun vm(executor: LaunchExecutor = GateExecutor()): DashboardViewModel {
        val v = DashboardViewModel()
        v.setTestDriver(executor, scope)
        return v
    }

    @Test
    fun launch_drivesIdleToRunningAndBackToIdle() {
        val exec = GateExecutor()
        val v = vm(exec)

        v.launch("a")
        val running = v.launchState.value
        assertTrue("expected Running after launch", running is LaunchState.Running)
        assertEquals("a", (running as LaunchState.Running).instanceId)

        v.stop()
        assertTrue("executor should have been cancelled", exec.cancelled)
        assertTrue("expected Idle after stop", v.launchState.value is LaunchState.Idle)
    }

    @Test
    fun launch_unknownIdIsIgnored() {
        val exec = GateExecutor()
        val v = vm(exec)
        v.launch("does-not-exist")
        assertTrue(v.launchState.value is LaunchState.Idle)
        assertEquals(0, exec.prepareCalls)
    }

    @Test
    fun launch_preflightFailureSurfacesError() {
        val exec = GateExecutor(prepareFails = true)
        val v = vm(exec)

        v.launch("a")
        val failed = v.launchState.value
        assertTrue("expected Failed after preflight error", failed is LaunchState.Failed)
        assertEquals("preflight boom", (failed as LaunchState.Failed).message)

        // The instance was still stamped as recently played (bubbles to 最近游玩).
        assertTrue(InstanceRepository.getById("a")!!.lastPlayed > 0L)

        v.dismissError()
        assertTrue(v.launchState.value is LaunchState.Idle)
    }

    @Test
    fun launch_ignoresConcurrentLaunchWhileRunning() {
        val exec = GateExecutor()
        val v = vm(exec)

        v.launch("a")
        v.launch("b") // ignored because 'a' is still running
        assertEquals("only the first launch should preflight", 1, exec.prepareCalls)
        assertTrue(v.launchState.value is LaunchState.Running)
    }

    @Test
    fun stopFromIdleIsSafe() {
        val exec = GateExecutor()
        val v = vm(exec)
        v.stop() // must not throw even when nothing is running
        assertTrue(v.launchState.value is LaunchState.Idle)
    }

    @Test
    fun hud_toggleFlipsVisibility() {
        val v = vm()
        assertFalse(v.hudVisible.value)
        v.toggleHud()
        assertTrue(v.hudVisible.value)
        v.toggleHud()
        assertFalse(v.hudVisible.value)
    }

    @Test
    fun launch_afterFailure_retriesSuccessfully() {
        // First launch fails preflight, then a retry (from Idle) succeeds.
        val failing = GateExecutor(prepareFails = true)
        val v = vm(failing)
        v.launch("a")
        assertTrue("expected Failed after preflight error", v.launchState.value is LaunchState.Failed)
        v.dismissError()
        assertTrue(v.launchState.value is LaunchState.Idle)

        // Retry with a working executor via the test seam.
        val working = GateExecutor()
        v.setTestDriver(working, scope)
        v.launch("a")
        assertTrue(v.launchState.value is LaunchState.Running)
        v.stop()
        assertTrue(v.launchState.value is LaunchState.Idle)
    }

    @Test
    fun stop_whileFailed_resetsSafely() {
        val exec = GateExecutor(prepareFails = true)
        val v = vm(exec)
        v.launch("a")
        assertTrue(v.launchState.value is LaunchState.Failed)
        v.stop() // must reset to Idle without throwing
        assertTrue(v.launchState.value is LaunchState.Idle)
    }

    @Test
    fun instancesMirrorRepository() {
        val v = vm()
        assertEquals(2, v.instances.value.size)
    }

    // --- Task 20: floating HUD controls ------------------------------------

    @Test
    fun hud_inputModeDefaultsToTouch() {
        val v = vm()
        assertFalse(v.inputMode.value.isMouse)
    }

    @Test
    fun hud_toggleInputModeFlipsTouchAndMouse() {
        val v = vm()
        assertEquals(InputMode.TOUCH, v.inputMode.value)
        v.toggleInputMode()
        assertEquals(InputMode.MOUSE, v.inputMode.value)
        v.toggleInputMode()
        assertEquals(InputMode.TOUCH, v.inputMode.value)
    }

    @Test
    fun hud_configStartsAtDefaults() {
        val v = vm()
        assertEquals(FloatingHudConfig.DEFAULTS, v.hudConfig.value)
    }

    @Test
    fun hud_setConfigPersists() {
        val v = vm()
        val custom = FloatingHudConfig(
            visible = true,
            opacity = 0.5f,
            autoHide = false,
            antiMisoperationZone = false,
        )
        v.setHudConfig(custom)
        assertEquals(0.5f, v.hudConfig.value.opacity, 0.001f)
        assertFalse(v.hudConfig.value.autoHide)
        assertFalse(v.hudConfig.value.antiMisoperationZone)
    }

    @Test
    fun hud_configSanitizesOutOfRangeValues() {
        val v = vm()
        val bad = FloatingHudConfig(
            opacity = 5f,
            autoHideDelayMs = 1L,
            antiMisoperationRadiusDp = 500f,
        )
        val sanitized = bad.sanitized()
        assertEquals(1.0f, sanitized.opacity, 0.001f)
        assertEquals(500L, sanitized.autoHideDelayMs) // clamped to 500 (min)
        assertTrue(sanitized.antiMisoperationRadiusDp <= 120f)
        v.setHudConfig(sanitized)
        assertEquals(sanitized, v.hudConfig.value)
    }

    @Test
    fun hud_forceQuitResetsToIdle() {
        val exec = GateExecutor()
        val v = vm(exec)
        v.launch("a")
        assertTrue(v.launchState.value is LaunchState.Running)
        v.forceQuit()
        assertTrue(v.launchState.value is LaunchState.Idle)
        assertTrue(exec.cancelled)
    }

    @Test
    fun hud_logLinesStartEmpty() {
        val v = vm()
        assertTrue(v.logLines.value.isEmpty())
    }

    @Test
    fun hud_logTouchThroughDefaultsToFalse() {
        val v = vm()
        assertFalse(v.hudConfig.value.logTouchThrough)
    }

    @Test
    fun hud_setLogTouchThroughPersists() {
        val v = vm()
        v.setHudConfig(v.hudConfig.value.copy(logTouchThrough = true))
        assertTrue(v.hudConfig.value.logTouchThrough)
        v.setHudConfig(v.hudConfig.value.copy(logTouchThrough = false))
        assertFalse(v.hudConfig.value.logTouchThrough)
    }

    @Test
    fun hud_clearLogEmptiesBuffer() {
        val v = vm()
        // Simulate a log event being delivered through the event bus.
        // RcEventBus.onEvent expects a JSON string (as the Rust core would send).
        val json = org.json.JSONObject()
            .put("seq", 1)
            .put("kind", "log")
            .put("message", "test log line")
            .put("scope", "global")
            .put("data", org.json.JSONObject().put("level", "stdout"))
            .toString()
        com.rc.launcher.core.RcEventBus.onEvent(json)
        assertFalse(v.logLines.value.isEmpty())
        v.clearLog()
        assertTrue(v.logLines.value.isEmpty())
    }
}

    // --- Task 21: log event classification & filtering -----------------------

    @Test
    fun log_event_classifiesInfoLevelFromStdout() {
        val v = vm()
        val json = org.json.JSONObject()
            .put("seq", 1)
            .put("kind", "log")
            .put("message", "[14:23:45] [main/INFO]: Setting user: Steve")
            .put("scope", "game")
            .put("data", org.json.JSONObject().put("level", "INFO").put("stream", "stdout"))
            .toString()
        com.rc.launcher.core.RcEventBus.onEvent(json)
        val line = v.logLines.value.lastOrNull()
        assertTrue(line != null)
        assertEquals(com.rc.launcher.ui.component.GameLogLevel.INFO, line!!.logLevel)
        assertFalse(line.isError)
        assertEquals("stdout", line.stream)
    }

    @Test
    fun log_event_classifiesErrorLevelFromStderr() {
        val v = vm()
        val json = org.json.JSONObject()
            .put("seq", 1)
            .put("kind", "log")
            .put("message", "Exception in thread \"main\" java.lang.OutOfMemoryError")
            .put("scope", "game")
            .put("data", org.json.JSONObject().put("level", "ERROR").put("stream", "stderr"))
            .toString()
        com.rc.launcher.core.RcEventBus.onEvent(json)
        val line = v.logLines.value.lastOrNull()
        assertTrue(line != null)
        assertEquals(com.rc.launcher.ui.component.GameLogLevel.ERROR, line!!.logLevel)
        assertTrue(line.isError)
        assertEquals("stderr", line.stream)
    }

    @Test
    fun log_event_classifiesRawStdoutWithoutLog4jMarker() {
        val v = vm()
        val json = org.json.JSONObject()
            .put("seq", 1)
            .put("kind", "log")
            .put("message", "Loading mods...")
            .put("scope", "game")
            .put("data", org.json.JSONObject().put("level", "stdout").put("stream", "stdout"))
            .toString()
        com.rc.launcher.core.RcEventBus.onEvent(json)
        val line = v.logLines.value.lastOrNull()
        assertTrue(line != null)
        assertEquals(com.rc.launcher.ui.component.GameLogLevel.STDOUT, line!!.logLevel)
        assertFalse(line.isError)
    }

    @Test
    fun log_event_classifiesRawStderrWithoutLog4jMarker() {
        val v = vm()
        val json = org.json.JSONObject()
            .put("seq", 1)
            .put("kind", "log")
            .put("message", "WARNING: ...")
            .put("scope", "game")
            .put("data", org.json.JSONObject().put("level", "stderr").put("stream", "stderr"))
            .toString()
        com.rc.launcher.core.RcEventBus.onEvent(json)
        val line = v.logLines.value.lastOrNull()
        assertTrue(line != null)
        assertEquals(com.rc.launcher.ui.component.GameLogLevel.STDERR, line!!.logLevel)
        assertTrue(line.isError)
    }

    @Test
    fun log_linesBoundedTo512Entries() {
        val v = vm()
        for (i in 1..600) {
            val json = org.json.JSONObject()
                .put("seq", i.toLong())
                .put("kind", "log")
                .put("message", "line $i")
                .put("scope", "game")
                .put("data", org.json.JSONObject().put("level", "stdout").put("stream", "stdout"))
                .toString()
            com.rc.launcher.core.RcEventBus.onEvent(json)
        }
        // The ring buffer must cap at 512; the oldest 88 lines must be
        // evicted so we never OOM the launcher on a verbose session.
        assertTrue(v.logLines.value.size <= 512)
    }
