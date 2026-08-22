package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the dashboard domain model (task 12 / task 21). */
class GameInstanceTest {

    private fun sample(): List<GameInstance> {
        val t = 1_000_000L
        return listOf(
            GameInstance("a", "A", "1.20.1", ModLoader.VANILLA, lastPlayed = t - 10),
            GameInstance("b", "B", "1.20.1", ModLoader.FABRIC, lastPlayed = 0L),
            GameInstance("c", "C", "1.12.2", ModLoader.FORGE, lastPlayed = t - 5),
            GameInstance("d", "D", "1.20.4", ModLoader.QUILT, lastPlayed = t - 8, isFavorite = true),
        )
    }

    @Test
    fun recentlyPlayed_filtersUnplayedAndOrdersDesc() {
        val recent = sample().recentlyPlayed()
        assertEquals(listOf("d", "a", "c"), recent.map { it.id })
        assertTrue(recent.none { it.lastPlayed == 0L })
    }

    @Test
    fun recentlyPlayed_respectsLimit() {
        assertEquals(2, sample().recentlyPlayed(limit = 2).size)
    }

    @Test
    fun dashboardOrder_putsFavoritesFirst() {
        val ordered = sample().dashboardOrder()
        assertEquals("d", ordered.first().id) // favorite wins over recency
    }

    @Test
    fun modLoader_fromName_isCaseInsensitiveAndFallsBackToVanilla() {
        assertEquals(ModLoader.FABRIC, ModLoader.fromName("fabric"))
        assertEquals(ModLoader.FORGE, ModLoader.fromName("FORGE"))
        assertEquals(ModLoader.VANILLA, ModLoader.fromName(null))
        assertEquals(ModLoader.VANILLA, ModLoader.fromName("bogus"))
    }

    @Test
    fun lastPlayedLabel_handlesNeverAndRecent() {
        val now = 10_000_000L
        assertEquals("从未游玩", GameInstance("x", "X", "1.0").copy(lastPlayed = 0L).lastPlayedLabel(now))
        assertEquals("刚刚", GameInstance("x", "X", "1.0").copy(lastPlayed = now - 30_000L).lastPlayedLabel(now))
        assertTrue(
            GameInstance("x", "X", "1.0").copy(lastPlayed = now - 3 * 60 * 60_000L)
                .lastPlayedLabel(now).endsWith("小时前"),
        )
        assertFalse(GameInstance("x", "X", "1.0").copy(lastPlayed = now - 30_000L).lastPlayedLabel(now).contains("天"))
    }
}
