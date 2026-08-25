package com.rc.launcher.ui.i18n

import kotlin.math.abs
import kotlin.math.roundToLong

/**
 * Locale-aware value formatting — a faithful port of the Rust core's
 * `crate::i18n::number` (task 20).
 *
 * ## Why a port rather than a JNI call per label
 *
 * [RcStrings] already holds the **whole** catalogue (one `i18nBundle` crossing),
 * and the format *skeletons* are ordinary catalogue keys — `format.size`,
 * `unit.mib`, `duration.minute.other`, … So Compose can assemble "1.4 GB" or
 * "剩余 3 分 20 秒" locally, allocation-cheaply, at 60 fps, and still be
 * byte-for-byte identical to what the core would have produced, because both
 * sides read the same skeletons.
 *
 * `RustBridge.i18nFormat` remains available for non-Compose consumers and is what
 * `RcValueFormatParityTest` cross-checks this port against.
 *
 * ## The invariants both sides share
 *
 *  * rounding is half **away from zero** (`1.25` -> `1.3`), matching
 *    `String.format` — Rust's native `{:.1}` would round half-to-even, so the
 *    core does this explicitly too;
 *  * a non-finite `Double` renders as `format.invalid_number` (`—`), never
 *    `NaN`/`Infinity`;
 *  * a missing or blank catalogue key falls back to a compiled-in ASCII default,
 *    so a broken table still renders digits (task-19 degradation);
 *  * rounding happens *before* the byte unit is chosen, so 1023.97 KiB is
 *    promoted to `1.0 MB` instead of `1024.0 KB`;
 *  * `-0.0` prints as `0`, never `-0`.
 */
object RcValueFormat {

    /** Upper bound on `fractionDigits` (mirrors `number::MAX_FRACTION_DIGITS`). */
    const val MAX_FRACTION_DIGITS = 9

    private const val DEFAULT_GROUP_SEPARATOR = ","
    private const val DEFAULT_DECIMAL_SEPARATOR = "."
    private const val GROUP_SIZE = 3

    /** Byte-unit catalogue keys, smallest first (1024-based). */
    val BYTE_UNIT_KEYS = listOf(
        "unit.byte", "unit.kib", "unit.mib", "unit.gib", "unit.tib", "unit.pib",
    )

    private val BYTE_UNIT_FALLBACK = listOf("B", "KB", "MB", "GB", "TB", "PB")

    /**
     * Duration units, largest first:
     * `(seconds, catalogue plural base key, compiled-in fallback template)`.
     *
     * The fallback matters more than it looks: [RcStrings.plural] echoes the
     * *key* on a miss, which is right for prose but disastrous inside an
     * assembled label — a progress row reading
     * `duration.minute.other duration.second.other` (what an un-hydrated table
     * would produce) is far worse than a terse `3 min 20 s`. Mirrors
     * `DURATION_UNITS` in the Rust core.
     */
    val DURATION_UNITS = listOf(
        Triple(86_400L, "duration.day", "{count} d"),
        Triple(3_600L, "duration.hour", "{count} h"),
        Triple(60L, "duration.minute", "{count} min"),
        Triple(1L, "duration.second", "{count} s"),
    )

    private const val DEFAULT_DURATION_PARTS = 2
    private const val RELATIVE_NOW_THRESHOLD = 60L

    /** Beyond 2^53 a Double has no fractional part left to round. */
    private const val EXACT_INTEGER_LIMIT = 9_007_199_254_740_992.0

    /**
     * A lookup that can never yield blank: unlike [RcStrings.get] it does *not*
     * echo the key, because a stray `unit.mib` inside a progress label would be
     * worse than a plain `MB`.
     */
    private fun RcStrings.skeleton(key: String, fallback: String): String {
        val value = if (has(key)) get(key) else fallback
        return value.ifBlank { fallback }
    }

    /**
     * A lookup where an **explicitly empty** value is meaningful.
     *
     * [skeleton] treats blank as missing, which is right for a unit name or a
     * template (an empty `format.size` would blank the whole label). The
     * *grouping* separator is the one exception: `format.group_separator =` is how
     * a catalogue says "do not group digits at all", so an entry that exists and
     * is empty is honoured rather than silently replaced by `,`. Only an *absent*
     * key falls back. Mirrors `number::group_separator` in the Rust core.
     */
    private fun groupSeparator(strings: RcStrings): String =
        if (strings.has("format.group_separator")) {
            strings["format.group_separator"]
        } else {
            DEFAULT_GROUP_SEPARATOR
        }

    /** Insert [separator] every [GROUP_SIZE] digits, counting from the right. */
    internal fun groupDigits(digits: String, separator: String): String {
        if (separator.isEmpty() || digits.length <= GROUP_SIZE) return digits
        val out = StringBuilder(digits.length + digits.length / GROUP_SIZE * separator.length)
        val head = if (digits.length % GROUP_SIZE == 0) GROUP_SIZE else digits.length % GROUP_SIZE
        out.append(digits, 0, head)
        var i = head
        while (i < digits.length) {
            out.append(separator).append(digits, i, i + GROUP_SIZE)
            i += GROUP_SIZE
        }
        return out.toString()
    }

    /**
     * Round half away from zero at [digits] decimals.
     *
     * `String.format("%.1f", …)` already rounds half-up, but it is locale
     * sensitive (a `de` device would emit `1,3`) and it rounds the *decimal*
     * expansion, so we do the arithmetic ourselves and assemble the string with
     * the catalogue's separators. That also keeps us bit-identical to the core.
     */
    internal fun roundHalfAwayFromZero(value: Double, digits: Int): Double {
        var factor = 1.0
        repeat(digits) { factor *= 10.0 }
        val scaled = value * factor
        if (!scaled.isFinite() || abs(scaled) >= EXACT_INTEGER_LIMIT) return value
        // Kotlin's roundToLong() is half-up for positives; mirror Rust's
        // f64::round (half away from zero) explicitly for negatives.
        val rounded = if (scaled < 0) -(abs(scaled).roundToLong()).toDouble()
        else scaled.roundToLong().toDouble()
        return rounded / factor
    }

    /** Render [value] with exactly [digits] decimals, using the plain ASCII path. */
    private fun fixed(value: Double, digits: Int): String {
        val rounded = roundHalfAwayFromZero(value, digits)
        // Locale.ROOT so a device set to de-DE cannot inject a comma here: the
        // decimal separator is the catalogue's job, not the platform's.
        return String.format(java.util.Locale.ROOT, "%.${digits}f", rounded)
    }

    /** Group an integer with the table's separator (`1,234,567`). */
    fun int(strings: RcStrings, value: Long): String {
        val separator = groupSeparator(strings)
        // Use the unsigned decimal text so Long.MIN_VALUE cannot overflow abs().
        val magnitude = if (value == Long.MIN_VALUE) {
            java.lang.Long.toUnsignedString(-value)
        } else {
            abs(value).toString()
        }
        val grouped = groupDigits(magnitude, separator)
        return if (value < 0) "-$grouped" else grouped
    }

    /** Render [value] with [fractionDigits] decimals, grouped and localised. */
    fun decimal(strings: RcStrings, value: Double, fractionDigits: Int = 1): String {
        if (!value.isFinite()) return strings.skeleton("format.invalid_number", "—")
        val digits = fractionDigits.coerceIn(0, MAX_FRACTION_DIGITS)
        val rendered = fixed(abs(value), digits)
        val dot = rendered.indexOf('.')
        val intPart = if (dot < 0) rendered else rendered.substring(0, dot)
        val fracPart = if (dot < 0) "" else rendered.substring(dot + 1)

        val group = groupSeparator(strings)
        // A blank *decimal* separator would fuse "1" and "5" into "15", so unlike
        // the grouping separator it always falls back.
        val decimalSep = strings.skeleton("format.decimal_separator", DEFAULT_DECIMAL_SEPARATOR)

        // Only sign a value that survived rounding: "-0.0" looks like a bug.
        val significant = rendered.any { it in '1'..'9' }
        val out = StringBuilder(rendered.length + 8)
        if (value < 0 && significant) out.append('-')
        out.append(groupDigits(intPart, group))
        if (fracPart.isNotEmpty()) out.append(decimalSep).append(fracPart)
        return out.toString()
    }

    /** Whole bytes are never fractional; every larger unit gets one decimal. */
    private fun byteFractionDigits(idx: Int): Int = if (idx == 0) 0 else 1

    /** Scale [bytes] into value + index into [BYTE_UNIT_KEYS] (1024 steps). */
    private fun scaleBytes(bytes: Long): Pair<Double, Int> {
        var value = bytes.coerceAtLeast(0).toDouble()
        var idx = 0
        while (value >= 1024.0 && idx + 1 < BYTE_UNIT_KEYS.size) {
            value /= 1024.0
            idx++
        }
        // Rounding can push the value back to 1024 ("1024.0 KB"); promote it.
        if (idx + 1 < BYTE_UNIT_KEYS.size) {
            if (roundHalfAwayFromZero(value, byteFractionDigits(idx)) >= 1024.0) {
                value /= 1024.0
                idx++
            }
        }
        return value to idx
    }

    /** `1.4 GB` — a byte size through the `format.size` skeleton. */
    fun bytes(strings: RcStrings, bytes: Long): String {
        val (value, idx) = scaleBytes(bytes)
        val unit = strings.skeleton(BYTE_UNIT_KEYS[idx], BYTE_UNIT_FALLBACK[idx])
        val number = decimal(strings, value, byteFractionDigits(idx))
        return RcStringFormat.interpolate(
            strings.skeleton("format.size", "{value} {unit}"),
            mapOf("value" to number, "unit" to unit),
        )
    }

    /** `1.2 MB/秒` — a transfer rate through the `format.rate` skeleton. */
    fun rate(strings: RcStrings, bytesPerSecond: Long): String {
        val (value, idx) = scaleBytes(bytesPerSecond)
        val unit = strings.skeleton(BYTE_UNIT_KEYS[idx], BYTE_UNIT_FALLBACK[idx])
        val number = decimal(strings, value, byteFractionDigits(idx))
        return RcStringFormat.interpolate(
            strings.skeleton("format.rate", "{value} {unit}/s"),
            mapOf("value" to number, "unit" to unit),
        )
    }

    /** `1.0 MB / 4.0 MB` — a byte-progress pair. */
    fun byteProgress(strings: RcStrings, done: Long, total: Long): String =
        RcStringFormat.interpolate(
            strings.skeleton("format.progress_of", "{done} / {total}"),
            mapOf("done" to bytes(strings, done), "total" to bytes(strings, total)),
        )

    /** `42.5%` — an already-scaled percentage. */
    fun percent(strings: RcStrings, percent: Double, fractionDigits: Int = 1): String =
        RcStringFormat.interpolate(
            strings.skeleton("format.percent", "{value}%"),
            mapOf("value" to decimal(strings, percent, fractionDigits)),
        )

    /** Percentage of [done] out of [total]; a zero [total] is 0 % (never NaN). */
    fun ratioPercent(strings: RcStrings, done: Long, total: Long, fractionDigits: Int = 1): String {
        val value = if (total == 0L) 0.0 else done.toDouble() * 100.0 / total.toDouble()
        return percent(strings, value, fractionDigits)
    }

    /** `59.9 FPS` — the AWT / renderer overlay readout. */
    fun fps(strings: RcStrings, fps: Double): String =
        RcStringFormat.interpolate(
            strings.skeleton("format.fps", "{value} FPS"),
            mapOf("value" to decimal(strings, fps, 1)),
        )

    /**
     * One duration piece (`3 minutes`), plural-aware, with a compiled-in
     * fallback. Deliberately *not* a bare [RcStrings.plural]: see [DURATION_UNITS].
     */
    private fun durationPiece(
        strings: RcStrings,
        base: String,
        fallback: String,
        count: Long,
    ): String {
        val key = RcStringFormat.pluralKey(strings.language, base, count)
        val template = if (strings.has(key)) strings[key].ifBlank { fallback } else fallback
        return RcStringFormat.interpolate(template, mapOf("count" to count.toString()))
    }

    /**
     * Humanise a duration, at most [maxParts] units (`3 分 20 秒`). The sign is
     * ignored — use [relativeTime] for direction.
     */
    fun duration(
        strings: RcStrings,
        seconds: Long,
        maxParts: Int = DEFAULT_DURATION_PARTS,
    ): String {
        val zero = { strings.skeleton("duration.zero", "0 s") }
        // Long.MIN_VALUE has no positive counterpart: saturate like Rust does.
        var remaining = if (seconds == Long.MIN_VALUE) Long.MAX_VALUE else abs(seconds)
        if (remaining == 0L || maxParts <= 0) return zero()

        val parts = ArrayList<String>(minOf(maxParts, DURATION_UNITS.size))
        for ((size, base, fallback) in DURATION_UNITS) {
            if (parts.size == maxParts) break
            val n = remaining / size
            if (n == 0L) continue
            remaining -= n * size
            parts.add(durationPiece(strings, base, fallback, n))
        }
        if (parts.isEmpty()) return zero()

        val pattern = strings.skeleton("format.duration_join", "{first} {second}")
        var acc = parts[0]
        for (i in 1 until parts.size) {
            acc = RcStringFormat.interpolate(pattern, mapOf("first" to acc, "second" to parts[i]))
        }
        return acc
    }

    /**
     * Phrase a timestamp relative to now: `deltaSeconds = now - timestamp`, so a
     * **positive** delta is in the past.
     */
    fun relativeTime(strings: RcStrings, deltaSeconds: Long): String {
        val magnitude = if (deltaSeconds == Long.MIN_VALUE) Long.MAX_VALUE else abs(deltaSeconds)
        if (magnitude < RELATIVE_NOW_THRESHOLD) return strings.skeleton("relative.now", "just now")
        val key = if (deltaSeconds > 0) "relative.past" else "relative.future"
        return RcStringFormat.interpolate(
            strings.skeleton(key, "{duration}"),
            mapOf("duration" to duration(strings, deltaSeconds)),
        )
    }

    /** `剩余 3 分 20 秒` — a download ETA; negative input is treated as zero. */
    fun eta(strings: RcStrings, seconds: Long): String =
        RcStringFormat.interpolate(
            strings.skeleton("download.eta", "{duration}"),
            mapOf("duration" to duration(strings, seconds.coerceAtLeast(0))),
        )

    /**
     * Every catalogue key this object reads — asserted by `check_i18n.py` and by
     * the Kotlin/Rust parity tests, so the two ports cannot drift.
     */
    fun requiredKeys(): List<String> = buildList {
        addAll(
            listOf(
                "format.group_separator", "format.decimal_separator", "format.invalid_number",
                "format.size", "format.rate", "format.percent", "format.progress_of",
                "format.duration_join", "format.fps",
                "duration.zero", "relative.now", "relative.past", "relative.future",
                "download.eta",
            ),
        )
        addAll(BYTE_UNIT_KEYS)
        for ((_, base, _) in DURATION_UNITS) {
            add("$base.one")
            add("$base.other")
        }
    }.sorted()
}

// --- RcStrings sugar --------------------------------------------------------
//
// So a call site reads `strings.bytes(size)` instead of
// `RcValueFormat.bytes(strings, size)`.

/** [RcValueFormat.bytes] on this table. */
fun RcStrings.bytes(value: Long): String = RcValueFormat.bytes(this, value)

/** [RcValueFormat.rate] on this table. */
fun RcStrings.rate(bytesPerSecond: Long): String = RcValueFormat.rate(this, bytesPerSecond)

/** [RcValueFormat.byteProgress] on this table. */
fun RcStrings.byteProgress(done: Long, total: Long): String =
    RcValueFormat.byteProgress(this, done, total)

/** [RcValueFormat.duration] on this table. */
fun RcStrings.duration(seconds: Long): String = RcValueFormat.duration(this, seconds)

/** [RcValueFormat.eta] on this table. */
fun RcStrings.eta(seconds: Long): String = RcValueFormat.eta(this, seconds)

/** [RcValueFormat.relativeTime] on this table. */
fun RcStrings.relativeTime(deltaSeconds: Long): String =
    RcValueFormat.relativeTime(this, deltaSeconds)

/** [RcValueFormat.percent] on this table. */
fun RcStrings.percent(value: Double, fractionDigits: Int = 1): String =
    RcValueFormat.percent(this, value, fractionDigits)

/** [RcValueFormat.ratioPercent] on this table. */
fun RcStrings.ratioPercent(done: Long, total: Long, fractionDigits: Int = 1): String =
    RcValueFormat.ratioPercent(this, done, total, fractionDigits)

/** [RcValueFormat.fps] on this table. */
fun RcStrings.fps(value: Double): String = RcValueFormat.fps(this, value)

/** [RcValueFormat.int] on this table. */
fun RcStrings.integer(value: Long): String = RcValueFormat.int(this, value)
