package com.rc.launcher.ui.awt

/**
 * Geometry of the AWT/Swing compatibility layer (task 18, "fakefx").
 *
 * The game JVM paints Minecraft's embedded AWT UI into an off-screen ARGB
 * *desktop* (caciocavallo); Compose draws that desktop onto a *surface* whose
 * size is whatever the phone gives us. This file is the Kotlin mirror of
 * `launch::awt::{ScaleMode, Placement, Viewport}` in the Rust core: the UI needs
 * the placement every frame (to blit the bitmap into the right rectangle) and
 * the pointer mapping for the letterbox bars, and doing that locally keeps the
 * JNI boundary at *one* call per frame.
 *
 * Everything is computed in integer arithmetic — exactly like the Rust side — so
 * no rounding drift can appear between "where we drew the desktop" and "where
 * the core thinks the touch landed", and no `NaN` can reach a blit.
 *
 * Pure Kotlin (no Android imports) so it is fully unit-testable on the JVM.
 */

/** How the virtual AWT desktop is fitted into the Compose surface. */
enum class AwtScaleMode(val id: String, val label: String) {
    /** Fill the surface exactly, ignoring the aspect ratio. */
    STRETCH("stretch", "拉伸填满"),

    /** Preserve the aspect ratio, letterboxing the remainder (**default**). */
    FIT("fit", "等比适配"),

    /** Preserve the aspect ratio and cover the surface, cropping the overflow. */
    FILL_CROP("fill_crop", "等比裁剪"),

    /** No scaling (1 desktop px = 1 surface px), centred. */
    CENTER("center", "原始尺寸");

    companion object {
        /** Parse the core's `scale_mode` id, falling back to [FIT]. */
        fun fromId(id: String?): AwtScaleMode =
            values().firstOrNull { it.id == id } ?: FIT
    }
}

/** A rectangle in desktop pixels (the damaged region of a frame). */
data class AwtRect(val x: Int, val y: Int, val width: Int, val height: Int) {
    val isEmpty: Boolean get() = width <= 0 || height <= 0

    /** Pixel count as a [Long], so a huge rectangle cannot overflow. */
    val area: Long get() = width.toLong() * height.toLong()

    companion object {
        /** The rectangle covering a whole `width x height` desktop. */
        fun whole(width: Int, height: Int): AwtRect = AwtRect(0, 0, width, height)
    }
}

/**
 * Where the desktop lands inside the surface. The origin may be negative for
 * [AwtScaleMode.FILL_CROP] / [AwtScaleMode.CENTER] (the picture then extends
 * past the edges), hence the signed coordinates.
 */
data class AwtPlacement(val x: Int, val y: Int, val width: Int, val height: Int) {
    val isEmpty: Boolean get() = width <= 0 || height <= 0
}

/** A pointer position in desktop pixels. */
data class AwtPoint(val x: Int, val y: Int)

/**
 * Maps between the virtual AWT desktop and the Compose surface it is drawn on.
 *
 * This is what makes a *touch* land on the right Swing button: Compose reports a
 * position in surface pixels and the AWT peers only understand desktop pixels.
 */
data class AwtViewport(
    val screenWidth: Int,
    val screenHeight: Int,
    val surfaceWidth: Int,
    val surfaceHeight: Int,
    val mode: AwtScaleMode = AwtScaleMode.FIT,
) {
    /** Where the desktop is drawn inside the surface. */
    fun placement(): AwtPlacement {
        if (screenWidth <= 0 || screenHeight <= 0 || surfaceWidth <= 0 || surfaceHeight <= 0) {
            return AwtPlacement(0, 0, 0, 0)
        }
        val cw = screenWidth.toLong()
        val ch = screenHeight.toLong()
        val sw = surfaceWidth.toLong()
        val sh = surfaceHeight.toLong()
        var w: Long
        var h: Long
        when (mode) {
            AwtScaleMode.STRETCH -> {
                w = sw
                h = sh
            }
            AwtScaleMode.CENTER -> {
                w = cw
                h = ch
            }
            AwtScaleMode.FIT, AwtScaleMode.FILL_CROP -> {
                // Compare sw/cw against sh/ch without dividing (no float, no NaN).
                val byWidth = sw * ch
                val byHeight = sh * cw
                val heightIsLimit =
                    if (mode == AwtScaleMode.FIT) byWidth > byHeight else byWidth < byHeight
                if (heightIsLimit) {
                    w = maxOf(1L, cw * sh / ch)
                    h = sh
                } else {
                    w = sw
                    h = maxOf(1L, ch * sw / cw)
                }
            }
        }
        return AwtPlacement(
            x = ((sw - w) / 2).toInt(),
            y = ((sh - h) / 2).toInt(),
            width = w.toInt(),
            height = h.toInt(),
        )
    }

    /** Scale factors `(x, y)` from desktop to surface pixels (for overlays). */
    fun scale(): Pair<Float, Float> {
        val p = placement()
        if (screenWidth <= 0 || screenHeight <= 0 || p.isEmpty) return 1f to 1f
        return (p.width.toFloat() / screenWidth) to (p.height.toFloat() / screenHeight)
    }

    /**
     * Map a surface position to a desktop pixel, or `null` when it is outside the
     * drawn area (a tap on a letterbox bar must not become a click at the edge)
     * or not finite.
     */
    fun mapPointer(surfaceX: Float, surfaceY: Float): AwtPoint? {
        if (!surfaceX.isFinite() || !surfaceY.isFinite()) return null
        val p = placement()
        if (p.isEmpty) return null
        val relX = surfaceX - p.x
        val relY = surfaceY - p.y
        if (relX < 0f || relY < 0f || relX >= p.width || relY >= p.height) return null
        val x = (relX * screenWidth / p.width).toInt()
        val y = (relY * screenHeight / p.height).toInt()
        return AwtPoint(
            x = x.coerceIn(0, maxOf(0, screenWidth - 1)),
            y = y.coerceIn(0, maxOf(0, screenHeight - 1)),
        )
    }

    /**
     * Like [mapPointer] but clamped into the desktop instead of rejected: used
     * while *dragging*, where a finger leaving the letterbox must keep dragging
     * the Swing scrollbar rather than dropping it.
     */
    fun mapPointerClamped(surfaceX: Float, surfaceY: Float): AwtPoint {
        val p = placement()
        if (!surfaceX.isFinite() || !surfaceY.isFinite() || p.isEmpty) return AwtPoint(0, 0)
        val relX = maxOf(0f, surfaceX - p.x)
        val relY = maxOf(0f, surfaceY - p.y)
        val x = (relX * screenWidth / p.width).toInt()
        val y = (relY * screenHeight / p.height).toInt()
        return AwtPoint(
            x = x.coerceIn(0, maxOf(0, screenWidth - 1)),
            y = y.coerceIn(0, maxOf(0, screenHeight - 1)),
        )
    }

    /** `true` when the surface position is inside the drawn desktop. */
    fun contains(surfaceX: Float, surfaceY: Float): Boolean = mapPointer(surfaceX, surfaceY) != null
}
