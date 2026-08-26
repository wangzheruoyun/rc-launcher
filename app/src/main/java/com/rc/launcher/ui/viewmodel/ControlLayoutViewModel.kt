package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.ControlElement
import com.rc.launcher.ui.model.ControlLayout
import com.rc.launcher.ui.model.ControlLayoutCatalog
import com.rc.launcher.ui.model.ControlLayoutMeta
import com.rc.launcher.ui.model.ControlLayoutRepositories
import com.rc.launcher.ui.model.ControlLayoutRepository
import com.rc.launcher.ui.model.ControlLayout.Companion.DEFAULT_ID
import com.rc.launcher.ui.model.GamepadAxis
import com.rc.launcher.ui.model.JoystickKind
import com.rc.launcher.ui.model.LayoutIssue
import com.rc.launcher.ui.model.LayoutSummary
import com.rc.launcher.ui.model.MappedKey
import com.rc.launcher.ui.model.SettingsRepositories
import com.rc.launcher.ui.model.SettingsRepository
import com.rc.launcher.ui.model.VirtualButton
import com.rc.launcher.ui.model.VirtualJoystick
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * State container for the controller / input-mapping editor (task 15).
 *
 * It owns the [ControlLayout] currently being edited, the list of saved custom
 * layouts and the id of the element selected in the editor. Every mutator is
 * copy-on-write, runs the layout through [ControlLayout.sanitized] (so out of
 * range / off-screen input can never reach the Rust core or the renderer) and
 * keeps a [dirty] flag so the UI can warn before losing unsaved edits.
 *
 * The repository is injected with a default so `viewModel()` can instantiate it
 * and tests can pass a custom [com.rc.launcher.ui.model.InMemoryControlLayoutRepository]
 * (mirrors [SettingsViewModel]). Applying / saving a layout also keeps
 * [com.rc.launcher.ui.model.LauncherSettings.controllerLayoutId] in sync so the
 * launch engine always knows which mapping to use (task 7).
 */
class ControlLayoutViewModel(
    private val repository: ControlLayoutRepository = ControlLayoutRepositories.default,
    private val settingsRepository: SettingsRepository = SettingsRepositories.default,
) : ViewModel() {

    private val _layout = MutableStateFlow(initialLayout())
    val layout: StateFlow<ControlLayout> = _layout.asStateFlow()

    private val _savedLayouts = MutableStateFlow(repository.list())
    val savedLayouts: StateFlow<List<ControlLayoutMeta>> = _savedLayouts.asStateFlow()

    private val _selectedElementId = MutableStateFlow<String?>(null)
    val selectedElementId: StateFlow<String?> = _selectedElementId.asStateFlow()

    private val _dirty = MutableStateFlow(false)
    val dirty: StateFlow<Boolean> = _dirty.asStateFlow()

    /** Validation problems for the layout currently being edited. */
    private val _issues = MutableStateFlow(emptyList<LayoutIssue>())
    val issues: StateFlow<List<LayoutIssue>> = _issues.asStateFlow()

    /** Built-in layouts shipped with the app (not persisted, not deletable). */
    val builtInLayouts: List<ControlLayoutMeta> = ControlLayoutCatalog.allMetas()

    private fun initialLayout(): ControlLayout {
        val activeId = runCatching { settingsRepository.load().controllerLayoutId }.getOrNull()
        return ControlLayoutCatalog.builtInById(activeId ?: DEFAULT_ID)
            ?: repository.load(activeId ?: DEFAULT_ID)
            ?: ControlLayoutCatalog.defaultLayout()
    }

    /** The element currently selected in the editor, or null. */
    fun selectedElement(): ControlElement? =
        _layout.value.elements.firstOrNull { it.id == _selectedElementId.value }

    // ---- Layout selection ---------------------------------------------------

    /** Load a built-in or saved layout by id and mark it active. */
    fun loadLayout(id: String) {
        val next = ControlLayoutCatalog.builtInById(id)
            ?: repository.load(id)
            ?: ControlLayoutCatalog.defaultLayout()
        _layout.value = next.sanitized()
        _selectedElementId.value = null
        _dirty.value = false
        _issues.value = _layout.value.validate()
        applyActiveLayoutId(next.id)
    }

    /** Mark the current layout as the active one without persisting it. */
    fun applyCurrent() = applyActiveLayoutId(_layout.value.id)

    // ---- Element editing ----------------------------------------------------

    fun selectElement(id: String?) {
        _selectedElementId.value = id
    }

    /** Move an existing element to a normalized [x], [y] (clamped on-screen). */
    fun moveElement(id: String, x: Float, y: Float) {
        updateElementById(id) { el ->
            when (el) {
                is VirtualButton -> el.copy(x = x.coerceIn(0f, 1f), y = y.coerceIn(0f, 1f))
                is VirtualJoystick -> el.copy(x = x.coerceIn(0f, 1f), y = y.coerceIn(0f, 1f))
            else -> el
            }
        }
    }

    /** Add a new virtual button at a normalized position and select it. */
    fun addButton(x: Float, y: Float) {
        val btn = VirtualButton(
            id = uniqueElementId("btn"),
            x = x.coerceIn(0f, 1f),
            y = y.coerceIn(0f, 1f),
            keys = listOf(MappedKey.KEY_SPACE),
            label = "按键",
        ).normalized()
        _layout.value = _layout.value.withElement(btn)
        _selectedElementId.value = btn.id
        _dirty.value = true
        _issues.value = _layout.value.validate()
    }

    /** Add a new touch joystick at a normalized position and select it. */
    fun addJoystick(x: Float, y: Float) {
        val js = VirtualJoystick(
            id = uniqueElementId("js"),
            x = x.coerceIn(0f, 1f),
            y = y.coerceIn(0f, 1f),
            kind = JoystickKind.MOVE,
        ).normalized()
        _layout.value = _layout.value.withElement(js)
        _selectedElementId.value = js.id
        _dirty.value = true
        _issues.value = _layout.value.validate()
    }

    fun updateButton(id: String, label: String, keys: List<MappedKey>, size: Float) {
        updateElementById(id) { el ->
            if (el is VirtualButton) {
                el.copy(
                    label = label,
                    keys = keys.distinct(),
                    size = size.coerceIn(VirtualButton.MIN_SIZE, VirtualButton.MAX_SIZE),
                ).normalized()
            } else {
                el
            }
        }
    }

    /**
     * Update a joystick's radius and drive axis, preserving its current
     * gamepad-axis bindings (convenience overload used when only the geometry or
     * the drive axis changes).
     */
    fun updateJoystick(id: String, radius: Float, kind: JoystickKind) {
        val cur = _layout.value.elements.filterIsInstance<VirtualJoystick>()
            .firstOrNull { it.id == id }
        updateJoystick(id, radius, kind, cur?.axisX, cur?.axisY)
    }

    /** Update a joystick, explicitly setting its gamepad-axis bindings. */
    fun updateJoystick(
        id: String,
        radius: Float,
        kind: JoystickKind,
        axisX: GamepadAxis?,
        axisY: GamepadAxis?,
    ) {
        updateElementById(id) { el ->
            if (el is VirtualJoystick) {
                el.copy(
                    radius = radius.coerceIn(VirtualJoystick.MIN_RADIUS, VirtualJoystick.MAX_RADIUS),
                    kind = kind,
                    axisX = axisX,
                    axisY = axisY,
                ).normalized()
            } else {
                el
            }
        }
    }

    fun removeElement(id: String) {
        _layout.value = _layout.value.withoutElement(id)
        if (_selectedElementId.value == id) _selectedElementId.value = null
        _dirty.value = true
        _issues.value = _layout.value.validate()
    }

    // ---- Persistence --------------------------------------------------------

    /**
     * Save the current layout. A built-in is always saved as a new custom id.
     * Pass [asCopy] = true (the "save as" action) to always create a new id even
     * when editing an existing custom layout.
     */
    fun saveCurrent(name: String? = null, asCopy: Boolean = false) {
        val base = _layout.value
        val name2 = name?.takeIf { it.isNotBlank() } ?: base.name
        val keepId = base.editable && !asCopy
        val targetId = if (keepId) base.id else uniqueId(name2)
        val toSave = base.copy(
            id = targetId,
            name = name2,
            editable = true,
            createdAt = if (keepId) base.createdAt else System.currentTimeMillis(),
        ).sanitized()
        repository.save(toSave)
        _savedLayouts.value = repository.list()
        _layout.value = toSave
        _dirty.value = false
        _issues.value = _layout.value.validate()
        applyActiveLayoutId(targetId)
    }

    /** Rename the in-progress layout (persisted on the next [saveCurrent]). */
    fun setName(name: String) {
        val trimmed = name.takeIf { it.isNotBlank() } ?: return
        _layout.value = _layout.value.copy(name = trimmed)
        _dirty.value = true
    }

    /** Delete the current layout (no-op for built-ins) and reset to default. */
    fun deleteCurrent() {
        val base = _layout.value
        if (!base.editable) return
        repository.delete(base.id)
        _savedLayouts.value = repository.list()
        loadLayout(DEFAULT_ID)
    }

    /** Discard unsaved edits, reloading the layout from its source. */
    fun resetCurrent() = loadLayout(_layout.value.id)

    /** Clone the current layout as a new custom (editable) layout ("save as"). */
    fun duplicateCurrent(name: String? = null) = saveCurrent(name, asCopy = true)

    /** A structural summary of the layout currently being edited. */
    fun summary(): LayoutSummary = _layout.value.summary()

    // ---- Internals ----------------------------------------------------------

    private fun updateElementById(id: String, block: (ControlElement) -> ControlElement) {
        val cur = _layout.value
        val next = cur.copy(elements = cur.elements.map { if (it.id == id) block(it) else it })
        _layout.value = next.sanitized()
        _dirty.value = true
        _issues.value = _layout.value.validate()
    }

    private fun uniqueElementId(prefix: String): String {
        val taken = _layout.value.elements.map { it.id }.toSet()
        var n = _layout.value.elements.size + 1
        var candidate = "${prefix}_$n"
        while (taken.contains(candidate)) candidate = "${prefix}_${++n}"
        return candidate
    }

    private fun uniqueId(base: String): String {
        val slug = base.lowercase()
            .replace(Regex("[^a-z0-9]+"), "_")
            .trim('_')
            .takeIf { it.isNotBlank() } ?: "layout"
        val taken = (ControlLayoutCatalog.all().map { it.id } + repository.list().map { it.id }).toSet()
        var candidate = "custom_$slug"
        var n = 1
        while (taken.contains(candidate)) candidate = "custom_${slug}_${n++}"
        return candidate
    }

    private fun applyActiveLayoutId(id: String) {
        runCatching {
            val current = settingsRepository.load()
            settingsRepository.save(current.copy(controllerLayoutId = id).sanitized())
        }
    }
}
