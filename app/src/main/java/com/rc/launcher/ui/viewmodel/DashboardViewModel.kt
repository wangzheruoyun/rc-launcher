package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** Lifecycle of a one-tap launch, surfaced on the home dashboard (task 12). */
sealed interface LaunchState {
    data object Idle : LaunchState
    data class Launching(val instanceId: String, val instanceName: String) : LaunchState
    data class Running(val instanceId: String, val instanceName: String) : LaunchState
    data class Failed(val instanceId: String, val message: String) : LaunchState
}

/**
 * Actual game spawning is delegated to a [LaunchExecutor] so the Rust-core
 * integration (task 7 `launchPreview` preflight + task 10 event bus) can replace
 * the simulator without touching the UI. The [SimulatedLaunchExecutor] below
 * exercises the full state machine on a device with no native library present.
 */
interface LaunchExecutor {
    /** Resolve + preflight (task 7). Returns failure detail on error. */
    suspend fun prepare(instance: GameInstance): Result<Unit>

    /**
     * Start the game process. Suspends until the process exits or [cancel] is
     * called, then returns (the ViewModel resets to [LaunchState.Idle]).
     */
    suspend fun run(instance: GameInstance): Result<Unit>

    /** Ask a running process to exit (best-effort, e.g. SIGTERM). */
    fun cancel()
}

/** Device-side simulator: builds for ~0.6 s, then "runs" until stopped. */
object SimulatedLaunchExecutor : LaunchExecutor {
    private var gate: CompletableDeferred<Unit>? = null

    override suspend fun prepare(instance: GameInstance): Result<Unit> {
        delay(600) // emulate classpath assembly + preflight checks
        return Result.success(Unit)
    }

    override suspend fun run(instance: GameInstance): Result<Unit> {
        gate = CompletableDeferred()
        try {
            gate!!.await() // stays "Running" until cancel() completes it
        } finally {
            gate = null
        }
        return Result.success(Unit)
    }

    override fun cancel() {
        gate?.complete(Unit)
    }
}

/**
 * Home-dashboard state container (task 12): the instance list, the one-tap
 * launch lifecycle and the floating HUD visibility.
 *
 * The list is owned by [InstanceRepository]; this ViewModel only mirrors it and
 * drives the launch state machine. All native / unsafe concerns stay behind the
 * [LaunchExecutor] boundary, keeping the UI pure and testable.
 *
 * The constructor is parameterless (true no-arg) so `viewModel()` can
 * instantiate it via reflection. The real [viewModelScope] is resolved lazily
 * and only on a device that provides `Dispatchers.Main`; [setTestDriver] lets
 * the unit tests inject a [LaunchExecutor] and a [CoroutineScope] (e.g.
 * `Dispatchers.Unconfined`) so the async launch flow can be driven
 * deterministically on the JVM without the Android main looper / Robolectric.
 */
class DashboardViewModel : ViewModel() {

    private val repository: InstanceRepository = InstanceRepository

    /** Production executor; swapped in tests via [setTestDriver]. */
    private var executor: LaunchExecutor = SimulatedLaunchExecutor

    /** Lazily resolved to [viewModelScope]; injected in tests to avoid Main. */
    private var scope: CoroutineScope? = null
    private fun launchScope(): CoroutineScope = scope ?: viewModelScope.also { scope = it }

    /** Live instance list, shared with the instances screen via the repository. */
    val instances: StateFlow<List<GameInstance>> = repository.instances

    private val _launchState = MutableStateFlow<LaunchState>(LaunchState.Idle)
    val launchState: StateFlow<LaunchState> = _launchState.asStateFlow()

    private val _hudVisible = MutableStateFlow(false)
    val hudVisible: StateFlow<Boolean> = _hudVisible.asStateFlow()

    private var runJob: Job? = null

    /** One-tap quick launch from the dashboard / instance card. */
    fun launch(id: String) {
        val inst = repository.instances.value.firstOrNull { it.id == id } ?: return
        val current = _launchState.value
        if (current is LaunchState.Launching || current is LaunchState.Running) return

        // Immediately bubble to "最近游玩" before the (async) launch resolves.
        repository.recordPlayed(id)
        _launchState.value = LaunchState.Launching(inst.id, inst.name)

        runJob?.cancel()
        runJob = launchScope().launch {
            // prepare() may either throw or return a failed Result; both must be
            // surfaced as LaunchState.Failed. The Rust-core integration (task 7
            // launchPreview preflight) reports problems via a failed Result, not
            // an exception, so we inspect the inner Result as well as any throw.
            val prepared = runCatching { executor.prepare(inst) }
            val preparedResult = prepared.getOrNull()
            if (prepared.isFailure || preparedResult?.isFailure == true) {
                val message = prepared.exceptionOrNull()?.message
                    ?: preparedResult?.exceptionOrNull()?.message
                    ?: "准备失败"
                _launchState.value = LaunchState.Failed(inst.id, message)
                return@launch
            }
            _launchState.value = LaunchState.Running(inst.id, inst.name)
            // Suspends until the game exits (or the user stops it).
            runCatching { executor.run(inst) }
            _launchState.value = LaunchState.Idle
        }
    }

    /** Stop a running instance (best-effort) and return to idle. */
    fun stop() {
        executor.cancel()
        runJob?.cancel()
        _launchState.value = LaunchState.Idle
    }

    /** Dismiss a launch failure banner. */
    fun dismissError() {
        _launchState.value = LaunchState.Idle
    }

    /** Toggle the floating performance HUD (also auto-shown while running). */
    fun toggleHud() {
        _hudVisible.value = !_hudVisible.value
    }

    /**
     * Test-only seam: replace the [LaunchExecutor] and the coroutine [scope]
     * used by [launch] / [stop]. Lets the launch state machine be driven on the
     * JVM without the Android main looper.
     */
    internal fun setTestDriver(executor: LaunchExecutor, scope: CoroutineScope) {
        this.executor = executor
        this.scope = scope
    }
}
