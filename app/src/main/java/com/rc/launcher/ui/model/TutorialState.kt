package com.rc.launcher.ui.model

import android.content.Context
import android.content.SharedPreferences

/**
 * Onboarding / newbie tutorial state (task 14).
 *
 * Captures two pieces of information that the launcher needs:
 *
 *  * [completed] — has the user ever finished the linear first-run flow?
 *    Drives the auto-show on cold start (only the *first* launch shows it,
 *    every subsequent launch respects the user's choice and skips the banner).
 *  * [lastStep] — where the user *stopped* last time (0 for the welcome
 *    screen). Stored so a skipped tutorial resumes from the same place when
 *    the user picks "Rewatch" later, instead of always restarting from step 0.
 *
 * No Android dependency beyond [SharedPreferences], so the model is fully
 * unit-testable on the JVM through [InMemoryTutorialStateRepository].
 */
data class TutorialState(
    val completed: Boolean = false,
    val lastStep: Int = 0,
    /** Whether the skin-import tutorial has been finished at least once (task 23). */
    val skinTutorialCompleted: Boolean = false,
    /** Last visited step of the skin-import tutorial (task 23). */
    val skinTutorialLastStep: Int = 0,
) {
    /**
     * `true` while the launcher is still on its very first run — used by
     * [com.rc.launcher.ui.RcApp] to decide whether to overlay the onboarding
     * screen on top of the main scaffold.
     */
    val isFirstRun: Boolean get() = !completed

    /**
     * Total number of steps in the flow (welcome + 4 feature pages + done).
     * Kept here (not in the [com.rc.launcher.ui.screen.OnboardingScreen])
     * because the indicator text uses it and because the persistence layer
     * needs to clamp [lastStep] against the same constant.
     */
    val totalSteps: Int get() = TutorialStep.entries.size

    /**
     * The current step, clamped into the valid range so a corrupted / older
     * prefs file (or a future version with fewer pages) can never throw.
     */
    val currentStep: TutorialStep
        get() = TutorialStep.entries.getOrNull(lastStep) ?: TutorialStep.WELCOME

    /** Total number of skin-import tutorial steps (task 23). */
    val skinTutorialTotalSteps: Int get() = SkinTutorialStep.entries.size

    /**
     * The current skin-import tutorial step, clamped into the valid range so a
     * corrupted / older prefs file can never throw (task 23).
     */
    val currentSkinTutorialStep: SkinTutorialStep
        get() = SkinTutorialStep.entries.getOrNull(skinTutorialLastStep)
            ?: SkinTutorialStep.GET_SKIN
}

/**
 * One page of the onboarding flow (task 14). The order is the display order;
 * the ordinal is what gets persisted as [TutorialState.lastStep].
 *
 * The enum is intentionally a flat list (no branching) — onboarding must be a
 * pure linear path so a skipped tutorial is unambiguous and re-entry always
 * resumes on the same page.
 */
enum class TutorialStep {
    WELCOME,
    ADD_ACCOUNT,
    CREATE_INSTANCE,
    PICK_RENDERER,
    IMPORT_MODPACK,
    FINISHED,
    ;
}

/**
 * Persistence contract for [TutorialState]. The split mirrors the pattern used
 * by [SettingsRepository], [LocaleStorage] and [ThemeStorage]: a
 * [SharedPreferences]-backed implementation for the app, an in-memory one for
 * tests and previews.
 */
interface TutorialStateRepository {
    fun load(): TutorialState
    fun save(state: TutorialState)
}

/**
 * Volatile, process-local store used by previews and unit tests. Round-trips
 * the in-memory copy exactly, which is what the tutorial-state unit tests
 * assert against.
 */
class InMemoryTutorialStateRepository(
    initial: TutorialState = TutorialState(),
) : TutorialStateRepository {
    private var state: TutorialState = initial

    override fun load(): TutorialState = state
    override fun save(state: TutorialState) {
        this.state = state
    }
}

/**
 * [SharedPreferences]-backed [TutorialStateRepository]. One key per primitive
 * keeps the on-disk format forgiving: a missing key falls back to the
 * [TutorialState] defaults, so a partial / older prefs file never fails to
 * load (task 19 robustness).
 */
class SharedPreferencesTutorialStateRepository(
    context: Context,
) : TutorialStateRepository {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    override fun load(): TutorialState {
        val completed = prefs.getBoolean(KEY_COMPLETED, false)
        val rawStep = if (prefs.contains(KEY_LAST_STEP)) {
            prefs.getInt(KEY_LAST_STEP, 0)
        } else {
            0
        }
        val clamped = rawStep.coerceIn(0, TutorialStep.entries.size - 1)
        val skinCompleted = prefs.getBoolean(KEY_SKIN_COMPLETED, false)
        val rawSkinStep = if (prefs.contains(KEY_SKIN_LAST_STEP)) {
            prefs.getInt(KEY_SKIN_LAST_STEP, 0)
        } else {
            0
        }
        val skinClamped = rawSkinStep.coerceIn(0, SkinTutorialStep.entries.size - 1)
        return TutorialState(
            completed = completed,
            lastStep = clamped,
            skinTutorialCompleted = skinCompleted,
            skinTutorialLastStep = skinClamped,
        )
    }

    override fun save(state: TutorialState) {
        prefs.edit()
            .putBoolean(KEY_COMPLETED, state.completed)
            .putInt(KEY_LAST_STEP, state.lastStep.coerceIn(0, TutorialStep.entries.size - 1))
            .putBoolean(KEY_SKIN_COMPLETED, state.skinTutorialCompleted)
            .putInt(KEY_SKIN_LAST_STEP, state.skinTutorialLastStep.coerceIn(0, SkinTutorialStep.entries.size - 1))
            .apply()
    }

    companion object {
        private const val NAME = "rc_tutorial_state"
        private const val KEY_COMPLETED = "completed"
        private const val KEY_LAST_STEP = "last_step"
        private const val KEY_SKIN_COMPLETED = "skin_completed"
        private const val KEY_SKIN_LAST_STEP = "skin_last_step"
    }
}

/**
 * Process-wide holder for the installed [TutorialStateRepository], mirroring
 * [SettingsRepositories]. Defaults to the [InMemory] variant so the launcher is
 * still usable in unit tests / previews; [com.rc.launcher.RcApplication]
 * swaps in the [SharedPreferences]-backed implementation at startup.
 */
object TutorialStateRepositories {
    @Volatile
    private var _default: TutorialStateRepository? = null

    val default: TutorialStateRepository
        get() = _default ?: InMemoryTutorialStateRepository().also { _default = it }

    fun install(repository: TutorialStateRepository) {
        _default = repository
    }
}

// === Skin import tutorial state (task 23) =================================

/**
 * Steps of the embedded skin-import tutorial (task 23).
 *
 * Mirrors the linear structure of [TutorialStep]: a single ordinal is persisted
 * so re-entry (or a language change) always resumes on the same page. The
 * tutorial is triggered on demand from the skin-preview dialog on the
 * Accounts screen.
 *
 * The order is the display order; the ordinal is what gets persisted as
 * [TutorialState.skinTutorialLastStep].
 */
enum class SkinTutorialStep {
    /** Where to download / find skins (official + third-party). */
    GET_SKIN,
    /** File-format requirements (PNG 64×64 / 64×32 and variants). */
    FILE_FORMAT,
    /** How to pick a local image via the launcher's file chooser. */
    IMPORT,
    /** How to upload and apply the skin to the selected account. */
    APPLY,
    /** Final confirmation page. */
    DONE,
    ;
}

/** Convenience: total number of skin-tutorial steps. */
val SkinTutorialStep.Companion.totalSteps: Int
    get() = SkinTutorialStep.entries.size
