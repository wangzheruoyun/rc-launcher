package com.rc.launcher.ui.navigation

import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Storage
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.Storage
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.navArgument
import com.rc.launcher.ui.screen.DownloadsScreen
import com.rc.launcher.ui.screen.HomeScreen
import com.rc.launcher.ui.screen.InstanceDetailScreen
import com.rc.launcher.ui.screen.InstallWizardScreen
import com.rc.launcher.ui.screen.InstancesScreen
import com.rc.launcher.ui.screen.SettingsScreen
import com.rc.launcher.ui.screen.AwtScreen
import com.rc.launcher.ui.screen.ControllerScreen
import com.rc.launcher.ui.screen.AccountsScreen
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString

/**
 * Single source of truth for navigation routes. Defining them once prevents the
 * bottom-navigation labels and the [NavHost] graph from drifting apart and lets
 * screens navigate by calling [instanceDetail] instead of hard-coding strings.
 */
object RcRoutes {
    const val HOME = "home"
    const val INSTANCES = "instances"
    const val DOWNLOADS = "downloads"
    const val SETTINGS = "settings"
    const val ACCOUNTS = "accounts"
    const val CONTROLLER = "controller"
    const val AWT = "awt"
    const val INSTALL = "install"
    const val INSTANCE_DETAIL = "instance/{id}"

    /** Builds the concrete route for the instance-detail screen. */
    fun instanceDetail(id: String): String = "instance/$id"
}

/**
 * A top-level destination shown in the bottom navigation bar. Mirrors the
 * single-Activity / multi-destination organisation of FCL's `fcl/` UI module.
 *
 * The destination carries an **i18n key** ([labelKey]) rather than a literal
 * label (task 20): the label is resolved at render time from [LocalRcStrings], so
 * switching the language re-labels the navigation bar without rebuilding the
 * destination list.
 */
data class TopLevelDestination(
    val route: String,
    val labelKey: String,
    val icon: ImageVector,
    val selectedIcon: ImageVector = icon,
)

/**
 * Canonical bottom-navigation destinations for the launcher. Screens are
 * realised in their own tasks (12 home/instances, 14 settings, …); here we only
 * define the navigation shell and placeholder screens.
 */
val RcTopLevelDestinations: List<TopLevelDestination> = listOf(
    TopLevelDestination(RcRoutes.HOME, RcStringKeys.NAV_HOME, Icons.Outlined.Home, Icons.Filled.Home),
    TopLevelDestination(RcRoutes.INSTANCES, RcStringKeys.NAV_INSTANCES, Icons.Outlined.Storage, Icons.Filled.Storage),
    TopLevelDestination(RcRoutes.DOWNLOADS, RcStringKeys.NAV_DOWNLOADS, Icons.Outlined.Download, Icons.Filled.Download),
    TopLevelDestination(RcRoutes.SETTINGS, RcStringKeys.NAV_SETTINGS, Icons.Outlined.Settings, Icons.Filled.Settings),
    TopLevelDestination(RcRoutes.ACCOUNTS, RcStringKeys.NAV_ACCOUNTS, Icons.Outlined.Person, Icons.Filled.Person),
)

@Composable
fun RcBottomNavigationBar(
    destinations: List<TopLevelDestination>,
    currentRoute: String?,
    onNavigate: (TopLevelDestination) -> Unit,
) {
    NavigationBar {
        for (dest in destinations) {
            val selected = currentRoute == dest.route
            // Task 20: resolved per recomposition, so a language switch relabels
            // the bar immediately (no Activity recreation).
            val label = rcString(dest.labelKey)
            NavigationBarItem(
                selected = selected,
                onClick = { onNavigate(dest) },
                icon = {
                    Icon(
                        imageVector = if (selected) dest.selectedIcon else dest.icon,
                        contentDescription = label,
                    )
                },
                label = { Text(label) },
            )
        }
    }
}

@Composable
fun RcNavHost(
    navController: NavHostController,
    modifier: androidx.compose.ui.Modifier = androidx.compose.ui.Modifier,
) {
    NavHost(
        navController = navController,
        startDestination = RcRoutes.HOME,
        modifier = modifier,
    ) {
        composable(RcRoutes.HOME) { HomeScreen(navController) }
        composable(RcRoutes.INSTANCES) { InstancesScreen(navController) }
        composable(RcRoutes.DOWNLOADS) { DownloadsScreen() }
        composable(RcRoutes.SETTINGS) { SettingsScreen(navController = navController) }
        composable(RcRoutes.ACCOUNTS) { AccountsScreen() }
        composable(RcRoutes.CONTROLLER) { ControllerScreen(onBack = { navController.popBackStack() }) }
        composable(RcRoutes.AWT) { AwtScreen(onBack = { navController.popBackStack() }) }
        composable(RcRoutes.INSTALL) { InstallWizardScreen(navController) }
        composable(
            RcRoutes.INSTANCE_DETAIL,
            arguments = listOf(navArgument("id") { type = androidx.navigation.NavType.StringType }),
        ) { backStackEntry ->
            InstanceDetailScreen(
                id = backStackEntry.arguments?.getString("id") ?: "",
                navController = navController,
            )
        }
    }
}
