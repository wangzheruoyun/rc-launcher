package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.InMemoryTutorialStateRepository
import com.rc.launcher.ui.model.TutorialState
import com.rc.launcher.ui.model.TutorialStep
import com.rc.launcher.ui.model.SkinTutorialStep
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pure-logic unit tests for the tutorial view-model of task 14.
 *
 * They use the in-memory repository so the assertions run on the plain JVM
 * unit-test runner and feed the task-21 CI gate. The contracts asserted here
 * are the same the production [com.rc.launcher.ui.model.SharedPreferencesTutorialStateRepository]
 * upholds on disk \u2014 moving to a different persistence layer must not regress
 * them.
 */
class TutorialViewModelTest {

    @Test
    fun next_walksThroughAllSteps() {
        val repo = InMemoryTutorialStateRepository()
        val vm = TutorialViewModel(repo)
        for (expected in TutorialStep.entries.indices.drop(1)) {
            vm.next()
            assertEquals(expected, vm.state.value.lastStep)
        }
        // Past the end, `next` is a no-op (the screen disables the button,
        // but the VM must still defend against a stray call).
        vm.next()
        assertEquals(TutorialStep.entries.size - 1, vm.state.value.lastStep)
    }

    @Test
    fun previous_doesNotGoBelowWelcome() {
        val repo = InMemoryTutorialStateRepository(TutorialState(lastStep = 0))
        val vm = TutorialViewModel(repo)
        vm.previous()
        assertEquals(0, vm.state.value.lastStep)
    }

    @Test
    fun nextAndPrevious_roundTrip() {
        val vm = TutorialViewModel(InMemoryTutorialStateRepository())
        vm.next()
        vm.next()
        assertEquals(2, vm.state.value.lastStep)
        vm.previous()
        assertEquals(1, vm.state.value.lastStep)
    }

    @Test
    fun finish_marksCompletedAndLandsOnFinished() {
        val vm = TutorialViewModel(InMemoryTutorialStateRepository())
        // Move off the welcome page first so the test exercises the "user
        // advanced, then tapped finish" path, not the "tap finish from step
        // 0" path.
        vm.next()
        vm.finish()
        val s = vm.state.value
        assertTrue(s.completed)
        assertEquals(TutorialStep.entries.size - 1, s.lastStep)
        assertEquals(TutorialStep.FINISHED, s.currentStep)
    }

    @Test
    fun skip_marksCompleted() {
        val vm = TutorialViewModel(InMemoryTutorialStateRepository())
        vm.skip()
        val s = vm.state.value
        assertTrue(s.completed)
        assertEquals(TutorialStep.entries.size - 1, s.lastStep)
    }

    @Test
    fun restart_clearsCompletedAndRewindsToWelcome() {
        val vm = TutorialViewModel(
            InMemoryTutorialStateRepository(
                TutorialState(completed = true, lastStep = TutorialStep.entries.size - 1),
            ),
        )
        vm.restart()
        val s = vm.state.value
        assertFalse(s.completed)
        assertEquals(0, s.lastStep)
        assertEquals(TutorialStep.WELCOME, s.currentStep)
    }

    @Test
    fun rewatch_keepsCompletedAndRewinds() {
        // The "Settings \u2192 Help \u2192 Rewatch" path: the user has already finished
        // onboarding, but wants to re-read it. The completed flag must stay
        // `true` so the cold-start auto-show does *not* re-trigger.
        val vm = TutorialViewModel(
            InMemoryTutorialStateRepository(
                TutorialState(completed = true, lastStep = TutorialStep.entries.size - 1),
            ),
        )
        vm.rewatch()
        val s = vm.state.value
        assertTrue(s.completed)
        assertEquals(0, s.lastStep)
    }

    @Test
    fun mutations_arePersisted() {
        val repo = InMemoryTutorialStateRepository()
        val vm = TutorialViewModel(repo)
        vm.next()
        vm.next()
        vm.finish()
        // A freshly-built VM reading the same backing store must see the same
        // state \u2014 persistence is the only way the gate in RcApp can survive a
        // process restart.
        val replay = TutorialViewModel(repo)
        val s = replay.state.value
        assertTrue(s.completed)
        assertEquals(2, s.lastStep)
    }

    @Test
    fun loadedState_outOfRange_lastStep_clamped() {
        // A pre-existing prefs file from an older build, or a hand-edited
        // entry, must not crash the VM. The [TutorialState.currentStep]
        // accessor clamps via `getOrNull(...) ?: TutorialStep.WELCOME`, and
        // the first VM action re-clamps through `commit` so the persisted
        // value lands in range.
        val vm = TutorialViewModel(
            InMemoryTutorialStateRepository(TutorialState(lastStep = 999)),
        )
        // Reading the state never throws, and the computed step is in range.
        assertEquals(TutorialStep.WELCOME, vm.state.value.currentStep)
        assertTrue(vm.state.value.lastStep >= 0)
        // A single mutation writes the clamped value back.
        vm.next()
        assertTrue(vm.state.value.lastStep in 0..TutorialStep.entries.size - 1)
    }

    // === Skin tutorial (task 23) =============================================

    @Test
    fun skinNext_walksThroughAllSkinSteps() {
        val repo = InMemoryTutorialStateRepository()
        val vm = TutorialViewModel(repo)
        for (expected in SkinTutorialStep.entries.indices.drop(1)) {
            vm.skinNext()
            assertEquals(expected, vm.state.value.skinTutorialLastStep)
        }
        // Past the end, skinNext is a no-op.
        vm.skinNext()
        assertEquals(SkinTutorialStep.entries.size - 1, vm.state.value.skinTutorialLastStep)
    }

    @Test
    fun skinPrevious_doesNotGoBelowGetSkin() {
        val repo = InMemoryTutorialStateRepository(TutorialState(skinTutorialLastStep = 0))
        val vm = TutorialViewModel(repo)
        vm.skinPrevious()
        assertEquals(0, vm.state.value.skinTutorialLastStep)
    }

    @Test
    fun skinFinish_marksSkinTutorialCompleted() {
        val vm = TutorialViewModel(InMemoryTutorialStateRepository())
        vm.skinNext()
        vm.skinFinish()
        val s = vm.state.value
        assertTrue(s.skinTutorialCompleted)
        assertEquals(SkinTutorialStep.entries.size - 1, s.skinTutorialLastStep)
        assertEquals(SkinTutorialStep.DONE, s.currentSkinTutorialStep)
    }

    @Test
    fun skinSkip_marksSkinTutorialCompleted() {
        val vm = TutorialViewModel(InMemoryTutorialStateRepository())
        vm.skinSkip()
        val s = vm.state.value
        assertTrue(s.skinTutorialCompleted)
    }

    @Test
    fun skinStart_resetsSkinTutorial() {
        val vm = TutorialViewModel(
            InMemoryTutorialStateRepository(
                TutorialState(skinTutorialCompleted = true, skinTutorialLastStep = 4),
            ),
        )
        vm.skinStart()
        val s = vm.state.value
        assertFalse(s.skinTutorialCompleted)
        assertEquals(0, s.skinTutorialLastStep)
        assertEquals(SkinTutorialStep.GET_SKIN, s.currentSkinTutorialStep)
    }

    @Test
    fun skinTutorial_mutations_arePersisted() {
        val repo = InMemoryTutorialStateRepository()
        val vm = TutorialViewModel(repo)
        vm.skinNext()
        vm.skinFinish()
        val replay = TutorialViewModel(repo)
        val s = replay.state.value
        assertTrue(s.skinTutorialCompleted)
        assertEquals(SkinTutorialStep.entries.size - 1, s.skinTutorialLastStep)
    }
}
