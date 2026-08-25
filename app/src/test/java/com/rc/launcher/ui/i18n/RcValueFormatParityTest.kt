package com.rc.launcher.ui.i18n

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test

/**
 * Proves the Kotlin value formatter is byte-identical to the Rust core (task 20).
 *
 * [RcValueFormat] is a hand-written port of `rust/.../src/i18n/number.rs`.
 * Compose uses the port rather than a JNI call per label — the catalogue bundle
 * already contains the format skeletons, so a scrolling download list costs no
 * FFI crossings. The price of that design is that the two implementations must
 * agree *exactly*, or the progress row and the launcher log would disagree about
 * the same byte count.
 *
 * So the Rust side is the **oracle**: `cargo run --example i18n_format_golden --
 * --write` renders a 650-case matrix (unit boundaries, rounding ties, zero and
 * negative values, unit promotion, singular/plural) into
 * `src/test/resources/i18n_format_golden.tsv`, and this test replays it.
 * `scripts/check_i18n.py` re-runs the generator so a stale fixture also fails CI.
 */
class RcValueFormatParityTest {

    private data class Case(
        val language: String,
        val kind: String,
        val value: String,
        val total: String,
        val digits: String,
        val parts: String,
        val expected: String,
    )

    private val repoRoot: File? by lazy {
        var dir: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (dir != null) {
            if (File(dir, "rust/crates/rc-launcher-core/i18n/zh-CN.properties").exists()) {
                return@lazy dir
            }
            dir = dir.parentFile
        }
        null
    }

    /**
     * `.properties` reader mirroring the Rust/Python parsers, including the
     * escapes — `\u0020` is the only way to express a significant space, and a
     * separator is exactly the kind of value that needs it.
     */
    private fun readProperties(file: File): Map<String, String> {
        val out = LinkedHashMap<String, String>()
        var logical = StringBuilder()
        var pending = false
        file.readLines(Charsets.UTF_8).forEach { raw ->
            val line = raw.removeSuffix("\r")
            if (!pending) {
                val t = line.trimStart().removePrefix("\uFEFF")
                if (t.isEmpty() || t.startsWith("#") || t.startsWith("!")) return@forEach
                logical = StringBuilder(t)
            } else {
                logical.append(line.trimStart())
            }
            val text0 = logical.toString()
            if ((text0.length - text0.trimEnd('\\').length) % 2 == 1) {
                logical.setLength(logical.length - 1)
                pending = true
                return@forEach
            }
            pending = false
            val text = logical.toString()
            var idx = -1
            var escaped = false
            for ((i, c) in text.withIndex()) {
                if (escaped) { escaped = false; continue }
                if (c == '\\') escaped = true else if (c == '=') { idx = i; break }
            }
            if (idx < 0) return@forEach
            out[unescape(text.substring(0, idx).trim())] = unescape(text.substring(idx + 1).trim())
        }
        return out
    }

    private fun unescape(s: String): String {
        if ('\\' !in s) return s
        val out = StringBuilder(s.length)
        var i = 0
        while (i < s.length) {
            val c = s[i]
            if (c != '\\') { out.append(c); i++; continue }
            i++
            if (i >= s.length) { out.append('\\'); break }
            when (val n = s[i]) {
                'n' -> out.append('\n')
                'r' -> out.append('\r')
                't' -> out.append('\t')
                'f' -> out.append('\u000C')
                'u' -> {
                    val hex = s.substring((i + 1).coerceAtMost(s.length), (i + 5).coerceAtMost(s.length))
                    if (hex.length == 4 && hex.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }) {
                        out.append(hex.toInt(16).toChar())
                        i += 4
                    } else {
                        out.append("\\u")
                    }
                }
                else -> out.append(n)
            }
            i++
        }
        return out.toString()
    }

    private fun table(tag: String): RcStrings? {
        val root = repoRoot ?: return null
        val f = File(root, "rust/crates/rc-launcher-core/i18n/$tag.properties")
        if (!f.exists()) return null
        // `fromTag` never returns null (unknown -> SYSTEM), and every tag in the
        // fixture is a shipped catalogue, so this is always a concrete language.
        val language = AppLanguage.fromTag(tag)
        return RcStrings.of(language, readProperties(f))
    }

    private fun goldenLines(): List<String>? {
        // Gradle puts `src/test/resources` on the classpath; fall back to the
        // repository path so an IDE run configuration works too.
        javaClass.getResourceAsStream("/i18n_format_golden.tsv")?.use { stream ->
            return stream.bufferedReader(Charsets.UTF_8).readLines()
        }
        val root = repoRoot ?: return null
        val f = File(root, "app/src/test/resources/i18n_format_golden.tsv")
        return if (f.exists()) f.readLines(Charsets.UTF_8) else null
    }

    private fun cases(): List<Case> = (goldenLines() ?: emptyList())
        .filter { it.isNotBlank() && !it.startsWith("#") }
        .map { line ->
            val f = line.split('\t')
            require(f.size == 7) { "malformed golden row: $line" }
            Case(f[0], f[1], f[2], f[3], f[4], f[5], f[6])
        }

    /** Dispatch a golden row through the Kotlin port. */
    private fun render(strings: RcStrings, c: Case): String {
        val long = { s: String -> s.toLongOrNull() ?: 0L }
        val double = { s: String -> s.toDoubleOrNull() ?: 0.0 }
        val digits = c.digits.toIntOrNull() ?: 1
        return when (c.kind) {
            "bytes" -> RcValueFormat.bytes(strings, long(c.value))
            "rate" -> RcValueFormat.rate(strings, long(c.value))
            "byte_progress" -> RcValueFormat.byteProgress(strings, long(c.value), long(c.total))
            "int" -> RcValueFormat.int(strings, long(c.value))
            "decimal" -> RcValueFormat.decimal(strings, double(c.value), digits)
            "percent" -> RcValueFormat.percent(strings, double(c.value), digits)
            "ratio" -> RcValueFormat.ratioPercent(strings, long(c.value), long(c.total), digits)
            "duration" -> c.parts.toIntOrNull()
                ?.let { RcValueFormat.duration(strings, long(c.value), it) }
                ?: RcValueFormat.duration(strings, long(c.value))
            "eta" -> RcValueFormat.eta(strings, long(c.value))
            "relative" -> RcValueFormat.relativeTime(strings, long(c.value))
            "fps" -> RcValueFormat.fps(strings, double(c.value))
            else -> error("unknown golden kind: ${c.kind}")
        }
    }

    @Test
    fun theGoldenFixtureIsPresentAndSubstantial() {
        assumeTrue("repository layout not found", repoRoot != null)
        val cases = cases()
        assertTrue(
            "golden fixture missing or tiny (regenerate: " +
                "cargo run --example i18n_format_golden -- --write) — got ${cases.size}",
            cases.size >= 300,
        )
        // Every kind the port implements must be represented.
        val kinds = cases.map { it.kind }.toSet()
        for (kind in listOf(
            "bytes", "rate", "byte_progress", "int", "decimal", "percent",
            "ratio", "duration", "eta", "relative", "fps",
        )) {
            assertTrue("golden fixture never exercises `$kind`", kind in kinds)
        }
        // And every shipped language.
        assertEquals(setOf("zh-CN", "zh-Hant", "en"), cases.map { it.language }.toSet())
    }

    @Test
    fun everyGoldenCaseMatchesTheRustCore() {
        assumeTrue("repository layout not found", repoRoot != null)
        val cases = cases()
        assumeTrue("golden fixture not found", cases.isNotEmpty())

        // Load each catalogue once, up front: a missing one skips the test rather
        // than reporting thousands of bogus mismatches.
        val tables = HashMap<String, RcStrings>()
        for (tag in cases.map { it.language }.toSet()) {
            val loaded = table(tag)
            assumeTrue("catalogue $tag not found", loaded != null)
            tables[tag] = loaded!!
        }

        val mismatches = ArrayList<String>()
        for (c in cases) {
            val strings = tables.getValue(c.language)
            val actual = render(strings, c)
            if (actual != c.expected) {
                mismatches.add(
                    "${c.language} ${c.kind} value=${c.value} total=${c.total} " +
                        "digits=${c.digits} parts=${c.parts}: " +
                        "rust=${c.expected.quoted()} kotlin=${actual.quoted()}",
                )
            }
        }
        assertTrue(
            "Kotlin/Rust formatter drift in ${mismatches.size}/${cases.size} cases:\n" +
                mismatches.take(25).joinToString("\n"),
            mismatches.isEmpty(),
        )
    }

    @Test
    fun theCatalogueShipsEveryKeyTheFormatterReads() {
        assumeTrue(repoRoot != null)
        for (tag in listOf("zh-CN", "zh-Hant", "en")) {
            val strings = table(tag) ?: continue
            val missing = RcValueFormat.requiredKeys().filterNot { strings.has(it) }
            assertTrue("$tag lacks formatter keys: $missing", missing.isEmpty())
        }
    }

    @Test
    fun degradesToAsciiDefaultsWithoutACatalogue() {
        // Task-19 contract: before the engine loads (or if the core is missing)
        // the table is empty and the formatter must still emit digits and units,
        // never a raw `unit.mib` key and never a crash.
        val empty = RcStrings.empty()
        assertEquals("0 B", RcValueFormat.bytes(empty, 0))
        assertEquals("1.5 KB", RcValueFormat.bytes(empty, 1_536))
        assertEquals("1.2 MB/s", RcValueFormat.rate(empty, 1_258_291))
        assertEquals("1,234,567", RcValueFormat.int(empty, 1_234_567))
        assertEquals("42.5%", RcValueFormat.percent(empty, 42.5, 1))
        assertEquals("59.9 FPS", RcValueFormat.fps(empty, 59.94))
        // Durations are the trap: `RcStrings.plural` echoes the key on a miss, so
        // an un-hydrated table used to render
        // "duration.minute.other duration.second.other" on a progress row.
        // DURATION_UNITS carries a compiled-in fallback for exactly this.
        assertEquals("3 min 20 s", RcValueFormat.duration(empty, 200))
        assertEquals("1 h 1 min", RcValueFormat.duration(empty, 3_660))
        assertEquals("1 d 1 h", RcValueFormat.duration(empty, 90_061))
        assertEquals("0 s", RcValueFormat.duration(empty, 0))
        for (out in listOf(
            RcValueFormat.duration(empty, 200),
            RcValueFormat.eta(empty, 200),
            RcValueFormat.relativeTime(empty, 200),
            RcValueFormat.byteProgress(empty, 1, 2),
        )) {
            assertFalse("leaked a key: $out", out.contains("duration."))
            assertFalse("leaked a key: $out", out.contains("relative."))
            assertFalse("leaked a key: $out", out.contains("format."))
            assertFalse("leaked a key: $out", out.contains("unit."))
            assertFalse("unresolved placeholder: $out", out.contains('{'))
            assertTrue(out.isNotBlank())
        }
    }

    @Test
    fun aPartiallyBlankTableStillRendersEveryLabel() {
        // A half-loaded / hand-edited table: blank values must behave like a
        // missing key, not like an empty label.
        val holey = RcStrings.of(
            AppLanguage.EN,
            mapOf(
                "unit.mib" to "",
                "format.size" to "   ",
                "duration.minute.other" to "",
                "format.group_separator" to "",
            ),
        )
        assertEquals("1.0 MB", RcValueFormat.bytes(holey, 1_048_576))
        assertEquals("3 min 20 s", RcValueFormat.duration(holey, 200))
        // A blank grouping separator is honoured (some locales do not group),
        // but it must not blank the digits.
        assertEquals("1234567", RcValueFormat.int(holey, 1_234_567))
    }

    @Test
    fun nonFiniteAndExtremeInputsNeverLeakDeveloperText() {
        val strings = table("zh-CN") ?: RcStrings.empty()
        for (bad in listOf(Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY)) {
            for (out in listOf(
                RcValueFormat.decimal(strings, bad, 2),
                RcValueFormat.percent(strings, bad, 1),
                RcValueFormat.fps(strings, bad),
            )) {
                val lower = out.lowercase()
                assertFalse(out, lower.contains("nan"))
                assertFalse(out, lower.contains("inf"))
                assertTrue(out, out.isNotBlank())
            }
        }
        // Long extremes must not overflow abs().
        assertTrue(RcValueFormat.int(strings, Long.MIN_VALUE).startsWith("-9,223,372,036,854,775,808"))
        assertEquals(
            RcValueFormat.duration(strings, Long.MAX_VALUE),
            RcValueFormat.duration(strings, Long.MIN_VALUE),
        )
        assertTrue(RcValueFormat.relativeTime(strings, Long.MIN_VALUE).isNotBlank())
        // A negative byte count clamps to zero rather than wrapping to 18 EB.
        assertEquals(RcValueFormat.bytes(strings, 0), RcValueFormat.bytes(strings, -1))
    }

    private fun String.quoted(): String = "\"" + this + "\""
}
