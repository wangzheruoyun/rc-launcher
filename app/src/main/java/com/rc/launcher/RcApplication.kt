package com.rc.launcher

import android.app.Application
import com.rc.launcher.core.RcEventBus
import com.rc.launcher.ui.model.SettingsRepositories
import com.rc.launcher.ui.model.AccountRepositories
import com.rc.launcher.ui.model.RustAccountRepository
import com.rc.launcher.ui.model.ControlLayoutRepositories
import com.rc.launcher.ui.model.SharedPreferencesControlLayoutRepository
import com.rc.launcher.ui.model.SharedPreferencesSettingsRepository
import com.rc.launcher.ui.awt.AwtBridges
import com.rc.launcher.ui.awt.RustAwtCanvasBridge
import com.rc.launcher.ui.i18n.LocaleEngine
import com.rc.launcher.ui.theme.ThemeEngine

/**
 * Application entry point. Initialises the shared [ThemeEngine] from the saved
 * preferences and connects the Rust-core [RcEventBus] (task 10) so events start
 * flowing as soon as the process is created — mirroring the pattern noted in
 * [RcEventBus]'s documentation (connect in `Application.onCreate`).
 */
class RcApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        ThemeEngine.init(this)
        // Wire the i18n framework (task 20) before any UI is created: it restores
        // the saved language, resolves "follow system" against the device locale
        // list and tells the Rust core which language to speak.
        LocaleEngine.init(this)
        // Wire the Settings Center (task 14) to on-disk persistence.
        SettingsRepositories.install(SharedPreferencesSettingsRepository(this))
        // Wire the account store (task 16) to the Rust core FFI bridge.
        AccountRepositories.install(RustAccountRepository(this))
        // Wire the controller / input-mapping layouts (task 15) to on-disk persistence.
        ControlLayoutRepositories.install(SharedPreferencesControlLayoutRepository(this))
        // Wire the AWT/Swing compatibility layer (task 18) to the Rust core.
        AwtBridges.install(RustAwtCanvasBridge())
        // The native core may be absent in some builds; never let that crash
        // the whole process — degrade gracefully (task 19).
        runCatching { RcEventBus.connect() }
    }
}
