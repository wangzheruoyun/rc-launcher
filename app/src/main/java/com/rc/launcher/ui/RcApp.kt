package com.rc.launcher.ui

import androidx.compose.runtime.Composable
import androidx.compose.material3.Text
import androidx.compose.ui.tooling.preview.Preview
import android.content.res.Configuration
import com.rc.launcher.ui.theme.RcBuiltInThemes
import com.rc.launcher.ui.theme.ThemeNightMode
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.theme.RcTheme
import com.rc.launcher.ui.i18n.RcLocalizationProvider
import com.rc.launcher.ui.theme.ThemeViewModel
import com.rc.launcher.ui.theme.RcBackground
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize

/**
 * Root composable of the launcher UI. It binds the [ThemeViewModel] to the
 * [RcTheme], publishes the adaptive [RcWindowInfo] (task 9) and mounts the
 * [MainScreen] navigation shell. This is the single entry point referenced from
 * [com.rc.launcher.MainActivity].
 */
@Composable
fun RcApp() {
    val themeViewModel: ThemeViewModel = viewModel()
    val theme by themeViewModel.currentTheme.collectAsStateWithLifecycle()
    val nightMode by themeViewModel.nightMode.collectAsStateWithLifecycle()

    // Task 20: the localisation provider wraps the whole UI, so `rcString(...)`
    // and `stringResource(...)` both follow the in-app language choice.
    RcLocalizationProvider {
        RcTheme(theme = theme, nightMode = nightMode) {
            // Task 9: measure the window once, at the root, and publish it as
            // `LocalRcWindowInfo`. Every screen below re-lays itself out from that
            // single measurement, so a rotation (which does *not* recreate the
            // Activity) simply re-measures and recomposes.
            ProvideRcWindowInfo {
                // Task 11: the custom launcher background (home + launch) is the
                // backmost layer; the scaffold above draws on top of it.
                Box(modifier = Modifier.fillMaxSize()) {
                    RcBackground(modifier = Modifier.fillMaxSize())
                    MainScreen()
                }
            }
        }
    }
}


@Preview(name = "Light", showBackground = true)
@Composable
private fun RcThemeLightPreview() {
    RcTheme(theme = RcBuiltInThemes.first(), nightMode = ThemeNightMode.LIGHT) {
        Text("Light 预览")
    }
}

@Preview(name = "Dark", showBackground = true, uiMode = Configuration.UI_MODE_NIGHT_YES)
@Composable
private fun RcThemeDarkPreview() {
    RcTheme(theme = RcBuiltInThemes.first(), nightMode = ThemeNightMode.DARK) {
        Text("Dark 预览")
    }
}
