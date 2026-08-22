package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import com.rc.launcher.ui.model.DohServer
import com.rc.launcher.ui.model.DohCatalog
import com.rc.launcher.ui.model.LauncherSettings
import com.rc.launcher.ui.model.MirrorCatalog
import com.rc.launcher.ui.model.MirrorSource
import com.rc.launcher.ui.model.RendererOption
import com.rc.launcher.ui.model.ResolutionMode
import com.rc.launcher.ui.model.SettingsRepositories
import com.rc.launcher.ui.model.SettingsRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * State container for the Settings Center (task 14).
 *
 * Thin, deterministic holder of a single [LauncherSettings] [StateFlow]: every
 * mutator copies the current value, runs it through [LauncherSettings.sanitized]
 * (so partial / out-of-range input can never reach the core) and persists the
 * result via the injected [SettingsRepository]. No Android dependency beyond the
 * [ViewModel] base class, which keeps it fully unit-testable on the JVM.
 *
 * The repository defaults to the process-wide [SettingsRepositories.default]
 * (installed from [com.rc.launcher.RcApplication]); tests pass an
 * [com.rc.launcher.ui.model.InMemorySettingsRepository] explicitly.
 */
class SettingsViewModel(
    private val repository: SettingsRepository = SettingsRepositories.default,
) : ViewModel() {

    private val _settings = MutableStateFlow(repository.load().sanitized())
    val settings: StateFlow<LauncherSettings> = _settings.asStateFlow()

    // Catalogues for the UI (single source of truth, shared with the Rust core).
    val mirrors: List<MirrorSource> = MirrorCatalog.all
    val renderers: List<RendererOption> = RendererOption.entries
    val dohServers: List<DohServer> = DohCatalog.all

    /** Apply [next], sanitize, publish and persist (failures are swallowed). */
    private fun commit(next: LauncherSettings) {
        val clean = next.sanitized()
        _settings.value = clean
        runCatching { repository.save(clean) }
    }

    // ---- Network / China optimisation --------------------------------------

    fun setMirror(id: String) = commit(_settings.value.copy(mirrorId = id))
    fun setAutoSelectFastestMirror(on: Boolean) =
        commit(_settings.value.copy(autoSelectFastestMirror = on))
    fun setUseDoh(on: Boolean) = commit(_settings.value.copy(useDoh = on))
    fun setDohServer(url: String) = commit(_settings.value.copy(dohServerUrl = url))

    // ---- Java / memory -----------------------------------------------------

    fun setJavaHeapMb(mb: Int) = commit(_settings.value.copy(javaHeapMb = mb))
    fun setJavaMinHeapMb(mb: Int?) = commit(_settings.value.copy(javaMinHeapMb = mb))
    fun setAutoAllocateMemory(on: Boolean) =
        commit(_settings.value.copy(autoAllocateMemory = on))
    fun setJavaVersion(version: Int?) = commit(_settings.value.copy(javaVersion = version))
    fun setJavaArgs(args: String) = commit(_settings.value.copy(javaArgs = args))

    // ---- Renderer / window -------------------------------------------------

    fun setRenderer(id: String) = commit(_settings.value.copy(rendererId = id))
    fun setResolutionMode(mode: ResolutionMode) =
        commit(_settings.value.copy(resolutionMode = mode))
    fun setCustomResolution(width: Int, height: Int) =
        commit(_settings.value.copy(customWidth = width, customHeight = height))
    fun setResolutionScale(scale: Float) =
        commit(_settings.value.copy(resolutionScale = scale))
    fun setFramerateLimit(fps: Int) = commit(_settings.value.copy(framerateLimit = fps))
    fun setFullscreen(on: Boolean) = commit(_settings.value.copy(fullscreen = on))

    // ---- Controller --------------------------------------------------------

    fun setControllerEnabled(on: Boolean) =
        commit(_settings.value.copy(controllerEnabled = on))
    fun setControllerLayout(id: String) = commit(_settings.value.copy(controllerLayoutId = id))
    fun setControllerDeadzone(value: Float) =
        commit(_settings.value.copy(controllerDeadzone = value))
    fun setControllerVibration(on: Boolean) =
        commit(_settings.value.copy(controllerVibration = on))

    // ---- Directory / misc --------------------------------------------------

    fun setGameFilesRoot(path: String) = commit(_settings.value.copy(gameFilesRoot = path))
    fun setAutoCleanLogs(on: Boolean) = commit(_settings.value.copy(autoCleanLogs = on))
    fun setKeepCrashReports(on: Boolean) =
        commit(_settings.value.copy(keepCrashReports = on))

    /** Restore factory defaults. */
    fun resetToDefaults() = commit(LauncherSettings())
}
