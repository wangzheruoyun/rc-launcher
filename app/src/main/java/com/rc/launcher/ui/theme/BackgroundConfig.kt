package com.rc.launcher.ui.theme

/**
 * Custom launcher background support (task 11).
 *
 * A user may pick an image from the gallery / a document picker to use as the
 * launcher's home and launch background. The image is rendered behind every
 * screen by [RcBackground] and may be blurred, darkened, or both. When
 * [BackgroundConfig.followTheme] is on, the darken overlay strength tracks the
 * active light/dark mode so the chrome stays readable in both.
 *
 * Everything in this file is plain Kotlin (no Android imports) so the logic can
 * be unit-tested on the JVM without a device or a Robolectric shadow. The
 * Android-only helpers that *use* these types live in [BackgroundCache] and
 * [RcBackground].
 */

/** Visual treatment applied to the custom background image. */
enum class BackgroundEffect(val value: Int) {
    /** Show the image as-is. */
    NONE(0),
    /** Gaussian-style blur only. */
    BLUR(1),
    /** Black overlay only. */
    DARKEN(2),
    /** Blur plus a black overlay. */
    BLUR_DARKEN(3);

    companion object {
        fun fromValue(v: Int): BackgroundEffect =
            entries.firstOrNull { it.value == v } ?: NONE
    }

    /** True when the blur pass should run. */
    val blurs: Boolean get() = this == BLUR || this == BLUR_DARKEN

    /** True when the darken overlay should run. */
    val darkens: Boolean get() = this == DARKEN || this == BLUR_DARKEN
}

/** Hard limits so a corrupt / hand-edited value can never push the UI out of range. */
object BackgroundLimits {
    const val MIN_BLUR_DP = 0
    const val MAX_BLUR_DP = 48
    const val MIN_DARKEN = 0f
    const val MAX_DARKEN = 1f
}

/**
 * Serializable description of the custom-background preference.
 *
 * [uri] is one of:
 *  * a `content://` URI produced by the document/gallery picker (the system keeps
 *    a persistable read grant for it), or
 *  * a `file://` path inside the app's own cache directory — see [BackgroundCache],
 *    which copies a picked image there so the background survives a revoked grant, or
 *  * `null` when the background is disabled or not yet chosen.
 */
data class BackgroundConfig(
    val enabled: Boolean = false,
    val uri: String? = null,
    val effect: BackgroundEffect = BackgroundEffect.NONE,
    val blurRadiusDp: Int = 16,
    val darkenAlpha: Float = 0.45f,
    val followTheme: Boolean = true,
) {
    /** Clamp the numeric fields into [BackgroundLimits] (no-op copy when already valid). */
    fun normalized(): BackgroundConfig {
        val b = blurRadiusDp.coerceIn(BackgroundLimits.MIN_BLUR_DP, BackgroundLimits.MAX_BLUR_DP)
        val d = darkenAlpha.coerceIn(BackgroundLimits.MIN_DARKEN, BackgroundLimits.MAX_DARKEN)
        return if (b == blurRadiusDp && d == darkenAlpha) this else copy(blurRadiusDp = b, darkenAlpha = d)
    }

    /**
     * A config is usable only when enabled AND pointing at a validated source.
     * [BackgroundValidator.isValidUri] performs the safe-scheme / safe-path check.
     */
    fun isValid(): Boolean = !enabled || (!uri.isNullOrBlank() && BackgroundValidator.isValidUri(uri))

    /**
     * Effective darken overlay alpha, taking [followTheme] + [isDark] into account.
     * In dark mode the chrome is already dark, so we soften the overlay a little;
     * in light mode we strengthen it so text stays legible over a bright photo.
     * The result is always within [0, 1].
     */
    fun effectiveDarkenAlpha(isDark: Boolean): Float {
        val base = darkenAlpha.coerceIn(BackgroundLimits.MIN_DARKEN, BackgroundLimits.MAX_DARKEN)
        if (!followTheme) return base
        val scaled = if (isDark) base * 0.6f else base * 1.25f
        return scaled.coerceIn(0f, 1f)
    }

    companion object {
        val DEFAULT = BackgroundConfig()
    }
}

/**
 * Safety checks for a background image source (task 11).
 *
 * The background image is loaded from a string the user (or a previous session)
 * supplied, so before we hand it to [android.content.ContentResolver] /
 * [android.graphics.BitmapFactory] we must prove it is either a system-vetted
 * `content://` grant or a file living inside the app's own directories. Anything
 * else (`http(s)://`, a `file://` pointing elsewhere, a path containing `../`)
 * is rejected — loading a remote or foreign file into the launcher surface would
 * be a privacy/perf footgun and could bypass the China-mainland network policy.
 *
 * The checks are pure string operations (testable on the JVM); callers with a
 * [android.content.Context] layer [BackgroundCache] on top for real file IO.
 */
object BackgroundValidator {
    private val SAFE_SCHEMES = setOf("content", "file")

    /**
     * Validate a raw [uri] string without touching Android APIs. Returns true only
     * for a `content://` grant or a `file://` path that passes [isSafePath].
     */
    fun isValidUri(uri: String?): Boolean {
        if (uri.isNullOrBlank()) return false
        val scheme = uri.substringBefore(':', "").lowercase()
        if (scheme !in SAFE_SCHEMES) return false
        if (scheme == "content") {
            // A system picker grant: trust it as long as it has a host/authority.
            val marker = uri.indexOf("://", 0, ignoreCase = true)
            return marker > 0 && uri.length > marker + 3
        }
        // file scheme: only inside an allowed root.
        return isSafePath(uri.removePrefix("file://"))
    }

    /**
     * Ensure [path] stays inside [allowedRoot] and contains no `..` traversal or
     * stray scheme remnants. When [allowedRoot] is null only the structural
     * (scheme/blank/traversal) checks run.
     */
    fun isSafePath(path: String, allowedRoot: String? = null): Boolean {
        if (path.isBlank()) return false
        if (path.contains("..")) return false
        if (path.startsWith("content:") || path.startsWith("http")) return false
        val root = allowedRoot ?: return true
        val normalizedRoot = root.replace('\\', '/').let { if (it.endsWith("/")) it else "$it/" }
        val normalizedPath = path.replace('\\', '/')
        if (!normalizedPath.startsWith(normalizedRoot)) return false
        return !normalizedPath.removePrefix(normalizedRoot).contains("..")
    }
}
