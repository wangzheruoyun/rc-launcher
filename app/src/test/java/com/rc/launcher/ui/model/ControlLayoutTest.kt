package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import com.rc.launcher.ui.model.LayoutSummary
import com.rc.launcher.ui.model.IssueSeverity
import com.rc.launcher.ui.model.GamepadAxis

/** Unit tests for the controller / input-mapping model (task 15). */
class ControlLayoutTest {

    @Test
    fun builtIns_areValidAndOnScreen() {
        for (layout in ControlLayoutCatalog.all()) {
            assertTrue(layout.id.isNotBlank())
            assertTrue(layout.name.isNotBlank())
            assertFalse(layout.editable)
            for (el in layout.elements) {
                assertTrue(el.id.isNotBlank())
                assertTrue(el.x in 0f..1f)
                assertTrue(el.y in 0f..1f)
                when (el) {
                    is VirtualButton -> assertTrue(el.size in VirtualButton.MIN_SIZE..VirtualButton.MAX_SIZE)
                    is VirtualJoystick -> assertTrue(el.radius in VirtualJoystick.MIN_RADIUS..VirtualJoystick.MAX_RADIUS)
                }
            }
        }
    }

    @Test
    fun builtInIdsAreDistinct() {
        val ids = ControlLayoutCatalog.all().map { it.id }
        assertEquals(ids.toSet().size, ids.size)
    }

    @Test
    fun withElement_addsAndReplacesById() {
        val base = ControlLayoutCatalog.default()
        val extra = VirtualButton("extra", 0.5f, 0.5f, listOf(MappedKey.KEY_E))
        val added = base.withElement(extra)
        assertEquals(base.elements.size + 1, added.elements.size)
        // Same id replaces rather than duplicates.
        val replaced = added.withElement(extra.copy(label = "X"))
        assertEquals(added.elements.size, replaced.elements.size)
        assertEquals("X", (replaced.elements.first { it.id == "extra" } as VirtualButton).label)
    }

    @Test
    fun withoutElement_removesById() {
        val base = ControlLayoutCatalog.default()
        val first = base.elements.first().id
        val removed = base.withoutElement(first)
        assertEquals(base.elements.size - 1, removed.elements.size)
        assertFalse(removed.hasElement(first))
    }

    @Test
    fun sanitized_clampsOffScreenAndOversized() {
        val bad = VirtualButton("b", 2f, -1f, listOf(MappedKey.KEY_W), size = 99f)
        val good = bad.normalized()
        assertEquals(1f, good.x)
        assertEquals(0f, good.y)
        assertEquals(VirtualButton.MAX_SIZE, good.size)
    }

    @Test
    fun sanitized_clampsJoystickRadius() {
        val bad = VirtualJoystick("j", -0.5f, 2f, radius = 5f)
        val good = bad.normalized()
        assertEquals(0f, good.x)
        assertEquals(1f, good.y)
        assertEquals(VirtualJoystick.MAX_RADIUS, good.radius)
    }

    @Test
    fun mappedKey_fromCode_roundTrips() {
        assertEquals(MappedKey.KEY_W, MappedKey.fromCode("key.keyboard.w"))
        assertEquals(MappedKey.BTN_A, MappedKey.fromCode("button.a"))
        assertNull(MappedKey.fromCode("does.not.exist"))
    }

    @Test
    fun joystickKind_fromName() {
        assertEquals(JoystickKind.LOOK, JoystickKind.fromName("LOOK"))
        assertEquals(JoystickKind.MOVE, JoystickKind.fromName(null))
        assertEquals(JoystickKind.MOVE, JoystickKind.fromName("bogus"))
    }

    @Test
    fun json_roundTrip_preservesLayout() {
        val layout = ControlLayoutCatalog.gamepad()
        val text = layout.toJsonString()
        val back = parseControlLayout(text)
        assertEquals(layout.id, back?.id)
        assertEquals(layout.name, back?.name)
        assertEquals(layout.elements.size, back?.elements?.size)
        // A sample button + joystick survive with their bindings.
        val gpA = back?.elements?.firstOrNull { it.id == "gp_a" } as? VirtualButton
        assertEquals(listOf(MappedKey.BTN_A), gpA?.keys)
        val move = back?.elements?.firstOrNull { it.id == "gp_move" } as? VirtualJoystick
        assertEquals(JoystickKind.MOVE, move?.kind)
    }

    @Test
    fun parseControlLayout_rejectsBlankIdOrName() {
        assertNull(parseControlLayout("{\"id\":\"\",\"name\":\"Y\"}"))
        assertNull(parseControlLayout("{\"id\":\"x\",\"name\":\"\"}"))
    }

    @Test
    fun parseControlLayout_handlesMalformed() {
        assertNull(parseControlLayout("not json"))
        assertNull(parseControlLayout("{\"id\":\"x\"}")) // missing name
        assertNull(parseControlLayout(""))
    }

    @Test
    fun parseControlLayout_skipsUnknownKeysButKeepsKnown() {
        val json = """{"id":"x","name":"Y","e":[{"t":"b","id":"b1","x":0.1,"y":0.2,"k":["key.keyboard.w","unknown.code"],"l":"W"}]}"""
        val parsed = parseControlLayout(json)
        assertEquals("x", parsed?.id)
        val btn = parsed?.elements?.firstOrNull() as? VirtualButton
        // The unknown code is dropped, the known one kept.
        assertEquals(listOf(MappedKey.KEY_W), btn?.keys)
        assertEquals("W", btn?.label)
    }

    @Test
    fun colorArgb_roundTripsThroughJson() {
        val btn = VirtualButton("c", 0.5f, 0.5f, colorArgb = 0xFF000000.toInt())
        val layout = ControlLayout("cl", "CL", listOf(btn))
        val back = parseControlLayout(layout.toJsonString())
        val parsed = back?.elements?.firstOrNull() as? VirtualButton
        assertEquals(0xFF000000.toInt(), parsed?.colorArgb)
    }
    @Test
    fun mappedKey_extendedVocabularyRoundTrips() {
        assertEquals(MappedKey.KEY_UP, MappedKey.fromCode("key.keyboard.up"))
        assertEquals(MappedKey.KEY_DOWN, MappedKey.fromCode("key.keyboard.down"))
        assertEquals(MappedKey.KEY_F1, MappedKey.fromCode("key.keyboard.f1"))
        assertEquals(MappedKey.KEY_9, MappedKey.fromCode("key.keyboard.9"))
        // All entries resolve back to themselves.
        for (k in MappedKey.ALL) assertEquals(k, MappedKey.fromCode(k.code))
    }

    @Test
    fun gamepadAxis_fromNameRoundTrips() {
        assertEquals(GamepadAxis.LEFT_X, GamepadAxis.fromName("LEFT_X"))
        assertEquals(GamepadAxis.TRIGGER_RIGHT, GamepadAxis.fromName("TRIGGER_RIGHT"))
        assertNull(GamepadAxis.fromName("nope"))
    }

    @Test
    fun validate_flagsDuplicateIdsAsError() {
        val dup = ControlLayout(
            "d", "D",
            listOf(VirtualButton("a", 0.5f, 0.5f), VirtualButton("a", 0.6f, 0.6f)),
        )
        val issues = dup.validate()
        assertTrue(issues.any { it.severity == IssueSeverity.ERROR && it.elementId == "a" })
    }

    @Test
    fun validate_warnsOnEmptyLayoutAndKeylessButton() {
        val empty = ControlLayout("e", "E", emptyList()).validate()
        assertTrue(empty.any { it.severity == IssueSeverity.WARNING && it.elementId == null })

        val keyless = ControlLayout(
            "n", "N", listOf(VirtualButton("b", 0.5f, 0.5f, emptyList())),
        ).validate()
        assertTrue(keyless.any { it.elementId == "b" && it.message.contains("按键") })
    }

    @Test
    fun validate_warnsOnOverlappingButtons() {
        val overlap = ControlLayout(
            "o", "O",
            listOf(
                VirtualButton("b1", 0.50f, 0.50f, size = 0.2f),
                VirtualButton("b2", 0.55f, 0.55f, size = 0.2f),
            ),
        ).validate()
        assertTrue(overlap.any { it.severity == IssueSeverity.WARNING })
    }

    @Test
    fun duplicate_clonesAsEditableWithNewId() {
        val src = ControlLayoutCatalog.default()
        val copy = src.duplicate("my_copy", "我的副本")
        assertEquals("my_copy", copy.id)
        assertEquals("我的副本", copy.name)
        assertTrue(copy.editable)
        assertEquals(src.elements.size, copy.elements.size)
    }

    @Test
    fun elementAt_hitsButtonsAndJoysticks() {
        val layout = ControlLayout(
            "h", "H",
            listOf(
                VirtualButton("b", 0.5f, 0.5f, size = 0.2f),
                VirtualJoystick("j", 0.2f, 0.2f, radius = 0.18f),
            ),
        )
        assertEquals("b", (layout.elementAt(0.5f, 0.5f) as? VirtualButton)?.id)
        assertEquals("j", (layout.elementAt(0.25f, 0.25f) as? VirtualJoystick)?.id)
        assertNull(layout.elementAt(0.95f, 0.95f))
    }

    @Test
    fun summary_reportsCountsAndKeys() {
        val s = ControlLayoutCatalog.default().summary()
        assertEquals(9, s.buttonCount)
        assertEquals(2, s.joystickCount)
        assertEquals(1, s.moveStickCount)
        assertEquals(1, s.lookStickCount)
        assertTrue(s.keysCovered.contains(MappedKey.KEY_SPACE))
    }

    @Test
    fun json_roundTrip_preservesJoystickAxesAndVersion() {
        val js = VirtualJoystick(
            "j1", 0.2f, 0.2f, 0.18f, JoystickKind.MOVE,
            GamepadAxis.LEFT_X, GamepadAxis.LEFT_Y,
        )
        val layout = ControlLayout("x", "Y", listOf(js))
        val text = layout.toJsonString()
        assertTrue(text.contains("\"v\":1"))
        val back = parseControlLayout(text)
        val parsed = back?.elements?.firstOrNull() as? VirtualJoystick
        assertEquals(GamepadAxis.LEFT_X, parsed?.axisX)
        assertEquals(GamepadAxis.LEFT_Y, parsed?.axisY)
    }

    @Test
    fun json_roundTrip_withoutAxesStaysNullable() {
        val js = VirtualJoystick("j2", 0.5f, 0.5f, 0.18f, JoystickKind.LOOK)
        val back = parseControlLayout(ControlLayout("x", "Y", listOf(js)).toJsonString())
        val parsed = back?.elements?.firstOrNull() as? VirtualJoystick
        assertNull(parsed?.axisX)
        assertNull(parsed?.axisY)
        assertEquals(JoystickKind.LOOK, parsed?.kind)
    }

}
