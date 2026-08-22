package com.rc.launcher.ui.resource

import android.app.ActivityManager
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.delay
import java.io.BufferedReader
import java.io.FileReader

/**
 * Pure aggregation helpers for device resource usage. Kept free of any Android
 * import so the percentage / CPU-delta maths is unit-testable on the JVM (task 21).
 */
object ResourceMath {
    /** Percentage in [0, 100]; 0 when [total] is unknown/zero. */
    fun percent(used: Long, total: Long): Float =
        if (total <= 0) 0f else ((used.toDouble() / total) * 100.0).coerceIn(0.0, 100.0).toFloat()

    /**
     * CPU utilisation in [0, 100] from two `/proc/stat` snapshots. `idle` is the
     * idle+iowait jiffies, `total` the sum of all jiffies; the delta over the
     * sampling window yields the busy fraction. Returns 0 when no time elapsed.
     */
    fun cpuPercent(prevIdle: Long, prevTotal: Long, curIdle: Long, curTotal: Long): Float {
        val totalDelta = (curTotal - prevTotal).toDouble()
        val idleDelta = (curIdle - prevIdle).toDouble()
        if (totalDelta <= 0.0) return 0f
        val usage = (1.0 - idleDelta / totalDelta).coerceIn(0.0, 1.0)
        return (usage * 100.0).toFloat().coerceIn(0f, 100f)
    }

    /** Parse the aggregate "cpu" line of `/proc/stat` into (idleJiffies, totalJiffies). */
    fun parseCpuLine(line: String): Pair<Long, Long> {
        // "cpu  user nice system idle iowait irq softirq steal guest guest_nice"
        val parts = line.split(Regex("\\s+")).drop(1).mapNotNull { it.toLongOrNull() }
        if (parts.isEmpty()) return 0L to 0L
        val total = parts.sum()
        val idle = parts.getOrElse(3) { 0L } + parts.getOrElse(4) { 0L }
        return idle to total
    }

    /** Read idle/total jiffies from `/proc/stat`; (0, 0) on any failure. */
    fun readCpu(): Pair<Long, Long> = try {
        val line = BufferedReader(FileReader("/proc/stat")).use { it.readLine() }
        if (line != null && line.startsWith("cpu ")) parseCpuLine(line) else (0L to 0L)
    } catch (_: Throwable) {
        0L to 0L
    }
}

/** Snapshot of device resource usage shown on the dashboard and the floating HUD. */
data class ResourceUsage(
    val usedMemBytes: Long = 0,
    val totalMemBytes: Long = 0,
    val usedStorageBytes: Long = 0,
    val totalStorageBytes: Long = 0,
    val cpuPercent: Double = 0.0,
    val timestamp: Long = 0,
) {
    val memPercent: Float get() = ResourceMath.percent(usedMemBytes, totalMemBytes)
    val storagePercent: Float get() = ResourceMath.percent(usedStorageBytes, totalStorageBytes)
    val isUnknown: Boolean get() = totalMemBytes <= 0L && totalStorageBytes <= 0L

    companion object {
        fun unknown(): ResourceUsage = ResourceUsage()
    }
}

/** Human-readable byte size, e.g. "1.4 GB". */
internal fun formatBytes(bytes: Long): String {
    if (bytes <= 0) return "0 B"
    val units = arrayOf("B", "KB", "MB", "GB", "TB")
    var v = bytes.toDouble()
    var i = 0
    while (v >= 1024 && i < units.lastIndex) {
        v /= 1024
        i++
    }
    return "%.1f %s".format(v, units[i])
}

/**
 * Samples real device resource usage (task 12 "资源占用").
 *
 * Memory comes from [ActivityManager.MemoryInfo], storage from [android.os.StatFs]
 * on the app data directory, and CPU from the delta of two `/proc/stat` reads.
 * Every platform call is wrapped so the dashboard degrades to
 * [ResourceUsage.unknown] instead of crashing on locked-down or preview contexts.
 */
class ResourceMonitor(context: Context) {
    private val appContext = context.applicationContext
    private var cpuPrev: Pair<Long, Long>? = null

    fun sample(): ResourceUsage = try {
        val am = appContext.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager
        val mem = ActivityManager.MemoryInfo()
        am?.getMemoryInfo(mem)
        val usedMem = (mem.totalMem - mem.availMem).coerceAtLeast(0L)
        val totalMem = mem.totalMem

        val stat = android.os.StatFs(appContext.filesDir.absolutePath)
        val blockSize = stat.blockSizeLong
        val totalBlocks = stat.blockCountLong
        val availBlocks = stat.availableBlocksLong
        val totalStore = totalBlocks * blockSize
        val usedStore = (totalBlocks - availBlocks) * blockSize

        val (idle, total) = ResourceMath.readCpu()
        val cpu = cpuPrev?.let { (pIdle, pTotal) ->
            ResourceMath.cpuPercent(pIdle, pTotal, idle, total)
        } ?: 0f
        cpuPrev = idle to total

        ResourceUsage(
            usedMemBytes = usedMem,
            totalMemBytes = totalMem,
            usedStorageBytes = usedStore,
            totalStorageBytes = totalStore,
            cpuPercent = cpu.toDouble(),
            timestamp = System.currentTimeMillis(),
        )
    } catch (_: Throwable) {
        ResourceUsage.unknown()
    }
}

/**
 * Compose state holding the latest [ResourceUsage], refreshed every second on a
 * background coroutine. Safe in previews (effects never run there) and in
 * headless contexts (the monitor returns [ResourceUsage.unknown] on failure).
 */
@Composable
fun rememberResourceUsage(): State<ResourceUsage> {
    val context = LocalContext.current
    val monitor = remember(context) { ResourceMonitor(context) }
    val state = remember { mutableStateOf(ResourceUsage.unknown()) }
    LaunchedEffect(monitor) {
        while (true) {
            state.value = monitor.sample()
            delay(1000)
        }
    }
    return state
}
