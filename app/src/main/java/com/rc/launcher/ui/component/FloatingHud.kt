package com.rc.launcher.ui.component

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.background
import androidx.compose.ui.draw.border
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.DragIndicator
import androidx.compose.material.icons.filled.ListAlt
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Mouse
import androidx.compose.material.icons.filled.Screenshot
import androidx.compose.material.icons.filled.Swipe
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.Export
import androidx.compose.material.icons.filled.Snapshot
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.TextField
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonGroup
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipSet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.consumeAll
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.platform.LocalViewConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString
import com.rc.launcher.ui.resource.ResourceUsage
import kotlinx.coroutines.delay
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlin.math.roundToInt

// ===========================================================================
// Task 20: In-game floating HUD configuration & action types
// ===========================================================================

/**
 * Settings that govern how the in-game floating HUD looks and behaves.
 *
 * These are the "HUD-specific" knobs surfaced in the settings screen; the
 * broader game / launcher settings live in [LauncherSettings]. The values are
 * persisted by the caller (typically via a settings repository) and passed in
 * as an immutable snapshot so the composable stays pure and testable.
 *
 * Mirrors the design described in task 20: draggable positioning, auto-hide,
 * opacity, anti-misoperation zone and touch-through.
 *
 * @property visible        Whether the HUD is currently shown at all.
 * @property opacity        Global alpha applied to the whole HUD (0 = transparent, 1 = opaque).
 * @property autoHide       When true the expanded menu collapses to the compact
 *                          bar after [autoHideDelayMs] of pointer inactivity.
 * @property autoHideDelayMs Idle time in milliseconds before auto-collapse.
 * @property antiMisoperationZone  When true an invisible dead-zone ring around
 *                          the HUD consumes touches so a thumb resting near the
 *                          HUD does not reach the game surface beneath.
 * @property antiMisoperationRadiusDp  Radius of the dead zone (dp).
 * @property logTouchThrough   When the real-time log is open, allow touches to
 *                          pass through the log panel to the game below.
 */
data class FloatingHudConfig(
    val visible: Boolean = true,
    val opacity: Float = 0.85f,
    val autoHide: Boolean = true,
    val autoHideDelayMs: Long = 3000L,
    val antiMisoperationZone: Boolean = true,
    val antiMisoperationRadiusDp: Float = 48f,
    val logTouchThrough: Boolean = false,
) {
    /** Clamp all numeric fields so a corrupted store can never break the UI. */
    fun sanitized(): FloatingHudConfig = copy(
        opacity = opacity.coerceIn(0.05f, 1.0f),
        autoHideDelayMs = autoHideDelayMs.coerceIn(500L, 10_000L),
        antiMisoperationRadiusDp = antiMisoperationRadiusDp.coerceIn(0f, 120f),
    )

    companion object {
        /** The single default instance used everywhere a fresh config is needed. */
        val DEFAULTS: FloatingHudConfig = FloatingHudConfig()
    }
}

/**
 * Input mode toggle cycled by the HUD menu (task 12 integration).
 *
 * Mirrors the AwtPointerMode concept on the Kotlin side so the HUD can flip
 * between absolute touch and relative mouse without a round-trip through the
 * bridge.
 */
enum class InputMode {
    TOUCH,
    MOUSE,
    ;

    /** Flip between the two modes. */
    fun toggle(): InputMode = if (this == TOUCH) MOUSE else TOUCH

    /** Whether the game cursor is currently captured (MOUSE mode). */
    val isMouse: Boolean get() = this == MOUSE
}

/**
 * Log level classification for game output lines (task 21).
 *
 * Mirrors the Rust-side [LogStream] plus the common Minecraft log levels so the
 * UI can filter / colour-code consistently.
 */
enum class GameLogLevel {
    TRACE,
    DEBUG,
    INFO,
    WARN,
    ERROR,
    FATAL,
    STDOUT,
    STDERR,
    UNKNOWN,
    ;

    /** Whether this level should be shown by default. TRACE and DEBUG are
     *  hidden unless explicitly enabled (they are too noisy for a game HUD). */
    val isVisible: Boolean get() = this != TRACE && this != DEBUG

    /** String resource key for a human-readable label, for level badges and
     *  legends (task 21). Falls back to [colorName] for levels without a
     *  dedicated key. */
    val labelKey: String get() = when (this) {
        TRACE -> RcStringKeys.HUD_LOG_LEVEL_TRACE
        DEBUG -> RcStringKeys.HUD_LOG_LEVEL_DEBUG
        INFO -> RcStringKeys.HUD_LOG_LEVEL_INFO
        WARN -> RcStringKeys.HUD_LOG_LEVEL_WARN
        ERROR -> RcStringKeys.HUD_LOG_LEVEL_ERROR
        FATAL -> RcStringKeys.HUD_LOG_LEVEL_FATAL
        STDOUT -> RcStringKeys.HUD_LOG_LEVEL_STDOUT
        STDERR -> RcStringKeys.HUD_LOG_LEVEL_STDERR
        UNKNOWN -> RcStringKeys.HUD_LOG_LEVEL_UNKNOWN
    }

    /** Colour tint for the log line in the overlay. */
    val colorName: String get() = when (this) {
        TRACE -> "trace"
        DEBUG -> "debug"
        INFO -> "info"
        WARN -> "warn"
        ERROR -> "error"
        FATAL -> "fatal"
        STDOUT -> "stdout"
        STDERR -> "stderr"
        UNKNOWN -> "unknown"
    }
}

/**
 * Filter mode for the in-game log overlay (task 21).
 *
 * Controls which log lines are displayed, based on the classified
 * [GameLogLevel] carried by each [GameLogLine].
 */
enum class LogFilterMode {
    /** Show every line regardless of level. */
    ALL,
    /** Only stdout lines (non-error output). */
    STDOUT,
    /** Only stderr lines (game engine errors, crash dumps). */
    STDERR,
    /** Only ERROR and FATAL lines. */
    ERRORS,
    /** Only WARN, ERROR and FATAL lines. */
    WARN;

    /** Whether [line] passes this filter.
     *
     *  STDOUT / STDERR filter by the **stream origin** ([GameLogLine.isError]
     *  indicates stderr), so a log4j `INFO` line that came from stdout is
     *  correctly shown under STDOUT — not silently dropped because its
     *  classified level is `INFO` rather than `STDOUT`. */
    fun matches(line: GameLogLine): Boolean = when (this) {
        ALL -> true
        STDOUT -> !line.isError
        STDERR -> line.isError
        ERRORS -> line.logLevel == GameLogLevel.ERROR || line.logLevel == GameLogLevel.FATAL
        WARN -> line.logLevel == GameLogLevel.WARN ||
            line.logLevel == GameLogLevel.ERROR ||
            line.logLevel == GameLogLevel.FATAL
    }

    /** Whether a log level is shown when this filter is active.
     *
     *  For level-based filters (ERRORS, WARN) this is exact. For stream-based
     *  filters (STDOUT, STDERR) it lists the levels *typically* associated with
     *  that stream so a legend/visibility toggle can be built without a full
     *  [GameLogLine]. */
    fun showsLevel(level: GameLogLevel): Boolean = when (this) {
        ALL -> true
        STDOUT -> level == GameLogLevel.STDOUT || level == GameLogLevel.INFO ||
            level == GameLogLevel.DEBUG || level == GameLogLevel.TRACE ||
            level == GameLogLevel.UNKNOWN
        STDERR -> level == GameLogLevel.STDERR || level == GameLogLevel.WARN ||
            level == GameLogLevel.ERROR || level == GameLogLevel.FATAL
        ERRORS -> level == GameLogLevel.ERROR || level == GameLogLevel.FATAL
        WARN -> level == GameLogLevel.WARN ||
            level == GameLogLevel.ERROR ||
            level == GameLogLevel.FATAL
    }
}

/**
 * A crash snapshot captured at the moment the game process crashed (task 21).
 *
 * Passed from the launch engine / ViewModel to the overlay so the player can
 * see *what* killed the game without leaving the in-game screen. The `logTail`
 * is the last N lines from the ring buffer; `exitCode` / `signal` and
 * `categoryId` mirror the Rust `GameExit` / `CrashReport` model.
 *
 * @property timestamp  Epoch-ms the crash was detected.
 * @property exitCode   JVM exit code (`null` when killed by signal).
 * @property signal     Terminating signal, when any (Unix only).
 * @property categoryId Crash category id (e.g. `"out_of_memory"`, `"native_crash"`).
 * @property summary    Human-readable one-line verdict.
 * @property advice     Actionable advice text (already localised).
 * @property logTail    The last N log lines before the crash (newest first).
 */
data class CrashSnapshot(
    val timestamp: Long,
    val exitCode: Int?,
    val signal: Int?,
    val categoryId: String?,
    val summary: String,
    val advice: String,
    val logTail: List<String> = emptyList(),
)

/**
 * UI state for the log export operation (task 21).
 *
 * The overlay shows this so the player gets non-blocking feedback when
 * exporting a large log buffer to disk.
 */
sealed interface ExportState {
    /** Idle — no export in progress. */
    data object Idle : ExportState

    /** An export is in flight. */
    data object Exporting : ExportState

    /** Export finished; [path] is where the file was written. */
    data class Done(val path: String) : ExportState

    /** Export failed; [error] is a short human-readable reason. */
    data class Failed(val error: String) : ExportState
}

/**
 * A single line of game output captured from the process stdout/stderr stream.
 *
 * The Kotlin side receives each line through the event bus as a `log` event;
 * [isError] is true for stderr so the overlay can colour-code warnings and
 * crashes differently from ordinary output. [stream] carries the raw pipe
 * origin ("stdout" / "stderr") for display and for the stream-based
 * [LogFilterMode] filters (STDOUT / STDERR).
 *
 * @property text     The (already redacted) log line body.
 * @property isError  True when this line came from stderr (== [stream] == "stderr").
 * @property level    Optional log level tag from the Rust `game_log` event
 *                    (the classified log4j level or the stream type).
 * @property timestamp  Millisecond epoch when the line was captured.
 * @property logLevel  Classified log level for filtering & colouring (task 21).
 * @property stream    Pipe origin: "stdout" or "stderr" (may be null for tests).
 */
data class GameLogLine(
    val text: String,
    val isError: Boolean = false,
    val level: String? = null,
    val timestamp: Long = 0L,
    val logLevel: GameLogLevel = GameLogLevel.UNKNOWN,
    val stream: String? = null,
) {
    /** Human-readable prefix used in the overlay, e.g. `[stderr] ...`. */
    val formattedText: String get() =
        if (isError) "[stderr] $text" else text

    /** Short timestamp string for the overlay header, e.g. "14:23:45.123". */
    val timeString: String get() {
        if (timestamp == 0L) return ""
        val t = timestamp
        val ms = (t % 1000).toString().padStart(3, '0')
        val s = (t / 1000) % 60
        val m = (t / 60000) % 60
        val h = (t / 3600000) % 24
        return "%02d:%02d:%02d.%s".format(h, m, s, ms)
    }
}

/**
 * Actions the floating HUD menu can dispatch.
 *
 * The game surface / launch layer supplies the callbacks; the HUD itself is
 * stateless with respect to what each action *does* (open a log window,
 * flip the input mode, grab a frame, kill the process…).
 */
sealed interface FloatingHudAction {
    /** Open / bring forward the real-time game log overlay. */
    data object OpenLog : FloatingHudAction

    /** Flip between touch and mouse input mode. */
    data object SwitchInputMode : FloatingHudAction

    /** Capture a screenshot of the current game frame. */
    data object Screenshot : FloatingHudAction

    /** Force-quit the running game process. */
    data object ForceQuit : FloatingHudAction

    /** Export the current log buffer to a file (task 21). */
    data object ExportLog : FloatingHudAction

    /** Take a crash snapshot — capture current log + system state (task 21). */
    data object TakeSnapshot : FloatingHudAction

    /** Clear the in-memory log buffer (task 21). */
    data object ClearLog : FloatingHudAction
}

// ===========================================================================
// Task 20: Full in-game floating HUD overlay
// ===========================================================================

/**
 * In-game floating HUD overlay (task 20).
 *
 * This is the **full-featured** replacement for the original [FloatingHud]
 * performance badge. It lives on top of the game surface, renders a compact
 * performance strip (FPS / CPU / memory / storage) that is always visible, and
 * expands into a menu on demand offering:
 *
 *  - **Real-time log** — opens the game log window; a touch-through toggle
 *    lets touches fall through the log to the game underneath.
 *  - **Switch input mode** — cycle between touch and mouse (task 12).
 *  - **Screenshot** — grab the current frame and save it.
 *  - **Force quit** — stop the running game process.
 *  - **HUD settings** — opacity, auto-hide, anti-misoperation zone.
 *
 * Key interaction properties:
 *  - **Draggable** — grab the handle to reposition; the position is clamped to
 *    the screen and remembered across recompositions.
 *  - **Auto-hide** — the expanded menu collapses after [FloatingHudConfig.autoHideDelayMs]
 *    of inactivity; the compact strip is always shown.
 *  - **Opacity** — driven entirely by [FloatingHudConfig.opacity].
 *  - **Anti-misoperation zone** — an invisible ring around the HUD consumes
 *    touches, preventing thumbs from reaching the game while aiming at a menu
 *    button.
 *  - **Touch pass-through** — the overlay never captures touches outside its
 *    own bounds; the game surface beneath receives them.
 *
 * @param fps            Current game frame-rate (from the Rust core event bus,
 *                       task 21).
 * @param usage          Device resource usage snapshot.
 * @param inputMode      Current input mode (touch / mouse).
 * @param onInputModeChange  Called when the user toggles the input mode *from
 *                       the HUD* (the Rust bridge then pushes a new event).
 * @param config         Persistent settings snapshot.
 * @param onConfigChange Called when the user flips a setting in the HUD.
 * @param onAction       Called for every menu action.
 * @param onClose        Optional close handler — the launcher may intercept
 *                       this to hide the HUD or fall back to the dashboard.
 * @param modifier       Applied to the root layout.
 */
@Composable
fun GameFloatingHud(
    fps: Int,
    usage: ResourceUsage,
    inputMode: InputMode = InputMode.TOUCH,
    onInputModeChange: (InputMode) -> Unit = {},
    config: FloatingHudConfig = FloatingHudConfig.DEFAULTS,
    onConfigChange: (FloatingHudConfig) -> Unit = {},
    onAction: (FloatingHudAction) -> Unit = {},
    onClose: (() -> Unit)? = null,
    logLines: List<GameLogLine> = emptyList(),
    crashSnapshot: CrashSnapshot? = null,
    exportState: ExportState = ExportState.Idle,
    onExport: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val inspectionMode = LocalInspectionMode.current
    val density = LocalDensity.current
    val viewConfiguration = LocalViewConfiguration.current
    val haptic = LocalHapticFeedback.current

    // --- Drag state --------------------------------------------------------
    // The HUD position is an *offset* inside a full-screen Box. We store it as
    // a mutable float pair so it survives recompositions and config changes
    // (the caller holds [config] in a ViewModel / saved state).
    var offsetX by remember { mutableFloatStateOf(120f) }
    var offsetY by remember { mutableFloatStateOf(120f) }

    // --- Menu expansion state ---------------------------------------------
    var menuOpen by remember { mutableStateOf(false) }

    // --- Real-time log overlay state (task 20) ------------------------------
    // The log is shown/hidden by the "Open real-time log" button; it lives
    // inside the expanded menu so it follows the HUD when dragged.
    var logVisible by remember { mutableStateOf(false) }

    // --- Auto-hide (task 20) ----------------------------------------------
    // When the menu is open and [config.autoHide] is enabled, a coroutine
    // waits for the delay and then collapses the menu unless the user has
    // interacted in the meantime.
    val autoHideDelay = config.autoHideDelayMs
    val autoHideEnabled = config.autoHide && !inspectionMode
    LaunchedEffect(menuOpen, autoHideEnabled, autoHideDelay) {
        if (!menuOpen || !autoHideEnabled) return@LaunchedEffect
        delay(autoHideDelay)
        if (menuOpen) {
            menuOpen = false
        }
    }

    // Re-arm the auto-hide timer on any user interaction.
    fun resetAutoHide() {
        if (autoHideEnabled) {
            // Re-trigger the LaunchedEffect by toggling then restoring.
            menuOpen = !menuOpen
            menuOpen = !menuOpen
        }
    }

    // --- Screen dimensions (for drag clamping) -----------------------------
    val screenWidthPx = with(density) { viewConfiguration.screenWidth }
    val screenHeightPx = with(density) { viewConfiguration.screenHeight }

    val hudWidthDp = 200.dp
    val hudWidthPx = with(density) { hudWidthDp.roundToPx() }
    val hudHeightPx = if (menuOpen) {
        with(density) { 380.dp.roundToPx() }
    } else {
        with(density) { 80.dp.roundToPx() }
    }

    // Clamp the stored position so the HUD never slides off the visible area.
    val clampedX = offsetX.coerceIn(0f, (screenWidthPx - hudWidthPx).toFloat())
    val clampedY = offsetY.coerceIn(0f, (screenHeightPx - hudHeightPx).toFloat())

    // Anti-misoperation dead zone — invisible ring consumed around the HUD.
    val deadZoneRadius = if (config.antiMisoperationZone) {
        with(density) { config.antiMisoperationRadiusDp.dp.roundToPx() }.toFloat()
    } else 0f

    Box(
        modifier = modifier
            .fillMaxSize()
            .graphicsLayer(
                // Apply opacity at the layer level so it composites in one pass
                // rather than per-child (avoids per-pixel alpha artifacts).
                alpha = config.opacity.coerceIn(0f, 1f),
                // Let the GPU composite this in a single draw.
                compositingStrategy = CompositingStrategy.Offscreen,
            )
            .offset { IntOffset(clampedX.roundToInt(), clampedY.roundToInt()) },
    ) {
        // ---- Anti-misoperation dead zone (invisible, touch-consuming) ------
        if (deadZoneRadius > 0f) {
            // A larger transparent box centered on the HUD consumes touches in
            // the ring between the HUD edge and [deadZoneRadius].
            val zoneHalf = (hudWidthPx / 2 + deadZoneRadius)
            val zoneSizePx = (zoneHalf * 2).roundToInt()
            Box(
                Modifier
                    .size(zoneSizePx, zoneSizePx)
                    .offset {
                        IntOffset(
                            ((clampedX + hudWidthPx / 2) - zoneHalf).roundToInt(),
                            ((clampedY + hudHeightPx / 2) - zoneHalf).roundToInt(),
                        )
                    }
                    // Consume every gesture in this zone so it never reaches
                    // the game surface beneath.
                    .pointerInput(Unit) {
                        detectTapGestures(
                            onPress = { /* swallowed */ },
                        )
                    }
                    .clearAndSetSemantics {},
            )
        }

        // ---- The HUD itself ------------------------------------------------
        Surface(
            // NOTE: no onClick here — menu buttons have their own handlers.
            // Drag detection lives on the handle (see below), not on the whole
            // Surface, so button taps are never intercepted.
            modifier = Modifier
                .width(with(density) { hudWidthPx.toDp() })
                .height(with(density) { hudHeightPx.toDp() })
                .clearAndSetSemantics {},
            shape = RoundedCornerShape(16.dp),
            tonalElevation = 4.dp,
            shadowElevation = 8.dp,
            color = if (isSystemInDarkTheme()) {
                MaterialTheme.colorScheme.surface.copy(alpha = 0.9f)
            } else {
                MaterialTheme.colorScheme.surface.copy(alpha = 0.96f)
            },
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(12.dp),
            ) {
                // ---- Header: drag handle + title + expand + close ----------
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    // Drag handle — always present so the HUD is always movable.
                    // Only this area receives drag gestures, so button taps
                    // elsewhere on the HUD are unaffected.
                    Box(
                        Modifier
                            .size(36.dp, 28.dp)
                            .pointerInput(Unit) {
                                detectDragGestures(
                                    onDragStart = { },
                                    onDragEnd = { },
                                    onDragCancel = { },
                                    onDrag = { change, _ ->
                                        change.consumeAll()
                                        offsetX += change.delta.x
                                        offsetY += change.delta.y
                                        offsetX = offsetX.coerceIn(
                                            0f, (screenWidthPx - hudWidthPx).toFloat()
                                        )
                                        offsetY = offsetY.coerceIn(
                                            0f, (screenHeightPx - hudHeightPx).toFloat()
                                        )
                                        resetAutoHide()
                                    },
                                )
                            }
                            .testTag("hud_drag_handle"),
                    ) {
                        Icon(
                            imageVector = Icons.Default.DragIndicator,
                            contentDescription = rcString(RcStringKeys.HUD_DRAG_CONTENT_DESCRIPTION),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.align(Alignment.Center),
                        )
                    }

                    Spacer(Modifier.width(8.dp))

                    Text(
                        text = rcString(RcStringKeys.HUD_MENU_TITLE),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.weight(1f),
                    )

                    Spacer(Modifier.weight(1f))

                    // Expand / collapse toggle
                    IconButton(
                        onClick = {
                            menuOpen = !menuOpen
                            if (menuOpen) {
                                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            }
                        },
                        modifier = Modifier.size(32.dp).testTag("hud_expand_button"),
                    ) {
                        Icon(
                            imageVector = if (menuOpen) Icons.Default.Close else Icons.Default.MoreVert,
                            contentDescription = if (menuOpen) {
                                rcString(RcStringKeys.HUD_COLLAPSE_CONTENT_DESCRIPTION)
                            } else {
                                rcString(RcStringKeys.HUD_EXPAND_CONTENT_DESCRIPTION)
                            },
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }

                    // Close button — always visible for quick dismissal.
                    if (onClose != null) {
                        IconButton(
                            onClick = {
                                onClose()
                                haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                            },
                            modifier = Modifier.size(32.dp).testTag("hud_close_button"),
                        ) {
                            Icon(
                                imageVector = Icons.Default.Close,
                                contentDescription = rcString(RcStringKeys.HUD_CLOSE_CONTENT_DESCRIPTION),
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }

                Spacer(Modifier.height(8.dp))

                // ---- Performance strip (always visible, compact) -------------
                HudPerformanceStrip(
                    fps = fps,
                    usage = usage,
                    modifier = Modifier.testTag("hud_performance_strip"),
                )

                // ---- Expanded menu -------------------------------------------
                AnimatedVisibility(
                    visible = menuOpen,
                    enter = fadeIn(animationSpec = tween(200)),
                    exit = fadeOut(animationSpec = tween(150)),
                    modifier = Modifier.testTag("hud_menu"),
                ) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(top = 8.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        // Menu button: Open real-time log
                        HudMenuButton(
                            icon = Icons.Default.ListAlt,
                            label = rcString(RcStringKeys.HUD_ACTION_OPEN_LOG),
                            onClick = {
                                logVisible = !logVisible
                                onAction(FloatingHudAction.OpenLog)
                                resetAutoHide()
                            },
                        )

                        // Menu button: Switch input mode
                        HudMenuButton(
                            icon = if (inputMode.isMouse) Icons.Default.Mouse else Icons.Default.Swipe,
                            label = buildString {
                                append(rcString(RcStringKeys.HUD_ACTION_SWITCH_INPUT))
                                append(" · ")
                                append(
                                    if (inputMode.isMouse) {
                                        rcString(RcStringKeys.HUD_INPUT_MODE_MOUSE)
                                    } else {
                                        rcString(RcStringKeys.HUD_INPUT_MODE_TOUCH)
                                    },
                                )
                            },
                            onClick = {
                                onAction(FloatingHudAction.SwitchInputMode)
                                onInputModeChange(inputMode.toggle())
                                resetAutoHide()
                            },
                        )

                        // Menu button: Screenshot
                        HudMenuButton(
                            icon = Icons.Default.Screenshot,
                            label = rcString(RcStringKeys.HUD_ACTION_SCREENSHOT),
                            onClick = {
                                onAction(FloatingHudAction.Screenshot)
                                resetAutoHide()
                            },
                        )

                        // Menu button: Export log (task 21)
                        HudMenuButton(
                            icon = Icons.Default.Export,
                            label = rcString(RcStringKeys.HUD_LOG_EXPORT),
                            onClick = {
                                onAction(FloatingHudAction.ExportLog)
                                resetAutoHide()
                            },
                        )

                        // Menu button: Take snapshot (task 21)
                        HudMenuButton(
                            icon = Icons.Default.Snapshot,
                            label = rcString(RcStringKeys.HUD_LOG_SNAPSHOT),
                            onClick = {
                                onAction(FloatingHudAction.TakeSnapshot)
                                resetAutoHide()
                            },
                        )

                        // Menu button: Clear log (task 21)
                        HudMenuButton(
                            icon = Icons.Default.Clear,
                            label = rcString(RcStringKeys.HUD_LOG_CLEAR),
                            onClick = {
                                onAction(FloatingHudAction.ClearLog)
                                resetAutoHide()
                            },
                        )

                        // Menu button: Force quit
                        HudMenuButton(
                            icon = Icons.Default.Stop,
                            label = rcString(RcStringKeys.HUD_ACTION_FORCE_QUIT),
                            onClick = {
                                onAction(FloatingHudAction.ForceQuit)
                                resetAutoHide()
                            },
                        )

                        // --- Real-time log overlay (task 20) --------------------------
                        if (logVisible) {
                            HudLogOverlay(
                                logLines = logLines,
                                touchThrough = config.logTouchThrough,
                                onTouchThroughChange = { onConfigChange(config.copy(logTouchThrough = it)) },
                                onClose = { logVisible = false },
                                crashSnapshot = crashSnapshot,
                                exportState = exportState,
                                onExport = onExport,
                                modifier = Modifier.padding(top = 4.dp),
                            )
                        }

                        // --- Settings sub-panel ---
                        HudSettingsPanel(
                            config = config,
                            onConfigChange = onConfigChange,
                            modifier = Modifier.padding(top = 4.dp),
                        )
                    }
                }
            }
        }
    }
}

/**
 * The compact performance metrics strip shown at all times inside the HUD.
 */
@Composable
private fun HudPerformanceStrip(
    fps: Int,
    usage: ResourceUsage,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        // FPS
        Row(
            horizontalArrangement = Arrangement.SpaceBetween,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                "FPS",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                fps.coerceAtLeast(0).toString(),
                style = MaterialTheme.typography.titleSmall,
                color = if (fps >= 45) MaterialTheme.colorScheme.primary
                else if (fps >= 20) MaterialTheme.colorScheme.secondary
                else MaterialTheme.colorScheme.error,
                fontWeight = FontWeight.Bold,
            )
        }
        // CPU / Memory / Disk
        Row(
            horizontalArrangement = Arrangement.SpaceBetween,
            modifier = Modifier.fillMaxWidth(),
        ) {
            HudMetricSmall("CPU", "%.0f%%".format(usage.cpuPercent))
            HudMetricSmall("RAM", "%.0f%%".format(usage.memPercent))
            HudMetricSmall("Disk", "%.0f%%".format(usage.storagePercent))
        }
    }
}

@Composable
private fun HudMetricSmall(label: String, value: String) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            text = value,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurface,
        )
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
        )
    }
}

/**
 * A single menu button inside the expanded HUD menu.
 */
@Composable
private fun HudMenuButton(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(10.dp),
        color = MaterialTheme.colorScheme.secondaryContainer.copy(alpha = 0.5f),
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        modifier = modifier
            .fillMaxWidth()
            .testTag("hud_menu_button"),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
            )
            Text(
                text = label,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.weight(1f),
            )
        }
    }
}

/**
 * Settings sub-panel inside the expanded HUD menu.
 *
 * Provides sliders and switches for opacity, auto-hide, and the
 * anti-misoperation zone.
 */
@Composable
private fun HudSettingsPanel(
    config: FloatingHudConfig,
    onConfigChange: (FloatingHudConfig) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(vertical = 6.dp, horizontal = 4.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = rcString(RcStringKeys.HUD_SETTING_TITLE),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.primary,
        )

        // Opacity
        Text(
            text = rcString(RcStringKeys.HUD_SETTING_OPACITY) + ": " +
                "${(config.opacity * 100).roundToInt()}%",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Slider(
            value = config.opacity,
            onValueChange = { onConfigChange(config.copy(opacity = it)) },
            valueRange = 0.1f..1.0f,
            modifier = Modifier.testTag("hud_opacity_slider"),
        )

        // Auto-hide
        HudSettingRow(
            label = rcString(RcStringKeys.HUD_SETTING_AUTO_HIDE),
            summary = rcString(RcStringKeys.HUD_SETTING_AUTO_HIDE_SUMMARY),
            checked = config.autoHide,
            onCheckedChange = { onConfigChange(config.copy(autoHide = it)) },
            modifier = Modifier.testTag("hud_auto_hide_switch"),
        )

        // Anti-misoperation zone
        HudSettingRow(
            label = rcString(RcStringKeys.HUD_SETTING_ANTI_MISOPERATION),
            summary = rcString(RcStringKeys.HUD_SETTING_ANTI_MISOPERATION_SUMMARY),
            checked = config.antiMisoperationZone,
            onCheckedChange = { onConfigChange(config.copy(antiMisoperationZone = it)) },
            modifier = Modifier.testTag("hud_anti_misop_switch"),
        )

        // Log touch-through
        HudSettingRow(
            label = rcString(RcStringKeys.HUD_LOG_PASSTHROUGH),
            summary = rcString(RcStringKeys.HUD_LOG_PASSTHROUGH_SUMMARY),
            checked = config.logTouchThrough,
            onCheckedChange = { onConfigChange(config.copy(logTouchThrough = it)) },
            modifier = Modifier.testTag("hud_log_passthrough_switch"),
        )
    }
}

@Composable
private fun HudSettingRow(
    label: String,
    summary: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxWidth()) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = label,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                Text(
                    text = summary,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                )
            }
            Switch(
                checked = checked,
                onCheckedChange = onCheckedChange,
                modifier = Modifier.testTag("hud_setting_switch"),
            )
        }
    }
}


// ===========================================================================
// Task 21: Enhanced real-time log overlay (filter + search + crash snapshot)
// ===========================================================================

/**
 * Real-time log overlay panel shown inside the expanded HUD menu (task 20/21).
 *
 * Displays captured game stdout/stderr lines in a scrollable, monospace view
 * with **level filtering** and **text search** (task 21). When a crash
 * snapshot is available it is rendered as an amber banner above the log so the
 * player sees the verdict without leaving the game screen.
 *
 * Interaction model:
 *  - When [touchThrough] is **false** (default) the panel's background region
 *    *consumes* all tap/press gestures so a resting thumb does not reach the
 *    game surface beneath while the player reads the log.
 *  - When [touchThrough] is **true** the panel does **not** install any pointer
 *    input, so Compose lets every touch fall through to the underlying game
 *    surface — the player can keep aiming/casting while following the log.
 *    The panel is rendered slightly transparent to signal the active state.
 *
 * The panel is self-contained: it manages its own search text, filter mode and
 * scroll state (auto-scroll-on-new-line); the caller owns the backing
 * [logLines] list.
 *
 * @param logLines           Captured game log lines (bounded ring buffer on the
 *                           Rust side already, now carrying classified
 *                           [GameLogLine.logLevel]).
 * @param touchThrough       Whether touches should pass through to the game.
 * @param onTouchThroughChange Called when the user flips the touch-through switch.
 * @param onClose            Called to collapse the log overlay.
 * @param crashSnapshot      Optional crash info captured at crash moment;
 *                           shown as a banner when non-null (task 21).
 * @param exportState        Current export status; `null` or `Idle` shows no
 *                           special feedback, `Exporting` shows a spinner.
 * @param onExport           Called when the user taps the export button.
 */
@Composable
private fun HudLogOverlay(
    logLines: List<GameLogLine>,
    touchThrough: Boolean,
    onTouchThroughChange: (Boolean) -> Unit,
    onClose: () -> Unit,
    crashSnapshot: CrashSnapshot? = null,
    exportState: ExportState = ExportState.Idle,
    onExport: () -> Unit = {},
    modifier: Modifier = Modifier,
) {
    val scrollState = rememberScrollState()

    // --- Filter + search state (task 21) -------------------------------------
    // These are local to the overlay so toggling filters does not propagate
    // to the Rust ring buffer — the full unfiltered list is still retained on
    // the native side for crash diagnosis and export.
    var filterMode by remember { mutableStateOf(LogFilterMode.ALL) }
    var searchQuery by remember { mutableStateOf(TextFieldValue("")) }

    // Derive the displayed (filtered) lines from the full ring. We keep this
    // a plain list computation, not a flow, because it only runs on recomposition
    // triggered by the (slow) logLines / filter changes — not per-frame.
    val displayed: List<GameLogLine> = remember(logLines, filterMode, searchQuery) {
        logLines.asSequence()
            .filter { filterMode.matches(it) }
            .filter { sl ->
                val q = searchQuery.text
                q.isEmpty() || it.text.contains(q, ignoreCase = true)
            }
            .toList()
    }

    // Auto-scroll to the bottom when new lines arrive — but only if the
    // user is already near the bottom (so they can scroll up to inspect
    // without being yanked back down).
    LaunchedEffect(displayed.size) {
        val max = scrollState.maxValue
        val current = scrollState.value
        if (current >= max - 50) {
            scrollState.animateScrollTo(max)
        }
    }

    val bgAlpha = if (touchThrough) 0.15f else 0.35f
    val logBg = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = bgAlpha)

    Column(modifier = modifier.fillMaxWidth()) {
        // ---- Header: title + touch-through toggle + close ----------------
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                text = rcString(RcStringKeys.HUD_LOG_OVERLAY_TITLE),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.primary,
            )

            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = rcString(RcStringKeys.HUD_LOG_PASSTHROUGH),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Switch(
                    checked = touchThrough,
                    onCheckedChange = onTouchThroughChange,
                    modifier = Modifier
                        .size(36.dp, 24.dp)
                        .testTag("hud_log_passthrough_switch"),
                )
            }

            IconButton(
                onClick = onClose,
                modifier = Modifier
                    .size(28.dp)
                    .testTag("hud_log_close"),
            ) {
                Icon(
                    imageVector = Icons.Default.Close,
                    contentDescription = rcString(RcStringKeys.HUD_CLOSE_CONTENT_DESCRIPTION),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(16.dp),
                )
            }
        }

        Spacer(Modifier.height(4.dp))

        // ---- Search bar (task 21) ----------------------------------------
        OutlinedTextField(
            value = searchQuery,
            onValueChange = { searchQuery = it },
            placeholder = {
                Text(
                    text = rcString(RcStringKeys.HUD_LOG_SEARCH_PLACEHOLDER),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                )
            },
            leadingIcon = {
                Icon(
                    imageVector = Icons.Default.Search,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                    modifier = Modifier.size(16.dp),
                )
            },
            modifier = Modifier
                .fillMaxWidth()
                .testTag("hud_log_search"),
            maxLines = 1,
            keyboardOptions = androidx.compose.ui.text.input.KeyboardOptions(keyboardType = KeyboardType.Text),
        )

        Spacer(Modifier.height(4.dp))

        // ---- Level filter chips (task 21) --------------------------------
        LogFilterChips(
            current = filterMode,
            onSelected = { filterMode = it },
            modifier = Modifier
                .fillMaxWidth()
                .testTag("hud_log_filter_chips"),
        )

        Spacer(Modifier.height(4.dp))

        // ---- Export button + status (task 21) ----------------------------
        HudExportBar(
            exportState = exportState,
            onExport = onExport,
            canExport = logLines.isNotEmpty(),
            modifier = Modifier
                .fillMaxWidth()
                .testTag("hud_log_export_bar"),
        )

        Spacer(Modifier.height(4.dp))

        // ---- Crash snapshot banner (task 21) ------------------------------
        if (crashSnapshot != null) {
            HudCrashSnapshotBanner(
                snapshot = crashSnapshot,
                modifier = Modifier
                    .fillMaxWidth()
                    .testTag("hud_crash_snapshot"),
            )
            Spacer(Modifier.height(4.dp))
        }

        // ---- Log content (filtered + searched) --------------------------
        // The content box itself does NOT install a pointer input when
        // touch-through is enabled, so Compose lets every gesture reach the
        // game surface beneath. When touch-through is disabled we add a
        // transparent press-swallowing layer that also blocks scrolling from
        // leaking through to the game.
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(140.dp)
                .background(logBg, RoundedCornerShape(8.dp))
                .then(
                    if (touchThrough) {
                        Modifier // no pointer input -> touches pass through
                    } else {
                        Modifier
                            .pointerInput(Unit) {
                                detectTapGestures(
                                    onPress = { /* swallow */ },
                                )
                            }
                            .clearAndSetSemantics {}
                    },
                ),
        ) {
            val showLines: List<GameLogLine> = displayed.takeLast(300)
            if (showLines.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(8.dp),
                ) {
                    Text(
                        text = if (searchQuery.text.isNotBlank()) {
                            rcString(RcStringKeys.HUD_LOG_SEARCH_PLACEHOLDER)
                        } else {
                            rcString(RcStringKeys.HUD_ACTION_OPEN_LOG)
                        },
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                            .copy(alpha = 0.5f),
                        fontFamily = FontFamily.Monospace,
                        modifier = Modifier.align(Alignment.Center),
                    )
                }
            } else {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(scrollState)
                        .padding(8.dp),
                ) {
                    showLines.takeLast(300).forEach { line ->
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            modifier = Modifier
                                .fillMaxWidth()
                                .testTag("hud_log_line"),
                        ) {
                            // Level badge (task 21): a small coloured label showing
                            // the classified log level, using the i18n label keys.
                            Text(
                                text = rcString(line.logLevel.labelKey),
                                style = MaterialTheme.typography.labelSmall,
                                color = levelColor(line),
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold,
                                modifier = Modifier
                                    .padding(end = 4.dp)
                                    .widthIn(min = 36.dp),
                            )
                            Text(
                                text = line.formattedText,
                                style = MaterialTheme.typography.labelSmall,
                                color = levelColor(line),
                                fontFamily = FontFamily.Monospace,
                                lineHeight = MaterialTheme.typography.labelSmall.lineHeight,
                                modifier = Modifier.weight(1f),
                            )
                            // Timestamp (if present)
                            if (line.timestamp > 0L) {
                                Text(
                                    text = line.timeString,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f),
                                    fontFamily = FontFamily.Monospace,
                                    modifier = Modifier.padding(start = 4.dp),
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

/**
 * Pick a colour for a log line based on its classified [GameLogLevel]
 * (task 21: colour-coded levels).
 */
@Composable
private fun levelColor(line: GameLogLine) = when (line.logLevel) {
    GameLogLevel.ERROR, GameLogLevel.FATAL, GameLogLevel.STDERR ->
        MaterialTheme.colorScheme.error
    GameLogLevel.WARN ->
        MaterialTheme.colorScheme.tertiary
    GameLogLevel.INFO ->
        MaterialTheme.colorScheme.primary
    GameLogLevel.DEBUG ->
        MaterialTheme.colorScheme.secondary
    GameLogLevel.TRACE ->
        MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.5f)
    GameLogLevel.STDOUT, GameLogLevel.UNKNOWN ->
        MaterialTheme.colorScheme.onSurfaceVariant
}

/**
 * Filter chip row for the log overlay (task 21).
 *
 * Mirrors the compact chip design from the mod browser (task 8) so the look
 * stays consistent across the app.
 */
@Composable
private fun LogFilterChips(
    current: LogFilterMode,
    onSelected: (LogFilterMode) -> Unit,
    modifier: Modifier = Modifier,
) {
    val modes = listOf(
        LogFilterMode.ALL to RcStringKeys.HUD_LOG_FILTER_ALL,
        LogFilterMode.STDOUT to RcStringKeys.HUD_LOG_FILTER_STDOUT,
        LogFilterMode.STDERR to RcStringKeys.HUD_LOG_FILTER_STDERR,
        LogFilterMode.ERRORS to RcStringKeys.HUD_LOG_FILTER_ERRORS,
        LogFilterMode.WARN to RcStringKeys.HUD_LOG_FILTER_WARN,
    )
    Row(
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
        modifier = modifier,
    ) {
        modes.forEach { (mode, key) ->
            FilterChip(
                selected = current == mode,
                onClick = { onSelected(mode) },
                label = {
                    Text(
                        text = rcString(key),
                        style = MaterialTheme.typography.labelSmall,
                    )
                },
                modifier = Modifier
                    .height(28.dp)
                    .testTag("hud_log_filter_${mode.name}"),
            )
        }
    }
}

/**
 * Export button + live status text for the log overlay (task 21).
 */
@Composable
private fun HudExportBar(
    exportState: ExportState,
    onExport: () -> Unit,
    canExport: Boolean,
    modifier: Modifier = Modifier,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
        modifier = modifier,
    ) {
        Text(
            text = when (exportState) {
                is ExportState.Exporting -> rcString(RcStringKeys.HUD_LOG_EXPORTING)
                is ExportState.Done -> rcString(
                    RcStringKeys.HUD_LOG_EXPORT_DONE,
                    "path" to exportState.path,
                )
                is ExportState.Failed -> rcString(
                    RcStringKeys.HUD_LOG_EXPORT_FAILED,
                    "error" to exportState.error,
                )
                else -> ""
            },
            style = MaterialTheme.typography.labelSmall,
            color = when (exportState) {
                is ExportState.Failed -> MaterialTheme.colorScheme.error
                else -> MaterialTheme.colorScheme.onSurfaceVariant
            },
            maxLines = 1,
        )

        Spacer(Modifier.weight(1f))

        IconButton(
            onClick = {
                if (exportState is ExportState.Exporting) return@IconButton
                onExport()
            },
            enabled = canExport && exportState !is ExportState.Exporting,
            modifier = Modifier
                .size(32.dp)
                .testTag("hud_log_export_button"),
        ) {
            Icon(
                imageVector = Icons.Default.Export,
                contentDescription = rcString(RcStringKeys.HUD_LOG_EXPORT),
                tint = if (canExport) {
                    MaterialTheme.colorScheme.primary
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.3f)
                },
                modifier = Modifier.size(16.dp),
            )
        }
    }
}

/**
 * Amber crash-verdict banner rendered above the log when a crash snapshot is
 * available (task 21: "崩溃瞬间快照").
 *
 * Shows the category, exit code / signal, one-line summary, actionable advice
 * and a short verbatim tail of the log lines captured before the crash.
 */
@Composable
private fun HudCrashSnapshotBanner(
    snapshot: CrashSnapshot,
    modifier: Modifier = Modifier,
) {
    val bannerBg = MaterialTheme.colorScheme.errorContainer.copy(alpha = 0.3f)
    val bannerBorder = MaterialTheme.colorScheme.error.copy(alpha = 0.5f)

    Column(
        modifier = modifier
            .background(bannerBg, RoundedCornerShape(8.dp))
            .border(1.dp, bannerBorder, RoundedCornerShape(8.dp))
            .padding(8.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                text = rcString(RcStringKeys.HUD_LOG_SNAPSHOT),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onErrorContainer,
                fontWeight = FontWeight.Bold,
            )
            Text(
                text = java.text.SimpleDateFormat("HH:mm:ss", java.util.Locale.getDefault())
                    .format(snapshot.timestamp),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onErrorContainer.copy(alpha = 0.7f),
            )
        }

        Spacer(Modifier.height(2.dp))

        Text(
            text = snapshot.summary,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onErrorContainer,
        )

        if (snapshot.advice.isNotBlank()) {
            Spacer(Modifier.height(2.dp))
            Text(
                text = snapshot.advice,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onErrorContainer.copy(alpha = 0.85f),
            )
        }

        // Resolve i18n strings in the composable scope before building the
        // detail string (rcString is @Composable and cannot be called inside
        // the non-composable buildString lambda).
        val exitCodeLabel = rcString(RcStringKeys.HUD_CRASH_EXIT_CODE)
        val naLabel = rcString(RcStringKeys.HUD_CRASH_NA)
        val signalLabel = rcString(RcStringKeys.HUD_CRASH_SIGNAL)
        val detail = buildString {
            append(exitCodeLabel)
            append(": ")
            append(snapshot.exitCode?.toString() ?: naLabel)
            if (snapshot.signal != null) {
                append(" ($signalLabel: ${snapshot.signal})")
            }
            if (!snapshot.categoryId.isNullOrEmpty()) {
                append(" · ${snapshot.categoryId}")
            }
        }
        Spacer(Modifier.height(2.dp))
        Text(
            text = detail,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onErrorContainer.copy(alpha = 0.7f),
        )

        if (snapshot.logTail.isNotEmpty()) {
            Spacer(Modifier.height(4.dp))
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(60.dp)
                    .background(
                        MaterialTheme.colorScheme.scrim.copy(alpha = 0.3f),
                        RoundedCornerShape(4.dp),
                    )
                    .verticalScroll(rememberScrollState())
                    .padding(4.dp),
            ) {
                Column {
                    snapshot.logTail.takeLast(20).forEach { tailLine ->
                        Text(
                            text = tailLine,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onErrorContainer
                                .copy(alpha = 0.8f),
                            fontFamily = FontFamily.Monospace,
                        )
                    }
                }
            }
        }
    }
}

// ===========================================================================
// Backward-compatible wrapper: the simple performance badge used on the
// HomeScreen dashboard (task 12). This delegates to the original compact
// rendering path so existing call sites keep working.
// ===========================================================================

/**
 * Floating frame-rate / performance HUD, modelled after MCTier's
 * `GameHudOverlay`. Rendered as a z-stacked overlay above the dashboard
 * so it never pushes content around; [onClose] hides it.
 *
 * This is the **original** compact form — for the full in-game overlay with
 * menu actions use [GameFloatingHud].
 */
@Composable
fun FloatingHud(
    fps: Int,
    usage: ResourceUsage,
    modifier: Modifier = Modifier,
    onClose: (() -> Unit)? = null,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.92f),
        shadowElevation = 6.dp,
        tonalElevation = 2.dp,
    ) {
        Column(
            modifier = Modifier.width(180.dp).padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("性能 HUD", style = MaterialTheme.typography.labelMedium)
                Spacer(Modifier.weight(1f))
                onClose?.let {
                    IconButton(onClick = it, modifier = Modifier.size(20.dp)) {
                        Icon(
                            imageVector = Icons.Filled.Close,
                            contentDescription = "关闭 HUD",
                            modifier = Modifier.size(14.dp),
                        )
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                HudMetric("FPS", fps.toString())
                HudMetric("CPU", "%.0f%%".format(usage.cpuPercent))
                HudMetric("内存", "%.0f%%".format(usage.memPercent))
            }
            Text(
                "存储占用 %.0f%%".format(usage.storagePercent),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun HudMetric(label: String, value: String) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(
            text = value,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.primary,
        )
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
