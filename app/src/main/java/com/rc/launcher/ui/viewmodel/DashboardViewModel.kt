package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.rc.launcher.ui.component.FloatingHudConfig
import com.rc.launcher.ui.component.CrashSnapshot
import com.rc.launcher.ui.component.ExportState
import com.rc.launcher.ui.component.GameLogLine
import com.rc.launcher.ui.component.GameLogLevel
import com.rc.launcher.ui.component.InputMode
import com.rc.launcher.core.RcEventBus
import com.rc.launcher.core.RcEventKind
import com.rc.launcher.core.RcEventListener
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
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

    // --- Task 20: in-game floating HUD state --------------------------------

    /** Current input mode exposed to the HUD menu (task 12 integration). */
    private val _inputMode = MutableStateFlow(InputMode.TOUCH)
    val inputMode: StateFlow<InputMode> = _inputMode.asStateFlow()

    /** Persistent config for the floating HUD (opacity, auto-hide, etc.). */
    private val _hudConfig = MutableStateFlow(FloatingHudConfig.DEFAULTS)
    val hudConfig: StateFlow<FloatingHudConfig> = _hudConfig.asStateFlow()

    // --- Task 20: real-time game log lines for the HUD overlay ------------

    /** Bounded ring buffer of captured game log lines (stderr highlighted). */
    private val _logLines = MutableStateFlow<List<GameLogLine>>(emptyList())
    val logLines: StateFlow<List<GameLogLine>> = _logLines.asStateFlow()

    // --- Task 21: crash snapshot + export state for the log overlay --------

    /**
     * Crash snapshot captured at the moment the game process crashed (task 21).
     * `null` when there is no active crash to show.
     */
    private val _crashSnapshot = MutableStateFlow<CrashSnapshot?>(null)
    val crashSnapshot: StateFlow<CrashSnapshot?> = _crashSnapshot.asStateFlow()

    /**
     * Export status for the log overlay (task 21). Shows "Exporting…" /
     * "Exported to …" / "Export failed" feedback.
     */
    private val _exportState = MutableStateFlow<ExportState>(ExportState.Idle)
    val exportState: StateFlow<ExportState> = _exportState.asStateFlow()

    /**
     * Capture a crash snapshot from the current log buffer so the in-game
     * overlay can show the verdict without leaving the game screen.
     */
    fun takeCrashSnapshot(
        exitCode: Int? = null,
        signal: Int? = null,
        categoryId: String? = null,
        summary: String,
        advice: String,
        logTail: List<String> = emptyList(),
    ) {
        _crashSnapshot.value = CrashSnapshot(
            timestamp = System.currentTimeMillis(),
            exitCode = exitCode,
            signal = signal,
            categoryId = categoryId,
            summary = summary,
            advice = advice,
            logTail = logTail,
        )
    }

    /** Dismiss the current crash snapshot. */
    fun dismissCrashSnapshot() {
        _crashSnapshot.value = null
    }

    /** Mark a log export as in-progress. */
    fun setExportInProgress(inProgress: Boolean) {
        _exportState.value = if (inProgress) ExportState.Exporting else ExportState.Idle
    }

    /** Report a successful export. */
    fun setExportDone(path: String) {
        _exportState.value = ExportState.Done(path)
    }

    /** Report an export failure. */
    fun setExportFailed(error: String) {
        _exportState.value = ExportState.Failed(error)
    }

    private var logListener: RcEventListener? = null

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
                clearLog()
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
        clearLog() // discard stale game log when the process exits
    }

    /** Dismiss a launch failure banner. */
    fun dismissError() {
        _launchState.value = LaunchState.Idle
    }

    /** Toggle the floating performance HUD (also auto-shown while running). */
    fun toggleHud() {
        _hudVisible.value = !_hudVisible.value
    }

    // --- Task 20: floating HUD controls -----------------------------------

    /** Cycle the input mode (touch ↔ mouse), mirroring AwtPointerMode. */
    fun toggleInputMode() {
        _inputMode.value = _inputMode.value.toggle()
    }

    /** Update the HUD configuration (opacity, auto-hide, …). */
    fun setHudConfig(config: FloatingHudConfig) {
        _hudConfig.value = config
    }

    /** Force-quit the running game — delegates to the executor. */
    fun forceQuit() {
        stop()
    }

    // --- Task 20: log collection ------------------------------------------

    /**
     * Subscribe to `log` events from the Rust core so the in-game HUD overlay
     * can display them in real time. The subscription is scoped to the
     * ViewModel: it is started in [init] and torn down in [onCleared],
     * so it never leaks nor receives events after the ViewModel is gone.
     */
    private fun subscribeLogEvents() {
        if (logListener != null) return // already subscribed
        val l = RcEventListener { event ->
            if (event.kind != RcEventKind.LOG) return@RcEventListener
            val level = event.data?.optString("level")?.takeIf { !it.isNullOrEmpty() }
            val stream = event.data?.optString("stream")?.takeIf { !it.isNullOrEmpty() }
            val isError = stream == "stderr" || level?.equals("stderr", ignoreCase = true) == true
            val logLevel = classifyLogLevel(level, stream)
            val line = GameLogLine(
                text = event.message,
                isError = isError,
                level = level,
                timestamp = System.currentTimeMillis(),
                logLevel = logLevel,
                stream = stream,
            )
            _logLines.update { list ->
                // Bounded ring buffer: keep at most 512 lines, dropping oldest.
                if (list.size >= 512) list.subList(list.size - 511, list.size) + line
                else list + line
            }
        }
        logListener = l
        RcEventBus.addListener(l)
    }

    /**
     * Classify a [GameLogLevel] from the level and stream strings carried by
     * the Rust `game_log` event (task 21).
     *
     * The Rust side emits `level` as the classified Minecraft log4j level
     * (INFO / WARN / ERROR / DEBUG / TRACE / FATAL) or the raw stream type
     * (stdout / stderr) when no log4j marker was found. `stream` is always
     * the pipe origin.
     */
    private fun classifyLogLevel(level: String?, stream: String?): GameLogLevel {
        val upper = level?.uppercase()
        return when (upper) {
            "FATAL" -> GameLogLevel.FATAL
            "ERROR" -> GameLogLevel.ERROR
            "WARN" -> GameLogLevel.WARN
            "INFO" -> GameLogLevel.INFO
            "DEBUG" -> GameLogLevel.DEBUG
            "TRACE" -> GameLogLevel.TRACE
            "STDOUT" -> GameLogLevel.STDOUT
            "STDERR" -> GameLogLevel.STDERR
            else -> when (stream?.uppercase()) {
                "STDERR" -> GameLogLevel.STDERR
                "STDOUT" -> GameLogLevel.STDOUT
                else -> GameLogLevel.UNKNOWN
            }
        }
    }

    /** Drop all buffered log lines (e.g. when the game process exits). */
    fun clearLog() {
        _logLines.value = emptyList()
    }

    /**
     * Toggle the favorite flag of the instance with [id]. No-op when the id
     * does not exist (e.g. during a delete race). Persists straight back to
     * the [com.rc.launcher.ui.model.InstanceRepository] so the dashboard and
     * the detail screen stay in sync (task 18).
     */
    fun toggleFavorite(id: String) {
        val current = repository.getById(id) ?: return
        repository.update(current.copy(isFavorite = !current.isFavorite))
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

    init {
        // Start collecting game log events for the HUD overlay.
        subscribeLogEvents()
    }

    override fun onCleared() {
        super.onCleared()
        // Tear down the event-bus subscription to avoid leaks / stale events.
        logListener?.let { RcEventBus.removeListener(it) }
        logListener = null
        runJob?.cancel()
    }
}
