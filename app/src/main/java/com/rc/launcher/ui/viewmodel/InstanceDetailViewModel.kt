package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.GameDirectoryType
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * State container for the instance-detail / settings-edit screen (task 13).
 *
 * It loads one [GameInstance] by id into [instance] and exposes immutable,
 * copy-on-write mutators that write straight back into [InstanceRepository].
 * This is the "settings editing & version isolation" half of task 13: every
 * change (name, notes, icon, Java version, directory-isolation strategy) is
 * persisted immediately, mirroring FCL's `VersionSetting` semantics.
 *
 * The repository is injected with a default so `viewModel()` can instantiate it
 * and tests can pass a custom repository.
 */
class InstanceDetailViewModel(
    private val repository: InstanceRepository = InstanceRepository,
) : ViewModel() {

    private val _instance = MutableStateFlow<GameInstance?>(null)
    val instance: StateFlow<GameInstance?> = _instance.asStateFlow()

    /** Load the instance with [id] into the editable state. */
    fun load(id: String) {
        _instance.value = repository.getById(id)
    }

    /** Apply [transform] to the current instance and persist the result. */
    fun update(transform: (GameInstance) -> GameInstance) {
        val current = _instance.value ?: return
        val updated = transform(current)
        _instance.value = updated
        repository.update(updated)
    }

    fun setName(name: String) = update { it.copy(name = name) }
    fun setNotes(notes: String) = update { it.copy(notes = notes) }
    fun setIconColor(color: Long) = update { it.copy(iconColor = color) }
    fun setJavaVersion(version: Int?) = update { it.copy(javaVersion = version) }
    fun setLoaderVersion(version: String?) = update { it.copy(loaderVersion = version) }

    fun setGameDirectoryType(type: GameDirectoryType) =
        update { it.copy(gameDirectoryType = type, customGameDir = if (type != GameDirectoryType.CUSTOM) null else it.customGameDir) }

    fun setCustomGameDir(dir: String) =
        update { it.copy(customGameDir = dir.ifBlank { null }) }

    fun toggleFavorite() = update { it.copy(isFavorite = !it.isFavorite) }

    /**
     * Delete the loaded instance. Returns the deleted id (for navigation) or
     * null when nothing was loaded.
     */
    fun delete(): String? {
        val current = _instance.value ?: return null
        repository.remove(current.id)
        _instance.value = null
        return current.id
    }

    /**
     * Duplicate the currently-loaded instance. The clone is persisted with a
     * unique id (and optional [overrideName]); the original stays loaded so the
     * user keeps editing it. Returns the new clone's id, or null when nothing
     * is loaded (task 13).
     */
    fun duplicate(overrideName: String? = null): String? {
        val current = _instance.value ?: return null
        val clone = repository.duplicate(current.id, overrideName) ?: return null
        return clone.id
    }
}
