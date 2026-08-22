package com.rc.launcher.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BrightnessAuto
import androidx.compose.material.icons.filled.DarkMode
import androidx.compose.material.icons.filled.LightMode
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import com.rc.launcher.ui.navigation.RcBottomNavigationBar
import com.rc.launcher.ui.navigation.RcNavHost
import com.rc.launcher.ui.navigation.RcRoutes
import com.rc.launcher.ui.navigation.RcTopLevelDestinations
import com.rc.launcher.ui.i18n.LocalRcStrings
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.theme.ThemeNightMode
import com.rc.launcher.ui.theme.ThemeViewModel

/**
 * The app shell: a [Scaffold] with a top app bar (dynamic title + a night-mode
 * quick toggle), a bottom navigation bar, and a [RcNavHost] hosting the
 * top-level screens. This is the "main framework" of task 11.
 *
 * The [ThemeViewModel] is scoped to the Activity, so the toggle here and the
 * settings screen share the same observable theme state.
 */
@Composable
fun MainScreen() {
    val navController = rememberNavController()
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute = navBackStackEntry?.destination?.route

    val themeVm: ThemeViewModel = viewModel()
    val nightMode by themeVm.nightMode.collectAsStateWithLifecycle()

    // Task 20: every title comes from the i18n catalogue, so switching the
    // language re-titles the app bar on the next recomposition.
    val strings = LocalRcStrings.current
    val title = when {
        currentRoute?.startsWith("instance/") == true ->
            strings[RcStringKeys.SCREEN_INSTANCE_DETAIL]
        currentRoute == RcRoutes.INSTALL -> strings[RcStringKeys.SCREEN_INSTALL]
        currentRoute == RcRoutes.CONTROLLER -> strings[RcStringKeys.SCREEN_CONTROLLER]
        currentRoute == RcRoutes.AWT -> strings[RcStringKeys.SCREEN_AWT]
        else -> RcTopLevelDestinations
            .firstOrNull { it.route == currentRoute }
            ?.let { strings[it.labelKey] }
    } ?: strings[RcStringKeys.APP_NAME]

    Scaffold(
        topBar = {
            CenterAlignedTopAppBar(
                title = { Text(title) },
                actions = {
                    IconButton(onClick = { themeVm.cycleNightMode() }) {
                        Icon(
                            imageVector = when (nightMode) {
                                ThemeNightMode.LIGHT -> Icons.Filled.LightMode
                                ThemeNightMode.DARK -> Icons.Filled.DarkMode
                                ThemeNightMode.SYSTEM -> Icons.Filled.BrightnessAuto
                            },
                            contentDescription = strings[RcStringKeys.THEME_NIGHT_TOGGLE],
                        )
                    }
                },
            )
        },
        bottomBar = {
            RcBottomNavigationBar(
                destinations = RcTopLevelDestinations,
                currentRoute = currentRoute,
                onNavigate = { dest ->
                    navController.navigate(dest.route) {
                        // Preserve the back stack and tab state across reselects.
                        popUpTo(navController.graph.startDestinationId) { saveState = true }
                        launchSingleTop = true
                        restoreState = true
                    }
                },
            )
        },
    ) { innerPadding ->
        RcNavHost(
            navController = navController,
            modifier = Modifier.padding(innerPadding),
        )
    }
}


@Preview(showBackground = true)
@Composable
private fun MainScreenPreview() {
    MainScreen()
}
