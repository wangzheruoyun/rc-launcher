package com.rc.launcher.ui

import androidx.compose.runtime.Composable
import androidx.compose.material3.Text
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.Modifier
import android.content.res.Configuration
import com.rc.launcher.ui.theme.RcBuiltInThemes
import com.rc.launcher.ui.theme.ThemeNightMode
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.theme.RcTheme
import com.rc.launcher.ui.i18n.RcLocalizationProvider
import com.rc.launcher.ui.theme.ThemeViewModel
import com.rc.launcher.ui.theme.RcBackground
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.navigation.NavHostController
import androidx.navigation.NavDestination.Companion.hasRoute
import androidx.navigation.compose.rememberNavController
import com.rc.launcher.ui.model.TutorialState
import com.rc.launcher.ui.navigation.OnboardingRoute
import com.rc.launcher.ui.viewmodel.TutorialViewModel

/**
 * Root composable of the launcher UI. It binds the [ThemeViewModel] to the
 * [RcTheme], publishes the adaptive [RcWindowInfo] (task 9), owns the single
 * shared [NavHostController], mounts the [MainScreen] navigation shell, and
 * (task 14) overlays the first-run onboarding on top of the shell until the
 * user finishes or skips it. This is the single entry point referenced from
 * [com.rc.launcher.MainActivity].
 *
 * The [NavHostController] is created here \u2014 not inside [MainScreen] \u2014 so the
 * onboarding overlay ([FirstRunOnboardingGate]) and the scaffold below share
 * the *same* back stack. A `Finish` from the onboarding therefore pops back to
 * the home tab exactly as the user expects, with no second NavController to
 * keep in sync.
 */
@Composable
fun RcApp() {
    val themeViewModel: ThemeViewModel = viewModel()
    val theme by themeViewModel.currentTheme.collectAsStateWithLifecycle()
    val nightMode by themeViewModel.nightMode.collectAsStateWithLifecycle()

    // Task 14: pull the tutorial state at startup. The repository read is
    // synchronous and tiny (two booleans / one int), so a one-shot collect
    // here is enough \u2014 no need to keep the StateFlow alive for the whole
    // lifetime.
    val tutorialViewModel: TutorialViewModel = viewModel()
    val tutorialState by tutorialViewModel.state.collectAsStateWithLifecycle()

    // One NavController, owned at the root, shared by the shell and the
    // onboarding overlay. Created with `rememberNavController` so a rotation
    // (which does *not* recreate the Activity, see task 9 manifest
    // configChanges) preserves the back stack.
    val navController = rememberNavController()

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
                    MainScreen(navController = navController)

                    // Task 14: first-run onboarding is layered on top of the
                    // shell using the *same* nav controller. The
                    // [LaunchedEffect] decides exactly once per "not yet
                    // completed" state transition whether to push
                    // [OnboardingRoute]; subsequent recompositions (language
                    // change, theme change, rotation) are no-ops so the user
                    // is never re-prompted against their will.
                    FirstRunOnboardingGate(
                        tutorialState = tutorialState,
                        navController = navController,
                    )
                }
            }
        }
    }
}

/**
 * Task 14 \u2014 the bridge between the persisted [TutorialState] and the root
 * [NavHostController].
 *
 * Reads the state's `completed` flag once and, when the user has not yet
 * finished the tutorial, pushes [OnboardingRoute] onto the back stack.
 * Because [LaunchedEffect] keys on the value, this fires only when the user
 * transitions from "incomplete" to "complete" (or back, via the "Rewatch"
 * button in Settings) \u2014 it never loops.
 */
@Composable
private fun FirstRunOnboardingGate(
    tutorialState: TutorialState,
    navController: NavHostController,
) {
    LaunchedEffect(tutorialState.completed) {
        if (!tutorialState.completed) {
            val current = navController.currentDestination
            // `hasRoute<OnboardingRoute>()` is the type-safe way to ask
            // "are we already on the onboarding screen?". Using it instead of
            // a string match means the route identifier is computed by the
            // navigation library itself, so a future change to how type-safe
            // routes are serialised (e.g. a kotlinx-serialization upgrade that
            // changes the serialName) cannot break this check.
            if (current == null || !current.hasRoute<OnboardingRoute>()) {
                navController.navigate(OnboardingRoute)
            }
        }
    }
}

@Preview(name = "Light", showBackground = true)
@Composable
private fun RcThemeLightPreview() {
    RcTheme(theme = RcBuiltInThemes.first(), nightMode = ThemeNightMode.LIGHT) {
        Text("Light \u9884\u89c8")
    }
}

@Preview(name = "Dark", showBackground = true, uiMode = Configuration.UI_MODE_NIGHT_YES)
@Composable
private fun RcThemeDarkPreview() {
    RcTheme(theme = RcBuiltInThemes.first(), nightMode = ThemeNightMode.DARK) {
        Text("Dark \u9884\u89c8")
    }
}
