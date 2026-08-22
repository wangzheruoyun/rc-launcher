package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the control-layout repository contract (task 15). */
class ControlLayoutRepositoryTest {

    @Test
    fun inMemory_saveLoadListDelete() {
        val repo = InMemoryControlLayoutRepository()
        assertEquals(0, repo.list().size)
        assertNull(repo.load("missing"))

        val layout = ControlLayout(
            id = "custom1",
            name = "我的布局",
            editable = true,
            elements = listOf(VirtualButton("b1", 0.5f, 0.5f, listOf(MappedKey.KEY_W))),
        )
        repo.save(layout)
        assertEquals(1, repo.list().size)
        assertEquals("我的布局", repo.list().first().name)
        assertEquals(true, repo.list().first().builtIn)

        val loaded = repo.load("custom1")
        assertEquals("custom1", loaded?.id)
        assertEquals(1, loaded?.elements?.size)

        // Re-save (upsert) keeps a single entry.
        repo.save(layout.copy(name = "改名"))
        assertEquals(1, repo.list().size)
        assertEquals("改名", repo.load("custom1")?.name)

        assertTrue(repo.delete("custom1"))
        assertEquals(0, repo.list().size)
        assertNull(repo.load("custom1"))
        assertEquals(false, repo.delete("custom1"))
    }

    @Test
    fun inMemory_saveSanitizesClampedElements() {
        val repo = InMemoryControlLayoutRepository()
        val dirty = ControlLayout(
            "c",
            "C",
            elements = listOf(VirtualButton("b", 9f, -9f, size = 99f)),
        )
        repo.save(dirty)
        val loaded = repo.load("c")!!
        val btn = loaded.elements.first() as VirtualButton
        assertEquals(1f, btn.x)
        assertEquals(0f, btn.y)
        assertEquals(VirtualButton.MAX_SIZE, btn.size)
    }
}
