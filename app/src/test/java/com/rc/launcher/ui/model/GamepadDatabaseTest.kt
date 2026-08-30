package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the gamepad mapping database + custom remap (task 4). */
class GamepadDatabaseTest {

    @Test
    fun genericIsFirstAndDefault() {
        assertEquals(ControllerProfile.GENERIC_ID, GamepadDatabase.all.first().id)
        assertEquals(GamepadDatabase.default.id, ControllerProfile.GENERIC_ID)
    }

    @Test
    fun identifyExactMatch() {
        val p = GamepadDatabase.identify(0x054C, 0x09CC)
        assertEquals("playstation", p.id)
        assertEquals(0x09CC, p.productId)
    }

    @Test
    fun identifyVendorFallback() {
        // An unknown Sony PID still resolves to the PlayStation profile.
        val p = GamepadDatabase.identify(0x054C, 0x0CE6)
        assertEquals("playstation", p.id)
    }

    @Test
    fun identifyUnknownFallsBackToGeneric() {
        val p = GamepadDatabase.identify(0xDEAD, 0xBEEF)
        assertEquals(ControllerProfile.GENERIC_ID, p.id)
    }

    @Test
    fun byIdUnknownReturnsGeneric() {
        assertEquals(ControllerProfile.GENERIC_ID, GamepadDatabase.byId("nope").id)
        assertEquals("xbox", GamepadDatabase.byId("xbox").id)
    }

    @Test
    fun allProfilesCoverBuiltins() {
        val ids = GamepadDatabase.all.map { it.id }.toSet()
        assertTrue(ids.containsAll(setOf("generic", "xbox", "playstation", "8bitdo", "flydigi")))
    }

    @Test
    fun flydigiIsNotGeneric() {
        assertFalse(GamepadDatabase.FLYDIGI.isGeneric)
    }

    // --- Custom remapping ----------------------------------------------------

    @Test
    fun remapOverrideTakesPrecedence() {
        val remap = ControllerRemap(
            buttons = mapOf(96 to StandardButton.WEST), // force physical A -> West
        )
        assertEquals(StandardButton.WEST, remap.resolveButton(96, StandardButton.SOUTH))
        // Untouched buttons fall back to the profile mapping.
        assertEquals(StandardButton.EAST, remap.resolveButton(97, StandardButton.EAST))
    }

    @Test
    fun remapAxisOverride() {
        val remap = ControllerRemap(
            axes = mapOf(11 to StandardAxis.RIGHT_X), // override axis 11
        )
        assertEquals(StandardAxis.RIGHT_X, remap.resolveAxis(11, StandardAxis.LEFT_X))
        assertNull(remap.resolveAxis(999, null))
    }

    @Test
    fun remapEmptyIsEmpty() {
        assertTrue(ControllerRemap().isEmpty())
        assertFalse(
            ControllerRemap(buttons = mapOf(1 to StandardButton.SOUTH)).isEmpty(),
        )
    }

    @Test
    fun remapRoundTripsThroughJson() {
        val remap = ControllerRemap(
            buttons = mapOf(96 to StandardButton.WEST, 97 to StandardButton.NORTH),
            axes = mapOf(11 to StandardAxis.RIGHT_X),
        )
        val json = remap.toJsonString()
        val parsed = ControllerRemap.fromJson(json)
        assertEquals(remap, parsed)
    }

    @Test
    fun remapFromMalformedJsonIsNull() {
        assertNull(ControllerRemap.fromJson("not json"))
        // Blank / null yields an empty (safe) remap, not null.
        assertEquals(ControllerRemap(), ControllerRemap.fromJson(""))
        assertEquals(ControllerRemap(), ControllerRemap.fromJson(null))
    }

    @Test
    fun standardButtonAndAxisCodesResolve() {
        assertEquals(StandardButton.SOUTH, StandardButton.fromCode("south"))
        assertEquals(StandardAxis.LEFT_X, StandardAxis.fromCode("left_x"))
        assertNull(StandardButton.fromCode("nope"))
    }
}
