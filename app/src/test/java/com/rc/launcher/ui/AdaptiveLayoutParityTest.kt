package com.rc.launcher.ui

import com.rc.launcher.ui.model.OrientationMode
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Proves the Kotlin adaptive-layout mirror agrees with the Rust core (task 9).
 *
 * [RcWindowInfo] is a hand-written port of `rust/.../src/display.rs`. Compose
 * resolves the layout locally — a rotating device must not pay a JNI crossing per
 * recomposition — so the two implementations must decide *identically*, otherwise
 * the core's diagnostics ("landscape, rail, 3 columns") and the drawn UI would
 * disagree.
 *
 * The Rust side is therefore the **oracle**: `cargo run --example
 * display_layout_golden -- --write` renders a device matrix (real phones,
 * foldables, tablets, every breakpoint edge, both orientations, degenerate sizes)
 * into `src/test/resources/display_layout_golden.tsv` plus the orientation policy
 * table into `display_orientation_golden.tsv`, and this test replays both.
 * `scripts/check_layout_parity.py` re-runs the generator so a stale fixture also
 * fails CI.
 */
class AdaptiveLayoutParityTest {

    private val repoRoot: File? by lazy {
        var dir: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (dir != null) {
            if (File(dir, "rust/crates/rc-launcher-core/src/display.rs").exists()) {
                return@lazy dir
            }
            dir = dir.parentFile
        }
        null
    }

    private fun fixture(name: String): List<String>? {
        // Gradle puts `src/test/resources` on the classpath; fall back to the
        // repository path so an IDE run configuration works too.
        javaClass.getResourceAsStream("/$name")?.use { stream ->
            return stream.bufferedReader(Charsets.UTF_8).readLines()
        }
        val root = repoRoot ?: return null
        val f = File(root, "app/src/test/resources/$name")
        return if (f.exists()) f.readLines(Charsets.UTF_8) else null
    }

    private fun rows(name: String, fields: Int): List<List<String>> =
        (fixture(name) ?: emptyList())
            .filter { it.isNotBlank() && !it.startsWith("#") }
            .map { line ->
                val f = line.split('\t')
                require(f.size == fields) { "malformed golden row in $name: $line" }
                f
            }

    @Test
    fun everyWindowGeometryResolvesLikeTheCore() {
        val cases = rows("display_layout_golden.tsv", 13)
        assumeTrue("golden fixture not on the classpath", cases.isNotEmpty())
        for (f in cases) {
            val w = f[0].toInt()
            val h = f[1].toInt()
            val m = RcWindowInfo.of(w, h)
            val where = "${w}x$h"
            // The sanitising factory must not move a geometry the core accepted.
            assertEquals("$where width_dp", w, m.widthDp)
            assertEquals("$where height_dp", h, m.heightDp)
            assertEquals("$where orientation", f[2], m.orientation.id)
            assertEquals("$where width_class", f[3], m.widthClass.id)
            assertEquals("$where height_class", f[4], m.heightClass.id)
            assertEquals("$where landscape", f[5].toBoolean(), m.isLandscape)
            assertEquals("$where short", f[6].toBoolean(), m.isShort)
            assertEquals("$where navigation_rail", f[7].toBoolean(), m.usesNavigationRail)
            assertEquals("$where instance_columns", f[8].toInt(), m.instanceColumns)
            assertEquals("$where settings_columns", f[9].toInt(), m.settingsColumns)
            assertEquals("$where dashboard_columns", f[10].toInt(), m.dashboardColumns)
            assertEquals("$where content_padding_dp", f[11].toInt(), m.contentPaddingDp)
            assertEquals("$where max_content_width_dp", f[12].toInt(), m.maxContentWidthDp)
        }
        // The matrix must actually contain rotated pairs, otherwise it would not
        // be testing the thing task 9 is about.
        val pairs = cases.map { it[0].toInt() to it[1].toInt() }.toSet()
        assertTrue(pairs.contains(392 to 872))
        assertTrue(pairs.contains(872 to 392))
    }

    @Test
    fun theOrientationPolicyTableMatchesTheCore() {
        val cases = rows("display_orientation_golden.tsv", 7)
        assumeTrue("golden fixture not on the classpath", cases.isNotEmpty())
        for (f in cases) {
            val mode = OrientationMode.fromId(f[0])
            assertEquals("unknown policy id in the fixture: ${f[0]}", f[0], mode.id)
            assertEquals("${f[0]} android", f[1], mode.androidScreenOrientation)
            assertEquals("${f[0]} forced", f[2] != "-", mode.isForced)

            // The launcher UI never swaps the game window itself (the core does,
            // through `OrientationPolicy::orient`), but the *rule* is mirrored here
            // so a change to it cannot land on one side only.
            val inW = f[3].toInt()
            val inH = f[4].toInt()
            val expected = f[5].toInt() to f[6].toInt()
            assertEquals("${f[0]} orient ${inW}x$inH", expected, orient(mode, inW, inH))
        }
        // Every mode the UI offers must appear in the core's table.
        val ids = cases.map { it[0] }.toSet()
        assertEquals(OrientationMode.entries.map { it.id }.toSet(), ids)
    }

    /** Kotlin mirror of `display::OrientationPolicy::orient`. */
    private fun orient(mode: OrientationMode, width: Int, height: Int): Pair<Int, Int> {
        val swap = when (mode) {
            OrientationMode.FOLLOW_SYSTEM -> false
            OrientationMode.LANDSCAPE -> height > width
            OrientationMode.PORTRAIT -> width > height
        }
        return if (swap) height to width else width to height
    }
}
