package com.rc.launcher

import android.app.ActivityInfo
import android.content.res.Configuration
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.viewModels
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import com.rc.launcher.ui.RcApp
import com.rc.launcher.ui.i18n.LocaleEngine
import com.rc.launcher.ui.i18n.RcLocaleContext
import com.rc.launcher.ui.model.OrientationMode
import com.rc.launcher.ui.viewmodel.SettingsViewModel
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    // Shared with the Compose settings screen (same activity-scoped ViewModelStore),
    // so a change made in SettingsScreen is reflected here immediately.
    private val settingsViewModel: SettingsViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Task 9 — screen orientation:
        //
        // * The manifest declares `screenOrientation="user"` (rotation allowed,
        //   but the device rotation lock is honoured) and
        //   `configChanges="orientation|screenSize|screenLayout|smallestScreenSize|…"`,
        //   so rotating does **not** recreate the Activity: Compose re-measures
        //   in place, no screen state is lost, and the game / AWT surface keeps
        //   its coordinate space (it is only re-letterboxed), which is what stops
        //   touches from landing at the wrong place after a rotation.
        // * The user's choice (跟随系统 / 强制横屏 / 强制竖屏) is applied here and
        //   kept in sync with the persisted settings, so flipping it in the
        //   settings screen rotates the launcher immediately.
        applyOrientation(settingsViewModel.settings.value.orientationMode())
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                settingsViewModel.settings.collect { applyOrientation(it.orientationMode()) }
            }
        }
        setContent {
            RcApp()
        }
    }

    /**
     * Map the launcher's orientation strategy onto an Android
     * `requestedOrientation`.
     *
     * The mapping is asserted against [OrientationMode.androidScreenOrientation]
     * (and therefore against the Rust core's `display::OrientationPolicy`) by
     * `MainActivityOrientationTest`, so the three layers can never disagree.
     */
    internal fun applyOrientation(mode: OrientationMode) {
        val requested = when (mode) {
            // `USER`, not `UNSPECIFIED`: respect the device rotation lock.
            OrientationMode.FOLLOW_SYSTEM -> ActivityInfo.SCREEN_ORIENTATION_USER
            // `SENSOR_*`: pin the axis but allow both directions of it.
            OrientationMode.LANDSCAPE -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            OrientationMode.PORTRAIT -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
        }
        if (requestedOrientation != requested) {
            requestedOrientation = requested
        }
    }

    /**
     * The manifest declares `android:configChanges="…|orientation|screenSize|
     * locale|layoutDirection"`, so the Activity is *not* recreated when the user
     * rotates the device or changes the system language.
     *
     * * Rotation (task 9): nothing to do here — Compose re-measures the window
     *   and `LocalRcWindowInfo` re-publishes the new size class, which re-flows
     *   the grids and swaps the bottom bar for the navigation rail.
     * * Language (task 20): feed the new preference list to the engine so a
     *   "follow system" selection re-resolves immediately.
     */
    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        LocaleEngine.onSystemLocalesChanged(RcLocaleContext.systemPreferredTags(this))
    }
}
