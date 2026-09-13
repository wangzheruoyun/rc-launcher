package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pure-logic unit tests for the tutorial state model of task 14 (no Android
 * dependency, so they run on the plain JVM unit-test runner and feed the
 * task-21 CI gate).
 *
 * They lock the invariants that keep the onboarding flow honest:
 *
 *  * the `completed` / `lastStep` pair behaves predictably across rounds;
 *  * the in-memory repository round-trips exactly (so the ViewModel cannot
 *    silently lose the user's progress between the "Skip" press and the
 *    next launch);
 *  * an out-of-range `lastStep` from a corrupted / older prefs file is
 *    clamped instead of throwing (task 19 robustness).
 */
class TutorialStateTest {

    @Test
    fun defaults_areFirstRunAndWelcome() {
        val s = TutorialState()
        assertFalse("default state is not completed", s.completed)
        assertEquals("default step is WELCOME", 0, s.lastStep)
        assertTrue("default is first run", s.isFirstRun)
        assertEquals(TutorialStep.WELCOME, s.currentStep)
    }

    @Test
    fun completedState_isNotFirstRun() {
        val s = TutorialState(completed = true, lastStep = TutorialStep.entries.size - 1)
        assertFalse(s.isFirstRun)
        assertEquals(TutorialStep.FINISHED, s.currentStep)
    }

    @Test
    fun currentStep_clampsOutOfRangeLastStep() {
        // Older build with more steps, or a manually edited prefs file \u2014 the
        // `currentStep` accessor must never throw and must always land on a
        // real enum entry.
        val tooLarge = TutorialState(lastStep = 99)
        assertEquals(TutorialStep.WELCOME, tooLarge.currentStep)

        val negative = TutorialState(lastStep = -5)
        assertEquals(TutorialStep.WELCOME, negative.currentStep)
    }

    @Test
    fun totalSteps_matchesEnum() {
        // A change in the [TutorialStep] enum is caught here: the constant
        // tracks the enum size exactly so the indicator and persistence layer
        // agree.
        assertEquals(TutorialStep.entries.size, TutorialState().totalSteps)
    }

    @Test
    fun inMemoryRepository_roundTrips() {
        val repo = InMemoryTutorialStateRepository()
        val original = TutorialState(completed = false, lastStep = 2)
        repo.save(original)
        assertEquals(original, repo.load())
    }

    @Test
    fun inMemoryRepository_isProcessLocal() {
        val repo = InMemoryTutorialStateRepository()
        repo.save(TutorialState(completed = true, lastStep = 5))
        // A second instance must not see the first's writes \u2014 the test holds
        // the production contract that the [SharedPreferences]-backed repo is
        // what survives a process restart.
        val other = InMemoryTutorialStateRepository()
        assertEquals(TutorialState(), other.load())
    }

    @Test
    fun enum_isOrderedAndExhaustive() {
        // The enum order is the display order \u2014 the persistence layer indexes
        // by ordinal, so adding an entry in the middle would silently shift
        // every existing user's `lastStep`. The test at least catches
        // duplicates.
        val entries = TutorialStep.entries
        assertEquals(entries.size, entries.toSet().size)
        // The first step is always WELCOME and the last is always FINISHED \u2014
        // any future refactor that breaks that ordering will show up here.
        assertEquals(TutorialStep.WELCOME, entries.first())
        assertEquals(TutorialStep.FINISHED, entries.last())
    }

    // === Skin tutorial state (task 23) =====================================

    @Test
    fun skinTutorial_defaults_areNotCompletedAndGetSkin() {
        val s = TutorialState()
        assertFalse(s.skinTutorialCompleted)
        assertEquals(0, s.skinTutorialLastStep)
        assertEquals(SkinTutorialStep.GET_SKIN, s.currentSkinTutorialStep)
        assertEquals(SkinTutorialStep.entries.size, s.skinTutorialTotalSteps)
    }

    @Test
    fun skinTutorial_currentStep_clampsOutOfRange() {
        val tooLarge = TutorialState(skinTutorialLastStep = 99)
        assertEquals(SkinTutorialStep.GET_SKIN, tooLarge.currentSkinTutorialStep)

        val negative = TutorialState(skinTutorialLastStep = -5)
        assertEquals(SkinTutorialStep.GET_SKIN, negative.currentSkinTutorialStep)
    }

    @Test
    fun skinTutorial_enum_isOrdered() {
        val entries = SkinTutorialStep.entries
        assertEquals(5, entries.size)
        assertEquals(SkinTutorialStep.GET_SKIN, entries.first())
        assertEquals(SkinTutorialStep.DONE, entries.last())
        // No duplicates in the display order.
        assertEquals(entries.size, entries.toSet().size)
    }
}
