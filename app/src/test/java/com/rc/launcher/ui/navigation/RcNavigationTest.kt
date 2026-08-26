package com.rc.launcher.ui.navigation

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
            RcRoutes.INSTANCE_DETAIL,
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
        )
        assertEquals("type-safe top-level routes must be unique", routes.size, routes.toSet().size)

        val detail = InstanceDetailRoute("abc-123")
        assertEquals("abc-123", detail.id)
        assertNotEquals(InstanceDetailRoute("x"), InstanceDetailRoute("y"))
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
