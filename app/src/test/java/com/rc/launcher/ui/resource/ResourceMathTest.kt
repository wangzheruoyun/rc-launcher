package com.rc.launcher.ui.resource

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the pure resource-aggregation maths (task 12 / task 21). */
class ResourceMathTest {

    @Test
    fun percent_isZeroForUnknownTotal() {
        assertEquals(0f, ResourceMath.percent(used = 500L, total = 0L))
    }

    @Test
    fun percent_scalesAndClamps() {
        assertEquals(50f, ResourceMath.percent(used = 1L, total = 2L))
        assertEquals(100f, ResourceMath.percent(used = 9L, total = 2L))
        assertEquals(0f, ResourceMath.percent(used = -1L, total = 2L))
    }

    @Test
    fun parseCpuLine_sumsAllAndCountsIdlePlusIowait() {
        // user nice system idle iowait irq softirq steal guest guest_nice
        val (idle, total) = ResourceMath.parseCpuLine("cpu 100 20 30 400 50 5 5 0 0 0")
        // total = 100+20+30+400+50+5+5 = 610
        assertEquals(610L, total)
        // idle = idle(400) + iowait(50) = 450
        assertEquals(450L, idle)
    }

    @Test
    fun parseCpuLine_handlesShortLines() {
        val (idle, total) = ResourceMath.parseCpuLine("cpu 10 0 0 5")
        assertEquals(15L, total)
        assertEquals(5L, idle)
    }

    @Test
    fun cpuPercent_zeroWhenNoElapsedTime() {
        assertEquals(0f, ResourceMath.cpuPercent(0L, 0L, 0L, 0L))
        assertEquals(0f, ResourceMath.cpuPercent(100L, 200L, 100L, 200L)) // no delta
    }

    @Test
    fun cpuPercent_halfBusyIsFiftyPercent() {
        // delta total = 1000, delta idle = 500 → 50% busy
        val p = ResourceMath.cpuPercent(prevIdle = 0L, prevTotal = 0L, curIdle = 500L, curTotal = 1000L)
        assertEquals(50f, p)
    }

    @Test
    fun cpuPercent_fullyBusyIsHundred() {
        val p = ResourceMath.cpuPercent(prevIdle = 0L, prevTotal = 0L, curIdle = 0L, curTotal = 1000L)
        assertEquals(100f, p)
    }

    @Test
    fun formatBytes_producesHumanReadableSizes() {
        assertEquals("0 B", formatBytes(0L))
        assertEquals("1.0 KB", formatBytes(1024L))
        assertEquals("1.0 MB", formatBytes(1024L * 1024L))
        assertTrue(formatBytes(3L * 1024 * 1024 * 1024).startsWith("3.0 G"))
    }
}
