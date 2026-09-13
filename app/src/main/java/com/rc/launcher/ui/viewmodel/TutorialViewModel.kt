package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.TutorialState
import com.rc.launcher.ui.model.TutorialStateRepositories
import com.rc.launcher.ui.model.TutorialStateRepository
import com.rc.launcher.ui.model.TutorialStep
import com.rc.launcher.ui.model.SkinTutorialStep
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * State container for the onboarding tutorial (task 14).
 *
 * Thin, deterministic holder of a single [TutorialState] [StateFlow]: every
 * mutator copies the current value, runs it through [TutorialState]'s built-in
 * clamping (so a partial / out-of-range input can never reach the persistence
 * layer) and saves the result via the injected [TutorialStateRepository]. No
 * Android dependency beyond the [ViewModel] base class, which keeps it fully
 * unit-testable on the JVM.
 *
 * The repository defaults to the process-wide [TutorialStateRepositories.default]
 * (installed from [com.rc.launcher.RcApplication]); tests pass an
 * [com.rc.launcher.ui.model.InMemoryTutorialStateRepository] explicitly.
 */
class TutorialViewModel(
    private val repository: TutorialStateRepository = TutorialStateRepositories.default,
) : ViewModel() {

    private val _state = MutableStateFlow(repository.load())
    val state: StateFlow<TutorialState> = _state.asStateFlow()

    /** Apply [next], publish and persist (failures are swallowed). */
    private fun commit(next: TutorialState) {
        val clamped = next.copy(
            lastStep = next.lastStep.coerceIn(0, TutorialStep.entries.size - 1),
            skinTutorialLastStep = next.skinTutorialLastStep.coerceIn(
                0, SkinTutorialStep.entries.size - 1,
            ),
        )
        _state.value = clamped
        runCatching { repository.save(clamped) }
    }

    /**
     * Move forward one step. If the user is already on [TutorialStep.FINISHED]
     * this is a no-op (the screen disables "Next" there) — calling it any
     * other way would happily bump past the end, which is exactly the bug this
     * guard exists to prevent.
     */
    fun next() {
        val current = _state.value
        val idx = TutorialStep.entries.indexOf(current.currentStep)
        if (idx < 0 || idx >= TutorialStep.entries.size - 1) return
        commit(current.copy(lastStep = idx + 1))
    }

    /**
     * Move back one step. If the user is already on [TutorialStep.WELCOME]
     * this is a no-op (the screen disables "Previous" there).
     */
    fun previous() {
        val current = _state.value
        val idx = TutorialStep.entries.indexOf(current.currentStep)
        if (idx <= 0) return
        commit(current.copy(lastStep = idx - 1))
    }

    /**
     * Skip to the last step and mark the tutorial as completed. Used by the
     * "Skip" button when the user does not want to see the full flow.
     */
    fun skip() {
        commit(
            _state.value.copy(
                lastStep = TutorialStep.entries.size - 1,
                completed = true,
            ),
        )
    }

    /**
     * Mark the tutorial as finished. Persists [completed] = true so the
     * next cold start does not auto-show the onboarding again, and parks
     * [lastStep] on the "finished" page in case the user wants to come back.
     */
    fun finish() {
        commit(
            _state.value.copy(
                lastStep = TutorialStep.entries.size - 1,
                completed = true,
            ),
        )
    }

    /**
     * Rewatch the tutorial. Resets the [completed] flag (so a future cold start
     * does not auto-show it unless the user re-enters the flow) and rewinds
     * [lastStep] to the welcome page.
     */
    fun restart() {
        commit(TutorialState(completed = false, lastStep = 0))
    }

    /**
     * Re-open the tutorial *without* resetting the [completed] flag. Used by
     * the "Rewatch" entry on the help screen — the user has already finished
     * the onboarding, this just lets them re-read it.
     */
    fun rewatch() {
        commit(_state.value.copy(lastStep = 0))
    }

    // === Skin import tutorial (task 23) =======================================

    /** Move the skin tutorial forward one step. */
    fun skinNext() {
        val current = _state.value
        val idx = SkinTutorialStep.entries.indexOf(current.currentSkinTutorialStep)
        if (idx >= 0 && idx < SkinTutorialStep.entries.size - 1) {
            commit(current.copy(skinTutorialLastStep = idx + 1))
        }
    }

    /** Move the skin tutorial back one step. */
    fun skinPrevious() {
        val current = _state.value
        val idx = SkinTutorialStep.entries.indexOf(current.currentSkinTutorialStep)
        if (idx > 0) {
            commit(current.copy(skinTutorialLastStep = idx - 1))
        }
    }

    /** Skip the skin tutorial. Marks it as completed without changing the step. */
    fun skinSkip() {
        commit(
            _state.value.copy(
                skinTutorialCompleted = true,
                skinTutorialLastStep = SkinTutorialStep.entries.size - 1,
            ),
        )
    }

    /** Mark the skin tutorial as finished. */
    fun skinFinish() {
        commit(
            _state.value.copy(
                skinTutorialCompleted = true,
                skinTutorialLastStep = SkinTutorialStep.entries.size - 1,
            ),
        )
    }

    /** Start the skin tutorial from the beginning. */
    fun skinStart() {
        commit(
            _state.value.copy(
                skinTutorialCompleted = false,
                skinTutorialLastStep = 0,
            ),
        )
    }

    /** Whether the skin tutorial has been completed at least once. */
    val skinTutorialCompleted: Boolean
        get() = _state.value.skinTutorialCompleted
}
