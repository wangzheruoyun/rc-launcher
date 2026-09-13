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
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
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
import com.rc.launcher.ui.screen.ModBrowserScreen
import com.rc.launcher.ui.screen.TranslationSettingsScreen
import com.rc.launcher.ui.screen.HomeScreen
import com.rc.launcher.ui.screen.InstanceDetailScreen
import com.rc.launcher.ui.screen.InstancesScreen
import com.rc.launcher.ui.screen.SettingsScreen
import com.rc.launcher.ui.screen.AccountsScreen
import com.rc.launcher.ui.screen.ControllerScreen
import com.rc.launcher.ui.screen.ControlLayoutLibraryScreen
import com.rc.launcher.ui.screen.AwtScreen
import com.rc.launcher.ui.screen.InstallWizardScreen
import com.rc.launcher.ui.screen.OnboardingScreen
import com.rc.launcher.ui.screen.FileManagerScreen
import com.rc.launcher.ui.screen.WorldManagerScreen
import com.rc.launcher.ui.screen.ResourcePackManagerScreen
import com.rc.launcher.ui.screen.ShaderPackManagerScreen
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

/** Task 15: community / built-in control-layout library. */
@Serializable data object ControlLayoutLibraryRoute

@Serializable data object AwtRoute

@Serializable data object InstallRoute

/** Task 17: modpack import pipeline (CurseForge / Modrinth / MultiMC). */
@Serializable data object ModpackImportRoute

/** Task 14: first-run / rewatch onboarding flow. */
@Serializable data object OnboardingRoute

@Serializable data object TranslationRoute

@Serializable data class InstanceDetailRoute(val id: String)

/**
 * Task 19: in-app small file manager. Carries the optional
 * [instanceId] (so the screen can resolve the per-instance
 * `effectiveGameDir`) and an optional [subdir] (so a deep-link like
 * "open the saves folder" can land directly inside it).
 */
@Serializable data class FileManagerRoute(
    val instanceId: String? = null,
    val subdir: String? = null,
)

/**
 * Task 26: world / save archive management screen.
 * Carries the instance id so the screen can resolve the per-instance
 *  directory through the version-isolation strategy.
 */
@Serializable data class WorldManagerRoute(val instanceId: String)

/**
 * Task 27: resource-pack browsing & management screen.
 * Lists resource packs in the per-instance  directory.
 */
@Serializable data class ResourcePackManagerRoute(val instanceId: String)

/**
 * Task 27: shader-pack browsing & management screen.
 * Lists shader packs in the per-instance  directory.
 */
@Serializable data class ShaderPackManagerRoute(val instanceId: String)

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
    const val ONBOARDING = "onboarding"
    const val INSTANCE_DETAIL = "instance/{id}"
    const val FILE_MANAGER = "file-manager"
    const val WORLD_MANAGER = "world-manager"
    const val RESOURCE_PACK_MANAGER = "resource-pack-manager"
    const val SHADER_PACK_MANAGER = "shader-pack-manager"

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

/**
 * Side navigation rail — the landscape / tablet counterpart of
 * [RcBottomNavigationBar] (task 9).
 *
 * A bottom bar in landscape is the worst of both worlds: it eats the scarce
 * vertical space *and* sits exactly under the thumbs that hold the device. The
 * rail moves navigation to the leading edge instead, which is also where the
 * Material 3 adaptive guidance puts it for medium/expanded widths.
 *
 * [showLabels] is driven by the window height: on a short landscape phone the
 * five labelled items would not fit, so the icons go label-less (the label is
 * still the icon's `contentDescription`, so TalkBack is unaffected).
 */
@Composable
fun RcNavigationRail(
    destinations: List<TopLevelDestination>,
    currentRoute: Any?,
    showLabels: Boolean = true,
    onNavigate: (TopLevelDestination) -> Unit,
) {
    NavigationRail {
        for (dest in destinations) {
            val selected = currentRoute == dest.route
            // Task 20: resolved per recomposition, so a language switch relabels
            // the rail immediately (no Activity recreation).
            val label = rcString(dest.labelKey)
            NavigationRailItem(
                selected = selected,
                onClick = { onNavigate(dest) },
                icon = {
                    Icon(
                        imageVector = if (selected) dest.selectedIcon else dest.icon,
                        contentDescription = label,
                    )
                },
                label = if (showLabels) {
                    { Text(label) }
                } else {
                    null
                },
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
        composable<ControllerRoute> {
            ControllerScreen(
                onBack = { navController.popBackStack() },
                onOpenLibrary = { navController.navigate(ControlLayoutLibraryRoute) },
            )
        }
        composable<ControlLayoutLibraryRoute> {
            ControlLayoutLibraryScreen(onBack = { navController.popBackStack() })
        }
        composable<AwtRoute> { AwtScreen(onBack = { navController.popBackStack() }) }
        composable<InstallRoute> { InstallWizardScreen(navController) }
        composable<ModpackImportRoute> { ModpackImportScreen(navController) }
        // Task 14: first-run / rewatch onboarding. Pop back to home on finish
        // so the bottom-bar layout stays intact; navigating "forward" would
        // leave the onboarding on top of the home screen.
        composable<OnboardingRoute> {
            OnboardingScreen(onFinished = {
                navController.popBackStack(route = HomeRoute, inclusive = false)
            })
        }
        composable<TranslationRoute> { TranslationSettingsScreen(navController = navController) }
        composable<InstanceDetailRoute> { backStackEntry ->
            val route = backStackEntry.toRoute<InstanceDetailRoute>()
            InstanceDetailScreen(
                id = route.id,
                navController = navController,
            )
        }
        // Task 19: in-app small file manager. Both [instanceId] and
        // [subdir] are optional: omitting them opens the default game
        // root; supplying them lands the user on the per-instance
        // effective game dir (and, optionally, on a sub-directory like
        // "saves" / "mods" / "resourcepacks" / "shaderpacks").
        composable<FileManagerRoute> { backStackEntry ->
            val route = backStackEntry.toRoute<FileManagerRoute>()
            FileManagerScreen(
                navController = navController,
                instanceId = route.instanceId,
                initialSubdir = route.subdir,
            )
        }
        // Task 26: world / save archive management.
        composable<WorldManagerRoute> { backStackEntry ->
            val route = backStackEntry.toRoute<WorldManagerRoute>()
            WorldManagerScreen(
                instanceId = route.instanceId,
                navController = navController,
            )
        }
        // Task 27: resource-pack management.
        composable<ResourcePackManagerRoute> { backStackEntry ->
            val route = backStackEntry.toRoute<ResourcePackManagerRoute>()
            ResourcePackManagerScreen(
                instanceId = route.instanceId,
                navController = navController,
            )
        }
        // Task 27: shader-pack management.
        composable<ShaderPackManagerRoute> { backStackEntry ->
            val route = backStackEntry.toRoute<ShaderPackManagerRoute>()
            ShaderPackManagerScreen(
                instanceId = route.instanceId,
                navController = navController,
            )
        }
    }
}
