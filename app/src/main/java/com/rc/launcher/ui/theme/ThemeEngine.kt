package com.rc.launcher.ui.theme

import android.content.Context
import kotlin.jvm.Volatile
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * App-wide, observable theme engine — the "ThemeEngine" concept borrowed from
 * FCL's `fcllibrary/theme/ThemeEngine.kt`. It owns the current [ThemeData] and
 * [ThemeNightMode] and exposes them as [StateFlow]s so Compose recomposes when
 * they change. Selection is persisted through a [ThemeStorage].
 *
 * A single instance is shared across the process (mirroring FCL's singleton
 * engine) and is initialised from [RcApplication.onCreate].
 */
object ThemeEngine {
    private val _availableThemes = MutableStateFlow(RcBuiltInThemes)
    val availableThemes: StateFlow<List<ThemeData>> = _availableThemes.asStateFlow()

    private val _currentTheme = MutableStateFlow(RcBuiltInThemes.first())
    val currentTheme: StateFlow<ThemeData> = _currentTheme.asStateFlow()

    private val _nightMode = MutableStateFlow(ThemeNightMode.SYSTEM)
    val nightMode: StateFlow<ThemeNightMode> = _nightMode.asStateFlow()

    @Volatile
    private var storage: ThemeStorage? = null

    /** Wire the engine to a [Context] and restore the saved selection. Idempotent. */
    fun init(context: Context) {
        val s = SharedPreferencesThemeStorage(context.applicationContext)
        storage = s
        val savedId = s.getThemeId()
        _currentTheme.value = RcBuiltInThemes.firstOrNull { it.id == savedId }
            ?: RcBuiltInThemes.first()
        _nightMode.value = ThemeNightMode.fromValue(s.getNightMode())
    }

    fun setTheme(id: String) {
        val theme = availableThemes.value.firstOrNull { it.id == id } ?: return
        _currentTheme.value = theme
        storage?.setThemeId(id)
    }

    fun setNightMode(mode: ThemeNightMode) {
        _nightMode.value = mode
        storage?.setNightMode(mode.value)
    }

    /** SYSTEM -> LIGHT -> DARK -> SYSTEM, handy for a one-tap toggle. */
    fun cycleNightMode() = setNightMode(_nightMode.value.next())
}
