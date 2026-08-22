package com.rc.launcher.ui.model

import android.content.Context
import android.content.SharedPreferences

/**
 * Persistence contract for [LauncherSettings] (task 14).
 *
 * Swappable for tests / previews — mirroring the theme engine's [ThemeStorage]
 * split. The production implementation is backed by [SharedPreferences]; an
 * [InMemorySettingsRepository] keeps the ViewModel unit-testable on the JVM with
 * zero Android dependencies.
 */
interface SettingsRepository {
    /** Load the current settings, or [LauncherSettings] defaults if absent. */
    fun load(): LauncherSettings

    /** Persist [settings]. Implementations should be tolerant of partial writes. */
    fun save(settings: LauncherSettings)
}

/**
 * Volatile, process-local store used by previews and unit tests. Round-trips the
 * in-memory copy exactly, which is what the tests assert against.
 */
class InMemorySettingsRepository(
    initial: LauncherSettings = LauncherSettings(),
) : SettingsRepository {
    private var current: LauncherSettings = initial

    override fun load(): LauncherSettings = current
    override fun save(settings: LauncherSettings) {
        current = settings
    }
}

/**
 * [SharedPreferences]-backed [SettingsRepository]. One key per primitive keeps
 * the on-disk format forgiving: a missing key simply falls back to the
 * [LauncherSettings] default, so a partial / older prefs file never fails to
 * load (task 19 robustness).
 */
class SharedPreferencesSettingsRepository(
    context: Context,
) : SettingsRepository {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    override fun load(): LauncherSettings {
        val s = LauncherSettings()
        val str = { key: String, def: String -> prefs.getString(key, def) ?: def }
        val bool = { key: String, def: Boolean -> prefs.getBoolean(key, def) }
        val int = { key: String, def: Int -> prefs.getInt(key, def) }
        val intOrNull = { key: String ->
            if (prefs.contains(key)) prefs.getInt(key, 0).takeIf { it != 0 } else null
        }
        return LauncherSettings(
            mirrorId = str(KEY_MIRROR, s.mirrorId),
            autoSelectFastestMirror = bool(KEY_AUTO_MIRROR, s.autoSelectFastestMirror),
            useDoh = bool(KEY_USE_DOH, s.useDoh),
            dohServerUrl = str(KEY_DOH_URL, s.dohServerUrl),
            javaHeapMb = int(KEY_HEAP, s.javaHeapMb),
            javaMinHeapMb = intOrNull(KEY_MIN_HEAP),
            autoAllocateMemory = bool(KEY_AUTO_HEAP, s.autoAllocateMemory),
            javaVersion = intOrNull(KEY_JAVA_VER),
            javaArgs = str(KEY_JAVA_ARGS, s.javaArgs),
            rendererId = str(KEY_RENDERER, s.rendererId),
            resolutionMode = ResolutionMode.fromName(prefs.getString(KEY_RES_MODE, s.resolutionMode.name)),
            customWidth = int(KEY_RES_W, s.customWidth),
            customHeight = int(KEY_RES_H, s.customHeight),
            resolutionScale = prefs.getFloat(KEY_RES_SCALE, s.resolutionScale),
            framerateLimit = int(KEY_FPS, s.framerateLimit),
            fullscreen = bool(KEY_FULLSCREEN, s.fullscreen),
            controllerEnabled = bool(KEY_CTRL_ENABLED, s.controllerEnabled),
            controllerLayoutId = str(KEY_CTRL_LAYOUT, s.controllerLayoutId),
            controllerDeadzone = prefs.getFloat(KEY_CTRL_DEADZONE, s.controllerDeadzone),
            controllerVibration = bool(KEY_CTRL_VIB, s.controllerVibration),
            gameFilesRoot = str(KEY_DIR_ROOT, s.gameFilesRoot),
            autoCleanLogs = bool(KEY_AUTO_CLEAN, s.autoCleanLogs),
            keepCrashReports = bool(KEY_KEEP_CRASH, s.keepCrashReports),
        ).sanitized()
    }

    override fun save(settings: LauncherSettings) {
        prefs.edit().apply {
            putString(KEY_MIRROR, settings.mirrorId)
            putBoolean(KEY_AUTO_MIRROR, settings.autoSelectFastestMirror)
            putBoolean(KEY_USE_DOH, settings.useDoh)
            putString(KEY_DOH_URL, settings.dohServerUrl)
            putInt(KEY_HEAP, settings.javaHeapMb)
            if (settings.javaMinHeapMb != null) {
                putInt(KEY_MIN_HEAP, settings.javaMinHeapMb)
            } else {
                remove(KEY_MIN_HEAP)
            }
            putBoolean(KEY_AUTO_HEAP, settings.autoAllocateMemory)
            if (settings.javaVersion != null) {
                putInt(KEY_JAVA_VER, settings.javaVersion)
            } else {
                remove(KEY_JAVA_VER)
            }
            putString(KEY_JAVA_ARGS, settings.javaArgs)
            putString(KEY_RENDERER, settings.rendererId)
            putString(KEY_RES_MODE, settings.resolutionMode.name)
            putInt(KEY_RES_W, settings.customWidth)
            putInt(KEY_RES_H, settings.customHeight)
            putFloat(KEY_RES_SCALE, settings.resolutionScale)
            putInt(KEY_FPS, settings.framerateLimit)
            putBoolean(KEY_FULLSCREEN, settings.fullscreen)
            putBoolean(KEY_CTRL_ENABLED, settings.controllerEnabled)
            putString(KEY_CTRL_LAYOUT, settings.controllerLayoutId)
            putFloat(KEY_CTRL_DEADZONE, settings.controllerDeadzone)
            putBoolean(KEY_CTRL_VIB, settings.controllerVibration)
            putString(KEY_DIR_ROOT, settings.gameFilesRoot)
            putBoolean(KEY_AUTO_CLEAN, settings.autoCleanLogs)
            putBoolean(KEY_KEEP_CRASH, settings.keepCrashReports)
        }.apply()
    }

    companion object {
        private const val NAME = "rc_settings"
        private const val KEY_MIRROR = "mirror_id"
        private const val KEY_AUTO_MIRROR = "auto_select_fastest_mirror"
        private const val KEY_USE_DOH = "use_doh"
        private const val KEY_DOH_URL = "doh_server_url"
        private const val KEY_HEAP = "java_heap_mb"
        private const val KEY_MIN_HEAP = "java_min_heap_mb"
        private const val KEY_AUTO_HEAP = "auto_allocate_memory"
        private const val KEY_JAVA_VER = "java_version"
        private const val KEY_JAVA_ARGS = "java_args"
        private const val KEY_RENDERER = "renderer_id"
        private const val KEY_RES_MODE = "resolution_mode"
        private const val KEY_RES_W = "custom_width"
        private const val KEY_RES_H = "custom_height"
        private const val KEY_RES_SCALE = "resolution_scale"
        private const val KEY_FPS = "framerate_limit"
        private const val KEY_FULLSCREEN = "fullscreen"
        private const val KEY_CTRL_ENABLED = "controller_enabled"
        private const val KEY_CTRL_LAYOUT = "controller_layout_id"
        private const val KEY_CTRL_DEADZONE = "controller_deadzone"
        private const val KEY_CTRL_VIB = "controller_vibration"
        private const val KEY_DIR_ROOT = "game_files_root"
        private const val KEY_AUTO_CLEAN = "auto_clean_logs"
        private const val KEY_KEEP_CRASH = "keep_crash_reports"
    }
}

/**
 * Process-wide settings repository holder. The real implementation is installed
 * from [com.rc.launcher.RcApplication.onCreate]; until then (e.g. Compose
 * previews) a throwaway [InMemorySettingsRepository] is used so the UI never
 * crashes for lack of an Android context.
 */
object SettingsRepositories {
    @Volatile
    private var _default: SettingsRepository? = null

    val default: SettingsRepository
        get() = _default ?: InMemorySettingsRepository().also { _default = it }

    fun install(repository: SettingsRepository) {
        _default = repository
    }
}
