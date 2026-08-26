package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.GameInstance
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
}
