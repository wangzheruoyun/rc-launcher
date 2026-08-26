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
import androidx.compose.ui.Modifier
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.toRoute
import kotlinx.serialization.Serializable
import com.rc.launcher.ui.screen.DownloadsScreen
import com.rc.launcher.ui.screen.HomeScreen
import com.rc.launcher.ui.screen.InstanceDetailScreen
import com.rc.launcher.ui.screen.InstancesScreen
import com.rc.launcher.ui.screen.SettingsScreen
import com.rc.launcher.ui.screen.AccountsScreen
import com.rc.launcher.ui.screen.ControllerScreen
import com.rc.launcher.ui.screen.AwtScreen
import com.rc.launcher.ui.screen.InstallWizardScreen
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString

/**
 * Type-safe navigation routes (Navigation Compose 2.9.x). Every destination is a
 * `@Serializable` class, so the [NavHost] registers it with `composable<T>()` and
 * call sites navigate with `navController.navigate(HomeRoute)` instead of
 * concatenating strings. This removes the entire class of route-typo / mismatch
 * bugs and is the modern replacement for the string-based [RcRoutes] table.
 *
 * `InstanceDetailRoute` carries its `id` argument directly in the type, so the
 * parameter can never be lost or mis-parsed.
 */
@Serializable data object HomeRoute

@Serializable data object InstancesRoute

@Serializable data object DownloadsRoute

@Serializable data object SettingsRoute

@Serializable data object AccountsRoute

@Serializable data object ControllerRoute

@Serializable data object AwtRoute

@Serializable data object InstallRoute

@Serializable data class InstanceDetailRoute(val id: String)

/**
 * Legacy string-route table. Kept as the single source of truth for the
 * human-readable route segments and asserted by [RcNavigationTest]; new code
 * should navigate with the type-safe [HomeRoute] family defined above.
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

    /**
     * Builds the concrete serialised route string for the instance-detail screen
     * (legacy helper, asserted by [RcNavigationTest]). Prefer the type-safe
     * [InstanceDetailRoute] object for new navigation call sites.
     */
    fun instanceDetail(id: String): String = "instance/$id"
}

/**
 * A top-level destination shown in the bottom navigation bar. Mirrors the
 * single-Activity / multi-destination organisation of FCL's `fcl/` UI module.
 *
 * The destination carries a **type-safe route object** ([route]) used both for
 * navigation and for selection comparison, plus an **i18n key** ([labelKey])
 * resolved at render time from [LocalRcStrings] (task 20): switching the
 * language re-labels the navigation bar without rebuilding the destination list.
 */
data class TopLevelDestination(
    val route: Any,
    val labelKey: String,
    val icon: ImageVector,
    val selectedIcon: ImageVector = icon,
)

/**
 * Canonical bottom-navigation destinations for the launcher. Screens are
 * realised in their own tasks (12 home/instances, 14 settings, ...); here we only
 * define the navigation shell and placeholder screens.
 */
val RcTopLevelDestinations: List<TopLevelDestination> = listOf(
    TopLevelDestination(HomeRoute, RcStringKeys.NAV_HOME, Icons.Outlined.Home, Icons.Filled.Home),
    TopLevelDestination(InstancesRoute, RcStringKeys.NAV_INSTANCES, Icons.Outlined.Storage, Icons.Filled.Storage),
    TopLevelDestination(DownloadsRoute, RcStringKeys.NAV_DOWNLOADS, Icons.Outlined.Download, Icons.Filled.Download),
    TopLevelDestination(SettingsRoute, RcStringKeys.NAV_SETTINGS, Icons.Outlined.Settings, Icons.Filled.Settings),
    TopLevelDestination(AccountsRoute, RcStringKeys.NAV_ACCOUNTS, Icons.Outlined.Person, Icons.Filled.Person),
)

@Composable
fun RcBottomNavigationBar(
    destinations: List<TopLevelDestination>,
    currentRoute: Any?,
    onNavigate: (TopLevelDestination) -> Unit,
) {
    NavigationBar {
        for (dest in destinations) {
            // `route` is the singleton for data-object routes, so identity
            // comparison cleanly detects the selected destination.
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
    modifier: Modifier = Modifier,
) {
    NavHost(
        navController = navController,
        // Type-safe start destination (replaces the string-based RcRoutes.HOME).
        startDestination = HomeRoute,
        modifier = modifier,
    ) {
        composable<HomeRoute> { HomeScreen(navController) }
        composable<InstancesRoute> { InstancesScreen(navController) }
        composable<DownloadsRoute> { DownloadsScreen() }
        composable<SettingsRoute> { SettingsScreen(navController = navController) }
        composable<AccountsRoute> { AccountsScreen() }
        composable<ControllerRoute> { ControllerScreen(onBack = { navController.popBackStack() }) }
        composable<AwtRoute> { AwtScreen(onBack = { navController.popBackStack() }) }
        composable<InstallRoute> { InstallWizardScreen(navController) }
        composable<InstanceDetailRoute> { backStackEntry ->
            val route = backStackEntry.toRoute<InstanceDetailRoute>()
            InstanceDetailScreen(
                id = route.id,
                navController = navController,
            )
        }
    }
}
