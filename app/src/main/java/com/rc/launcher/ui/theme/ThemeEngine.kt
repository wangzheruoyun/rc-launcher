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
 *
 * Task 11 extends the engine with a [BackgroundConfig] [StateFlow] that drives the
 * custom launcher background (home + launch). Like the theme/night-mode state it
 * is observable, mutated only through the helpers below, and persisted via the
 * same [ThemeStorage].
 */
object ThemeEngine {
    private val _availableThemes = MutableStateFlow(RcBuiltInThemes)
    val availableThemes: StateFlow<List<ThemeData>> = _availableThemes.asStateFlow()

    private val _currentTheme = MutableStateFlow(RcBuiltInThemes.first())
    val currentTheme: StateFlow<ThemeData> = _currentTheme.asStateFlow()

    private val _nightMode = MutableStateFlow(ThemeNightMode.SYSTEM)
    val nightMode: StateFlow<ThemeNightMode> = _nightMode.asStateFlow()

    private val _backgroundConfig = MutableStateFlow(BackgroundConfig.DEFAULT)
    val backgroundConfig: StateFlow<BackgroundConfig> = _backgroundConfig.asStateFlow()

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
        _backgroundConfig.value = s.getBackgroundConfig().normalized()
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

    // ---- Background (task 11) ---------------------------------------------

    private fun updateBackground(transform: (BackgroundConfig) -> BackgroundConfig) {
        val next = transform(_backgroundConfig.value).normalized()
        _backgroundConfig.value = next
        storage?.setBackgroundConfig(next)
    }

    fun setBackgroundEnabled(enabled: Boolean) = updateBackground { it.copy(enabled = enabled) }

    /**
     * Set the chosen image source. Passing null clears the image (and disables the
     * background). The [uri] is expected to be either a `content://` grant or a
     * `file://` path vetted by [BackgroundValidator]; an invalid value is rejected
     * and the previous config is kept.
     */
    fun setBackgroundUri(uri: String?) {
        if (uri == null) {
            updateBackground { BackgroundConfig.DEFAULT }
            return
        }
        if (!BackgroundValidator.isValidUri(uri)) return
        updateBackground { it.copy(uri = uri, enabled = true) }
    }

    fun setBackgroundEffect(effect: BackgroundEffect) = updateBackground { it.copy(effect = effect) }

    fun setBlurRadius(dp: Int) = updateBackground {
        it.copy(blurRadiusDp = dp.coerceIn(BackgroundLimits.MIN_BLUR_DP, BackgroundLimits.MAX_BLUR_DP))
    }

    fun setDarkenAlpha(alpha: Float) = updateBackground {
        it.copy(darkenAlpha = alpha.coerceIn(BackgroundLimits.MIN_DARKEN, BackgroundLimits.MAX_DARKEN))
    }

    fun setFollowTheme(follow: Boolean) = updateBackground { it.copy(followTheme = follow) }

    /** Reset the background to its defaults and drop the saved image source. */
    fun clearBackground() = updateBackground { BackgroundConfig.DEFAULT }

    /** Replace the whole background config at once (used by the settings UI). */
    fun setBackgroundConfig(config: BackgroundConfig) = updateBackground { config }
}
