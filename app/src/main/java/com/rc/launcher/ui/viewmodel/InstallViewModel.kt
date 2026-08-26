package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * State container for the version-installation wizard (task 13).
 *
 * The wizard is a small, fully-deterministic state machine: a [step] pointer and
 * an [InstallRequest] accumulator. All branching (e.g. skipping the loader step
 * for vanilla) lives in the pure [InstallStep] helpers in [com.rc.launcher.ui.model],
 * so this ViewModel stays a thin, testable holder of [StateFlow]s — no Android
 * dependencies beyond the [ViewModel] base class.
 *
 * Populating the launcher with real loader/version lists is delegated to
 * [LoaderCatalog]; the Rust core (task 4) will later replace it with live,
 * mirror-sourced metadata.
 */
class InstallViewModel(
    private val repository: InstanceRepository = InstanceRepository,
) : ViewModel() {

    private val _step = MutableStateFlow(InstallStep.LOADER)
    val step: StateFlow<InstallStep> = _step.asStateFlow()

    private val _request = MutableStateFlow(InstallRequest())
    val request: StateFlow<InstallRequest> = _request.asStateFlow()

    // ---- Field setters (each keeps the accumulator immutable & copy-on-write) ----

    fun setLoader(loader: ModLoader) {
        _request.update { req ->
            // Drop a now-irrelevant loader version when switching families.
            val lv = if (loader == ModLoader.VANILLA) null else req.loaderVersion
            req.copy(loader = loader, loaderVersion = lv)
        }
    }

    fun setGameVersion(version: String) {
        _request.update { it.copy(gameVersion = version, loaderVersion = null) }
    }

    fun setLoaderVersion(version: LoaderVersion?) {
        _request.update { it.copy(loaderVersion = version) }
    }

    fun setName(name: String) = _request.update { it.copy(name = name) }
    fun setIconColor(color: Long) = _request.update { it.copy(iconColor = color) }
    fun setNotes(notes: String) = _request.update { it.copy(notes = notes) }
    fun setJavaVersion(version: Int?) = _request.update { it.copy(javaVersion = version) }
    fun setGameDirectoryType(type: GameDirectoryType) =
        _request.update { it.copy(gameDirectoryType = type) }

    fun setCustomGameDir(dir: String) = _request.update { it.copy(customGameDir = dir) }

    // ---- Navigation ----

    /** Whether the user may advance from the current step. */
    fun canProceed(): Boolean = _step.value.canProceed(_request.value)

    /** True while the wizard is not on the first step. */
    fun canGoBack(): Boolean = _step.value.previous(_request.value) != null

    /** Advance one step; returns false if already on the last step. */
    fun next(): Boolean {
        val next = _step.value.next(_request.value) ?: return false
        _step.value = next
        return true
    }

    /** Step back one step; returns false if already on the first step. */
    fun back(): Boolean {
        val prev = _step.value.previous(_request.value) ?: return false
        _step.value = prev
        return true
    }

    /** Restart the wizard with a fresh request. */
    fun reset() {
        _step.value = InstallStep.LOADER
        _request.value = InstallRequest()
    }

    // ---- Catalog helpers (UI convenience) ----

    fun availableGameVersions(): List<String> = LoaderCatalog.gameVersions

    fun availableLoaderVersions(): List<LoaderVersion> =
        LoaderCatalog.loaderVersions(_request.value.loader, _request.value.gameVersion)

    // ---- Commit ----

    /**
     * Build a [GameInstance] from the current [request], ensure a unique id
     * (never silently overwrite an existing instance) and persist it.
     *
     * Returns the created instance, or `null` when the request is not valid
     * ([InstallRequest.isValid] is false). The UI only reaches the commit step
     * after [canProceed] passes, but the guard keeps the repository free of
     * half-formed instances even if [create] is called out of order.
     */
    fun create(): GameInstance? {
        val req = _request.value
        if (!req.isValid) return null
        val existing = repository.instances.value.map { it.id }.toSet()
        var id = req.defaultId()
        var n = 1
        while (id in existing) {
            id = "${req.defaultId()}-$n"
            n++
        }
        val instance = req.buildInstance(id)
        repository.add(instance)
        return instance
    }
}
