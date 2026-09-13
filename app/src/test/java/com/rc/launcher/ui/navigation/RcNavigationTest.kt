package com.rc.launcher.ui.navigation

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pure-logic unit tests for the navigation model of task 11 (no composition, so
 * they run on the plain JVM unit-test runner and feed the task-21 CI gate).
 *
 * They lock the invariants that keep the bottom-navigation labels and the
 * [NavHost] graph from drifting apart — the whole point of centralising every
 * route in [RcRoutes] instead of hard-coding strings at each call site.
 */
class RcNavigationTest {

    @Test
    fun routes_areNonEmptyAndDistinct() {
        val routes = listOf(
            RcRoutes.HOME,
            RcRoutes.INSTANCES,
            RcRoutes.DOWNLOADS,
            RcRoutes.SETTINGS,
            RcRoutes.ACCOUNTS,
            RcRoutes.CONTROLLER,
            RcRoutes.AWT,
            RcRoutes.INSTALL,
            RcRoutes.ONBOARDING,
            RcRoutes.INSTANCE_DETAIL,
            RcRoutes.FILE_MANAGER,
        )
        assertTrue("routes must not be empty", routes.isNotEmpty())
        routes.forEach { assertFalse("route must not be blank: [$it]", it.isBlank()) }
        assertEquals("route constants must be unique", routes.size, routes.toSet().size)
    }

    @Test
    fun instanceDetail_buildsConcreteRoute() {
        val route = RcRoutes.instanceDetail("abc-123")
        assertEquals("instance/abc-123", route)
        assertTrue("must be under the instance/ segment", route.startsWith("instance/"))
        // The {id} template token must be fully replaced by the concrete id.
        assertFalse("template token must be substituted", route.contains("{id}"))
    }

    @Test
    /**
     * Regression guard for the type-safe route migration (task 11, 2026.08 line):
     * every top-level destination is represented by a distinct `@Serializable`
     * route object, and the instance-detail route carries its `id` argument in
     * the type — so navigation can never lose or mis-parse the parameter the way
     * a hand-built "instance/$id" string could.
     */
    @Test
    fun typeSafeRoutes_areDistinctAndCarryArgs() {
        val routes = listOf(
            HomeRoute, InstancesRoute, DownloadsRoute, SettingsRoute,
            AccountsRoute, ControllerRoute, AwtRoute, InstallRoute,
            ControlLayoutLibraryRoute, ModpackImportRoute, OnboardingRoute,
            TranslationRoute,
        )
        assertEquals("type-safe top-level routes must be unique", routes.size, routes.toSet().size)

        // Task 27: the resource-pack / shader-pack management routes each carry
        // a required instanceId so the screen can resolve the per-instance
        // `resourcepacks/` / `shaderpacks/` directory.
        val rp = ResourcePackManagerRoute(instanceId = "fabric-1.20.1")
        assertEquals("fabric-1.20.1", rp.instanceId)
        assertNotEquals(
            ResourcePackManagerRoute(instanceId = "a"),
            ResourcePackManagerRoute(instanceId = "b"),
        )

        val sp = ShaderPackManagerRoute(instanceId = "fabric-1.20.1")
        assertEquals("fabric-1.20.1", sp.instanceId)
        assertNotEquals(
            ShaderPackManagerRoute(instanceId = "a"),
            ShaderPackManagerRoute(instanceId = "b"),
        )

        // Task 26: the world-manager route also carries a required instanceId.
        val wm = WorldManagerRoute(instanceId = "fabric-1.20.1")
        assertEquals("fabric-1.20.1", wm.instanceId)

        val detail = InstanceDetailRoute("abc-123")
        assertEquals("abc-123", detail.id)
        assertNotEquals(InstanceDetailRoute("x"), InstanceDetailRoute("y"))

        // Task 19: the in-app small file manager is a single route that
        // carries two optional arguments. Both must default to null so
        // "open the game root" is just `navigate(FileManagerRoute())`.
        val defaultManager = FileManagerRoute()
        assertEquals(null, defaultManager.instanceId)
        assertEquals(null, defaultManager.subdir)

        val instanceManager = FileManagerRoute(instanceId = "fabric-1.20.1", subdir = "saves")
        assertEquals("fabric-1.20.1", instanceManager.instanceId)
        assertEquals("saves", instanceManager.subdir)
    }

    @Test
    fun instanceDetail_preservesIdSegment() {
        // An id containing a slash still stays a single logical segment here; the
        // NavHost's StringType argument absorbs it, and the builder must not
        // silently drop or re-escape it.
        val route = RcRoutes.instanceDetail("a/b")
        assertEquals("instance/a/b", route)
        assertTrue(route.startsWith("instance/"))
    }
}
