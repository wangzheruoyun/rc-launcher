package com.rc.launcher.ui.awt

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Unit tests for the physical keyboard & mouse layer (task 12).
 *
 * These are the guard rails for the *contract with the Rust core*: the ids, the
 * bounds and the JSON keys asserted here are the ones `launch::input` parses, and
 * `scripts/check_awt_wire.py` checks that the two sources still agree.
 */
class AwtMouseTest {

    // ---- pointer mode ------------------------------------------------------

    @Test
    fun pointerModeIdsMirrorTheCore() {
        assertEquals("absolute", AwtPointerMode.ABSOLUTE.id)
        assertEquals("captured", AwtPointerMode.CAPTURED.id)
        assertEquals(AwtPointerMode.CAPTURED, AwtPointerMode.fromId("captured"))
        // Unknown keeps the safe default instead of throwing.
        assertEquals(AwtPointerMode.ABSOLUTE, AwtPointerMode.fromId("sideways"))
        assertEquals(AwtPointerMode.ABSOLUTE, AwtPointerMode.fromId(null))
        assertTrue(AwtPointerMode.CAPTURED.isCaptured)
        assertEquals(AwtPointerMode.CAPTURED, AwtPointerMode.ABSOLUTE.toggled())
    }

    // ---- sensitivity -------------------------------------------------------

    @Test
    fun sensitivityIsClampedAndNanSafe() {
        assertEquals(1000, AwtMouseSensitivity.quantise(1f))
        assertEquals(AwtMouseSensitivity.MIN_PERMILLE, AwtMouseSensitivity.quantise(0f))
        assertEquals(AwtMouseSensitivity.MAX_PERMILLE, AwtMouseSensitivity.quantise(1e9f))
        assertEquals(1000, AwtMouseSensitivity.quantise(Float.NaN))
        val hostile = AwtMouseSensitivity(xPermille = 0, yPermille = 99_999).sanitized()
        assertEquals(AwtMouseSensitivity.MIN_PERMILLE, hostile.xPermille)
        assertEquals(AwtMouseSensitivity.MAX_PERMILLE, hostile.yPermille)
    }

    @Test
    fun sensitivityLabelsReadLikeAMultiplier() {
        assertEquals("1×", AwtMouseSensitivity.DEFAULT.label())
        assertEquals("2×", AwtMouseSensitivity.uniform(2f).label())
        assertEquals("0.4×", AwtMouseSensitivity.uniform(0.4f).label())
        assertEquals("1.25×", AwtMouseSensitivity.uniform(1.25f).label())
        assertTrue(AwtMouseSensitivity.uniform(1f).copy(invertY = true).label().contains("反转"))
        assertTrue(AwtMouseSensitivity(1000, 2000).label().contains("/"))
    }

    @Test
    fun sensitivityRoundTripsThroughTheCoresJson() {
        val original = AwtMouseSensitivity.uniform(1.5f).copy(invertY = true)
        val parsed = AwtMouseSensitivity.parse(original.toJson() as JsonValue.Obj)
        assertEquals(original, parsed)
        // The core also spells it as floats; both are accepted.
        val floats = parseJson("""{"x":2.0,"y":0.5,"invert_y":false}""") as JsonValue.Obj
        val fromFloats = AwtMouseSensitivity.parse(floats)
        assertEquals(2000, fromFloats.xPermille)
        assertEquals(500, fromFloats.yPermille)
        // A missing document is the default, never an exception.
        assertEquals(AwtMouseSensitivity.DEFAULT, AwtMouseSensitivity.parse(null))
    }

    @Test
    fun uniformSensitivityEditsBothAxes() {
        val edited = AwtMouseSensitivity(500, 2000).withUniform(1.5f)
        assertEquals(1500, edited.xPermille)
        assertEquals(1500, edited.yPermille)
        assertTrue(edited.isUniform)
    }

    // ---- bindings ----------------------------------------------------------

    @Test
    fun bindingsAreSingleHopSoACycleIsImpossible() {
        val bindings = AwtKeyBindings.EMPTY
            .withKey("key.keyboard.e", "key.keyboard.f")
            .withKey("f", "e")
        assertEquals("f", bindings.resolveKey("e"))
        assertEquals("e", bindings.resolveKey("f"))
        assertEquals("f", bindings.resolveKey("KEY.KEYBOARD.E"))
        assertEquals("q", bindings.resolveKey("q"))
        assertEquals(2, bindings.size)
    }

    @Test
    fun bindingAKeyToItselfClearsIt() {
        val bindings = AwtKeyBindings.EMPTY.withKey("e", "f")
        assertFalse(bindings.isEmpty)
        assertTrue(bindings.withKey("e", "e").isEmpty)
        assertTrue(bindings.withoutKey("key.keyboard.e").isEmpty)
        // A blank name is ignored rather than stored.
        assertTrue(AwtKeyBindings.EMPTY.withKey("", "f").isEmpty)
    }

    @Test
    fun buttonBindingsSwapAndRoundTrip() {
        val bindings = AwtKeyBindings.EMPTY
            .withButton(AwtMouseButton.LEFT, AwtMouseButton.RIGHT)
            .withKey("e", "f")
        assertEquals(AwtMouseButton.RIGHT, bindings.resolveButton(AwtMouseButton.LEFT))
        assertEquals(AwtMouseButton.MIDDLE, bindings.resolveButton(AwtMouseButton.MIDDLE))
        assertEquals(bindings, AwtKeyBindings.parse(bindings.toJson() as JsonValue.Obj))
        assertTrue(
            bindings.withButton(AwtMouseButton.LEFT, AwtMouseButton.LEFT).buttons.isEmpty(),
        )
    }

    @Test
    fun malformedBindingsAreSkippedNotFatal() {
        val json = parseJson(
            """{"keys":{"e":"f","broken":7},"buttons":{"1":3,"9":1,"x":2}}""",
        ) as JsonValue.Obj
        val bindings = AwtKeyBindings.parse(json)
        assertEquals("f", bindings.resolveKey("e"))
        assertEquals(AwtMouseButton.RIGHT, bindings.resolveButton(AwtMouseButton.LEFT))
        assertEquals(1, bindings.keys.size)
        assertEquals(1, bindings.buttons.size)
        assertEquals(AwtKeyBindings.EMPTY, AwtKeyBindings.parse(null))
    }

    @Test
    fun keyNamesAreNormalisedTheSameWayAsTheCore() {
        assertEquals("left.shift", AwtKeyBindings.normalise("  KEY.KEYBOARD.LEFT_SHIFT "))
        assertEquals("left.shift", AwtKeyBindings.normalise("Left Shift"))
        assertEquals("w", AwtKeyBindings.normalise("W"))
    }

    // ---- settings ----------------------------------------------------------

    @Test
    fun hybridModeDecidesWhichDeviceMayMoveThePointer() {
        val absolute = AwtInputSettings.DEFAULT
        assertTrue(absolute.accepts(AwtPointerSource.TOUCH))
        assertTrue(absolute.accepts(AwtPointerSource.MOUSE))
        val strict = absolute.copy(pointerMode = AwtPointerMode.CAPTURED, hybridTouch = false)
        assertFalse("a palm must not turn the view", strict.accepts(AwtPointerSource.TOUCH))
        assertTrue(strict.accepts(AwtPointerSource.MOUSE))
        assertTrue(strict.accepts(AwtPointerSource.STYLUS))
        assertTrue(strict.copy(hybridTouch = true).accepts(AwtPointerSource.TOUCH))
    }

    @Test
    fun settingsRoundTripThroughTheCoresSnapshot() {
        val settings = AwtInputSettings(
            pointerMode = AwtPointerMode.CAPTURED,
            hybridTouch = false,
            nativeInput = false,
            sensitivity = AwtMouseSensitivity.uniform(2.5f).copy(invertY = true),
            scrollPermille = 2_000,
            bindings = AwtKeyBindings.EMPTY.withKey("e", "f"),
        )
        assertEquals(settings, AwtInputSettings.parse(settings.toJson() as JsonValue.Obj))
        // Every key the core reads must be present.
        val json = settings.toJson() as JsonValue.Obj
        for (key in listOf(
            "pointer_mode",
            "hybrid_touch",
            "native_input",
            "sensitivity",
            "scroll_permille",
            "bindings",
        )) {
            assertTrue("settings JSON lost $key", json.entries.containsKey(key))
        }
        assertEquals(AwtInputSettings.DEFAULT, AwtInputSettings.parse(null))
    }

    @Test
    fun settingsAreClampedIntoTheRangeTheCoreAccepts() {
        val clamped = AwtInputSettings.DEFAULT.copy(scrollPermille = 0).sanitized()
        assertEquals(AwtInputSettings.MIN_SCROLL_PERMILLE, clamped.scrollPermille)
        val fast = AwtInputSettings.DEFAULT.copy(scrollPermille = 1_000_000).sanitized()
        assertEquals(AwtInputSettings.MAX_SCROLL_PERMILLE, fast.scrollPermille)
    }

    @Test
    fun theGameInputStateIsParsedForTheDiagnosticsPanel() {
        val json = parseJson(
            "{\"cursor\":{\"x\":-40,\"y\":7},\"grabbed\":true,\"held_keys\":2," +
                "\"held_buttons\":1,\"framebuffer\":{\"width\":854,\"height\":480}}",
        ) as JsonValue.Obj
        val state = AwtGameInputState.parse(json)
        assertEquals(-40, state.cursorX)
        assertTrue(state.grabbed)
        assertEquals(2, state.heldKeys)
        assertEquals(854, state.frameWidth)
        assertTrue(state.describe().contains("已捕获"))
        assertEquals(AwtGameInputState.EMPTY, AwtGameInputState.parse(null))
    }

    // ---- the tracker -------------------------------------------------------

    /** `MotionEvent.BUTTON_*` bits, spelled out so the test needs no Android. */
    private companion object {
        const val PRIMARY = 1
        const val SECONDARY = 2
        const val TERTIARY = 4
    }

    @Test
    fun theTrackerTurnsAButtonStateIntoPressesAndReleases() {
        val tracker = AwtMouseTracker()
        // Move + press, in that order: the press must land at its own position.
        val down = tracker.onPointer(10f, 20f, PRIMARY, AwtPointerSource.MOUSE, pressed = true)
        assertEquals(2, down.size)
        assertEquals(AwtPointerPhase.MOVE, (down[0] as AwtPointerEvent).phase)
        assertEquals(AwtPointerPhase.DOWN, (down[1] as AwtPointerEvent).phase)
        assertEquals(AwtMouseButton.LEFT, (down[1] as AwtPointerEvent).button)
        assertTrue(tracker.isDragging)

        // Same position, primary released and secondary pressed in one state.
        val swap = tracker.onButtonState(SECONDARY)
        val phases = swap.filterIsInstance<AwtPointerEvent>().map { it.phase to it.button }
        assertTrue(
            "expected a right press and a left release, got $phases",
            phases.contains(AwtPointerPhase.DOWN to AwtMouseButton.RIGHT) &&
                phases.contains(AwtPointerPhase.UP to AwtMouseButton.LEFT),
        )
        assertEquals(setOf(AwtMouseButton.RIGHT), tracker.heldButtons)

        val up = tracker.onButtonState(0)
        assertEquals(AwtPointerPhase.UP, (up.single() as AwtPointerEvent).phase)
        assertFalse(tracker.isDragging)
    }

    @Test
    fun theTrackerReportsTheMiddleButton() {
        val tracker = AwtMouseTracker()
        val events = tracker.onPointer(1f, 1f, TERTIARY, AwtPointerSource.MOUSE, pressed = true)
        assertEquals(
            AwtMouseButton.MIDDLE,
            events.filterIsInstance<AwtPointerEvent>().last().button,
        )
    }

    @Test
    fun aTouchActsAsTheConfiguredButton() {
        val tracker = AwtMouseTracker()
        val events = tracker.onPointer(
            5f,
            5f,
            buttonState = 0,
            source = AwtPointerSource.TOUCH,
            pressed = true,
            touchButton = AwtMouseButton.RIGHT,
        )
        val down = events.filterIsInstance<AwtPointerEvent>().last()
        assertEquals(AwtPointerPhase.DOWN, down.phase)
        assertEquals(AwtMouseButton.RIGHT, down.button)
        assertEquals(AwtPointerSource.TOUCH, down.source)
    }

    @Test
    fun theTrackerFiltersAStrayTouchWhileCaptured() {
        val tracker = AwtMouseTracker(
            AwtInputSettings.DEFAULT.copy(
                pointerMode = AwtPointerMode.CAPTURED,
                hybridTouch = false,
            ),
        )
        assertTrue(
            tracker.onPointer(1f, 1f, 0, AwtPointerSource.TOUCH, pressed = true).isEmpty(),
        )
        assertTrue(tracker.onRelative(3f, 0f).isNotEmpty())
        // With the mixed mode on the same tap works again.
        tracker.applySettings(tracker.settings.copy(hybridTouch = true))
        assertTrue(
            tracker.onPointer(1f, 1f, 0, AwtPointerSource.TOUCH, pressed = true).isNotEmpty(),
        )
    }

    @Test
    fun relativeMotionIgnoresNothingAndGarbage() {
        val tracker = AwtMouseTracker()
        assertTrue(tracker.onRelative(0f, 0f).isEmpty())
        assertTrue(tracker.onRelative(Float.NaN, 1f).isEmpty())
        val moved = tracker.onRelative(1.5f, -2f).single() as AwtRelativePointerEvent
        assertEquals(1.5f, moved.dx, 0f)
        assertEquals(-2f, moved.dy, 0f)
        assertEquals(AwtPointerSource.MOUSE, moved.source)
    }

    @Test
    fun capturingReleasesWhatTheOldModeHeld() {
        val tracker = AwtMouseTracker()
        tracker.onPointer(4f, 4f, PRIMARY, AwtPointerSource.MOUSE, pressed = true)
        val events = tracker.setCaptured(true)
        assertEquals(AwtPointerPhase.UP, (events.first() as AwtPointerEvent).phase)
        assertEquals(AwtCaptureEvent(true), events.last())
        assertTrue(tracker.settings.captured)
        // Idempotent: capturing twice is not two grabs.
        assertTrue(tracker.setCaptured(true).isEmpty())
        assertEquals(AwtCaptureEvent(false), tracker.setCaptured(false).last())
    }

    @Test
    fun aCapturedMouseClicksWithoutCoordinates() {
        val tracker = AwtMouseTracker()
        tracker.setCaptured(true)
        // A captured pointer has no position, so the click must be an
        // AwtButtonEvent (which the core resolves at its virtual pointer).
        val down = tracker.onCapturedButtonState(PRIMARY)
        assertEquals(listOf(AwtButtonEvent(AwtMouseButton.LEFT, true)), down)
        // The same state twice is not a second click.
        assertTrue(tracker.onCapturedButtonState(PRIMARY).isEmpty())
        val swap = tracker.onCapturedButtonState(SECONDARY)
        assertTrue(swap.contains(AwtButtonEvent(AwtMouseButton.RIGHT, true)))
        assertTrue(swap.contains(AwtButtonEvent(AwtMouseButton.LEFT, false)))
        assertEquals(
            listOf(AwtButtonEvent(AwtMouseButton.RIGHT, false)),
            tracker.onCapturedButtonState(0),
        )
        // Releasing while captured speaks the same dialect.
        tracker.onCapturedButtonState(TERTIARY)
        assertEquals(
            listOf(AwtButtonEvent(AwtMouseButton.MIDDLE, false)),
            tracker.releaseHeld(),
        )
    }

    @Test
    fun losingTheCompositionReleasesEveryHeldButton() {
        val tracker = AwtMouseTracker()
        tracker.onPointer(4f, 4f, PRIMARY or SECONDARY, AwtPointerSource.MOUSE, pressed = true)
        assertEquals(2, tracker.heldButtons.size)
        val released = tracker.releaseHeld()
        assertEquals(2, released.size)
        assertTrue(released.all { (it as AwtPointerEvent).phase == AwtPointerPhase.UP })
        assertTrue(tracker.releaseHeld().isEmpty())
        tracker.reset()
        assertFalse(tracker.isDragging)
    }

    @Test
    fun aModeChangeDropsTheGestureInFlight() {
        val tracker = AwtMouseTracker()
        tracker.onPointer(4f, 4f, PRIMARY, AwtPointerSource.MOUSE, pressed = true)
        val events = tracker.applySettings(
            tracker.settings.copy(pointerMode = AwtPointerMode.CAPTURED),
        )
        assertEquals(1, events.size)
        assertEquals(AwtPointerPhase.UP, (events.single() as AwtPointerEvent).phase)
        assertTrue(tracker.heldButtons.isEmpty())
    }
}
