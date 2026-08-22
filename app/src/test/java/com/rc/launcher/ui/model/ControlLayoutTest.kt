package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

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
}
