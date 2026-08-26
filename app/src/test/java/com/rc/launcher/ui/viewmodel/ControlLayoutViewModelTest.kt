package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.ControlLayout
import com.rc.launcher.ui.model.ControlLayoutCatalog
import com.rc.launcher.ui.model.InMemoryControlLayoutRepository
import com.rc.launcher.ui.model.InMemorySettingsRepository
import com.rc.launcher.ui.model.JoystickKind
import com.rc.launcher.ui.model.IssueSeverity
import com.rc.launcher.ui.model.GamepadAxis
import com.rc.launcher.ui.model.MappedKey
import com.rc.launcher.ui.model.VirtualButton
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the controller / input-mapping editor view model (task 15). */
class ControlLayoutViewModelTest {

    private fun vm(): ControlLayoutViewModel =
        ControlLayoutViewModel(InMemoryControlLayoutRepository(), InMemorySettingsRepository())

    @Test
    fun initial_loadsDefaultBuiltIn() {
        val v = vm()
        assertEquals(ControlLayout.DEFAULT_ID, v.layout.value.id)
        assertEquals(false, v.layout.value.editable)
        assertEquals(false, v.dirty.value)
    }

    @Test
    fun loadLayout_builtInSyncsSettings() {
        val settings = InMemorySettingsRepository()
        val v = ControlLayoutViewModel(InMemoryControlLayoutRepository(), settings)
        v.loadLayout(ControlLayout.GAMEPAD_ID)
        assertEquals(ControlLayout.GAMEPAD_ID, v.layout.value.id)
        assertEquals(false, v.layout.value.editable)
        assertEquals(false, v.dirty.value)
        assertEquals(ControlLayout.GAMEPAD_ID, settings.load().controllerLayoutId)
    }

    @Test
    fun addButton_marksDirtyAndSelects() {
        val v = vm()
        val before = v.layout.value.elements.size
        v.addButton(0.2f, 0.3f)
        assertEquals(before + 1, v.layout.value.elements.size)
        assertTrue(v.dirty.value)
        assertNotNull(v.selectedElementId.value)
    }

    @Test
    fun moveElement_clampsOutOfRange() {
        val v = vm()
        v.addButton(0.5f, 0.5f)
        val id = v.selectedElementId.value!!
        v.moveElement(id, 5f, -2f)
        val moved = v.layout.value.elements.first { it.id == id }
        assertEquals(1f, moved.x)
        assertEquals(0f, moved.y)
        // In-range move is honoured exactly.
        v.moveElement(id, 0.25f, 0.35f)
        val moved2 = v.layout.value.elements.first { it.id == id }
        assertEquals(0.25f, moved2.x)
        assertEquals(0.35f, moved2.y)
    }

    @Test
    fun updateButton_persistsKeysLabelAndSize() {
        val v = vm()
        val btnId = v.layout.value.elements.filterIsInstance<VirtualButton>().first().id
        v.updateButton(btnId, "ABC", listOf(MappedKey.KEY_A, MappedKey.KEY_B), 0.3f)
        val b = v.layout.value.elements.first { it.id == btnId } as VirtualButton
        assertEquals("ABC", b.label)
        assertEquals(listOf(MappedKey.KEY_A, MappedKey.KEY_B), b.keys)
        assertEquals(0.3f, b.size)
    }

    @Test
    fun saveCurrent_builtInCreatesCustomLayout() {
        val settings = InMemorySettingsRepository()
        val repo = InMemoryControlLayoutRepository()
        val v = ControlLayoutViewModel(repo, settings)
        v.loadLayout(ControlLayout.GAMEPAD_ID)
        v.saveCurrent("我的布局")
        assertTrue(v.layout.value.editable)
        assertTrue(v.layout.value.id.startsWith("custom_"))
        assertEquals(1, repo.list().size)
        assertEquals(v.layout.value.id, settings.load().controllerLayoutId)
    }

    @Test
    fun saveCurrent_editableOverwritesSameId() {
        val repo = InMemoryControlLayoutRepository()
        val v = ControlLayoutViewModel(repo, InMemorySettingsRepository())
        v.saveCurrent("布局A") // from default built-in -> custom
        val id = v.layout.value.id
        val name = "布局A改名"
        v.saveCurrent(name) // editable -> overwrite same id
        assertEquals(id, v.layout.value.id)
        assertEquals(1, repo.list().size)
        assertEquals(name, repo.load(id)?.name)
    }

    @Test
    fun persistedLayout_survivesNewViewModelInstance() {
        val settings = InMemorySettingsRepository()
        val repo = InMemoryControlLayoutRepository()
        val v = ControlLayoutViewModel(repo, settings)
        v.saveCurrent("共享布局")
        val customId = v.layout.value.id

        val v2 = ControlLayoutViewModel(repo, settings)
        // Settings still point at the custom layout, so a fresh VM resolves it.
        assertEquals(customId, v2.layout.value.id)
    }

    @Test
    fun deleteCurrent_removesCustomAndResets() {
        val settings = InMemorySettingsRepository()
        val repo = InMemoryControlLayoutRepository()
        val v = ControlLayoutViewModel(repo, settings)
        v.saveCurrent("待删除")
        assertEquals(1, repo.list().size)
        v.deleteCurrent()
        assertEquals(0, repo.list().size)
        assertEquals(ControlLayout.DEFAULT_ID, v.layout.value.id)
    }

    @Test
    fun deleteCurrent_onBuiltInIsNoOp() {
        val v = vm()
        v.loadLayout(ControlLayout.WASD_ID)
        v.deleteCurrent()
        assertEquals(ControlLayout.WASD_ID, v.layout.value.id)
    }

    @Test
    fun resetCurrent_discardsEdits() {
        val v = vm()
        v.loadLayout(ControlLayout.DEFAULT_ID)
        val n0 = v.layout.value.elements.size
        v.addButton(0.5f, 0.5f)
        assertEquals(n0 + 1, v.layout.value.elements.size)
        v.resetCurrent()
        assertEquals(n0, v.layout.value.elements.size)
        assertEquals(false, v.dirty.value)
    }

    @Test
    fun saveCurrent_asCopyAlwaysCreatesNewId() {
        val repo = InMemoryControlLayoutRepository()
        val v = ControlLayoutViewModel(repo, InMemorySettingsRepository())
        v.saveCurrent("布局A") // custom
        val id1 = v.layout.value.id
        v.saveCurrent("布局A副本", asCopy = true) // save as -> new id
        val id2 = v.layout.value.id
        assertEquals(2, repo.list().size)
        assertEquals(true, id1 != id2)
    }

    @Test
    fun builtInLayoutsExposedForPicker() {
        val v = vm()
        assertEquals(ControlLayoutCatalog.allMetas(), v.builtInLayouts)
    }
    @Test
    fun issues_exposedAndRefreshedOnEdit() {
        val v = vm()
        // The issues stream is refreshed whenever the layout is edited.
        val before = v.issues.value.size
        // Two closely placed buttons should surface an overlap warning.
        v.addButton(0.5f, 0.5f)
        v.addButton(0.52f, 0.52f)
        assertTrue(v.issues.value.size > before)
        assertTrue(v.issues.value.any { it.severity == IssueSeverity.WARNING })
    }

    @Test
    fun summary_reflectsEditedLayout() {
        val v = vm()
        val s = v.summary()
        assertEquals(2, s.joystickCount)
        assertEquals(9, s.buttonCount)
    }

    @Test
    fun duplicateCurrent_createsNewCustomLayout() {
        val repo = InMemoryControlLayoutRepository()
        val settings = InMemorySettingsRepository()
        val v = ControlLayoutViewModel(repo, settings)
        v.loadLayout(ControlLayout.GAMEPAD_ID)
        v.duplicateCurrent("手柄副本")
        assertTrue(v.layout.value.editable)
        assertTrue(v.layout.value.id.startsWith("custom_"))
        assertEquals(1, repo.list().size)
    }

    @Test
    fun updateJoystick_bindsGamepadAxes() {
        val v = vm()
        v.addJoystick(0.5f, 0.5f)
        val jsId = v.layout.value.elements.filterIsInstance<VirtualJoystick>().first().id
        v.updateJoystick(jsId, 0.2f, JoystickKind.LOOK, GamepadAxis.LEFT_X, GamepadAxis.LEFT_Y)
        val js = v.layout.value.elements.first { it.id == jsId } as VirtualJoystick
        assertEquals(GamepadAxis.LEFT_X, js.axisX)
        assertEquals(GamepadAxis.LEFT_Y, js.axisY)
        assertEquals(JoystickKind.LOOK, js.kind)
    }

    @Test
    fun updateJoystick_withoutAxesKeepsExistingBinding() {
        val v = vm()
        v.addJoystick(0.5f, 0.5f)
        val jsId = v.layout.value.elements.filterIsInstance<VirtualJoystick>().first().id
        v.updateJoystick(jsId, 0.2f, JoystickKind.LOOK, GamepadAxis.RIGHT_X, GamepadAxis.RIGHT_Y)
        // Adjusting radius alone must NOT wipe the axis binding.
        v.updateJoystick(jsId, 0.25f, JoystickKind.LOOK)
        val js = v.layout.value.elements.first { it.id == jsId } as VirtualJoystick
        assertEquals(GamepadAxis.RIGHT_X, js.axisX)
        assertEquals(GamepadAxis.RIGHT_Y, js.axisY)
    }

}
