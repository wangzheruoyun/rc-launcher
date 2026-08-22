package com.rc.launcher.ui.awt

/**
 * The AWT bridge wire formats (task 18), Kotlin side.
 *
 * The Rust core owns the *live* transport (it pumps the named pipes the game JVM
 * writes to), so the app normally never touches these bytes. Two cases still
 * need them, and both matter:
 *
 * * **Self-test / diagnostics** — the AWT screen can encode a locally generated
 *   test pattern and push it through `RustBridge.awtSubmitFrame`, which proves
 *   the whole chain (wire format → validation → canvas → bitmap) works on this
 *   device *without* a running game.
 * * **Kotlin-owned transport** — a future bridge implemented in Java/Kotlin
 *   (e.g. reading the game's socket in a `Service`) can decode frames and encode
 *   the records returned by `RustBridge.awtDrainEvents`.
 *
 * Layouts (little-endian, mirroring `launch::awt`):
 *
 * ```text
 * frame header (32 B):
 *   0  u32 magic ("RCAF")   4 u16 version   6 u16 format
 *   8  u32 seq             12 u16 width    14 u16 height
 *   16 u16 damage.x        18 u16 damage.y 20 u16 damage.w  22 u16 damage.h
 *   24 u32 payload_len (bytes)             28 u32 flags (bit0 = full frame)
 * event record (32 B): 8 x i32 — id, x, y, button, keyCode, keyChar, modifiers, wheel
 * ```
 *
 * Pure Kotlin (no Android imports): unit-testable on the JVM.
 */
object AwtWire {
    /** `"RCAF"` — RC launcher AWT frame. */
    const val FRAME_MAGIC: Int = 0x5243_4146

    /** Current frame wire version. */
    const val FRAME_VERSION: Int = 1

    /** Size of the fixed frame header in bytes. */
    const val FRAME_HEADER_LEN: Int = 32

    /** Size of one encoded AWT event record. */
    const val EVENT_RECORD_LEN: Int = 32

    /** `PixelFormat::IntArgb` — 0xAARRGGBB. */
    const val FORMAT_ARGB: Int = 0

    /** `PixelFormat::IntRgb` — 0x00RRGGBB (alpha forced opaque by the core). */
    const val FORMAT_RGB: Int = 1

    /** Hard upper bound for a desktop edge (`MAX_CANVAS_DIM` in the core). */
    const val MAX_CANVAS_DIM: Int = 8192

    /** Fully opaque black — the colour an AWT desktop starts as. */
    const val OPAQUE_BLACK: Int = 0xFF000000.toInt()

    /**
     * Encode a frame carrying only [damage] (row-major, `damage.area` pixels).
     *
     * @throws IllegalArgumentException when the dimensions, the damage rectangle
     *   or the payload size are not a legal frame — the same validation the core
     *   does, so a bad self-test fails here instead of being rejected over JNI.
     */
    fun encodeFrame(
        seq: Int,
        width: Int,
        height: Int,
        pixels: IntArray,
        damage: AwtRect = AwtRect.whole(width, height),
        format: Int = FORMAT_ARGB,
    ): ByteArray {
        require(width in 1..MAX_CANVAS_DIM && height in 1..MAX_CANVAS_DIM) {
            "invalid AWT desktop size ${width}x$height (1..$MAX_CANVAS_DIM)"
        }
        require(!damage.isEmpty) { "an AWT frame needs a non-empty damage rectangle" }
        require(damage.x >= 0 && damage.y >= 0) { "damage origin must not be negative" }
        require(damage.x + damage.width <= width && damage.y + damage.height <= height) {
            "damage ${damage.width}x${damage.height}+${damage.x}+${damage.y} exceeds ${width}x$height"
        }
        require(pixels.size.toLong() == damage.area) {
            "payload has ${pixels.size} pixels, expected ${damage.area}"
        }
        val out = ByteArray(FRAME_HEADER_LEN + pixels.size * 4)
        putInt(out, 0, FRAME_MAGIC)
        putShort(out, 4, FRAME_VERSION)
        putShort(out, 6, format)
        putInt(out, 8, seq)
        putShort(out, 12, width)
        putShort(out, 14, height)
        putShort(out, 16, damage.x)
        putShort(out, 18, damage.y)
        putShort(out, 20, damage.width)
        putShort(out, 22, damage.height)
        putInt(out, 24, pixels.size * 4)
        putInt(out, 28, if (damage == AwtRect.whole(width, height)) 1 else 0)
        var offset = FRAME_HEADER_LEN
        for (pixel in pixels) {
            putInt(out, offset, pixel)
            offset += 4
        }
        return out
    }

    /**
     * Parse a frame header, or `null` when [bytes] is not a valid one (truncated,
     * bad magic / version / format, absurd dimensions, damage outside the
     * desktop, payload length disagreeing with the damage). Never throws.
     */
    fun decodeFrameHeader(bytes: ByteArray): AwtFrameHeader? {
        if (bytes.size < FRAME_HEADER_LEN) return null
        if (getInt(bytes, 0) != FRAME_MAGIC) return null
        if (getShort(bytes, 4) != FRAME_VERSION) return null
        val format = getShort(bytes, 6)
        if (format != FORMAT_ARGB && format != FORMAT_RGB) return null
        val width = getShort(bytes, 12)
        val height = getShort(bytes, 14)
        if (width !in 1..MAX_CANVAS_DIM || height !in 1..MAX_CANVAS_DIM) return null
        val damage = AwtRect(
            getShort(bytes, 16),
            getShort(bytes, 18),
            getShort(bytes, 20),
            getShort(bytes, 22),
        )
        if (damage.isEmpty) return null
        if (damage.x + damage.width > width || damage.y + damage.height > height) return null
        val payloadLen = getInt(bytes, 24)
        if (payloadLen < 0 || payloadLen % 4 != 0) return null
        if (payloadLen.toLong() != damage.area * 4L) return null
        if (bytes.size - FRAME_HEADER_LEN < payloadLen) return null
        return AwtFrameHeader(
            seq = getInt(bytes, 8),
            width = width,
            height = height,
            format = format,
            damage = damage,
            payloadLen = payloadLen,
            full = damage == AwtRect.whole(width, height),
        )
    }

    /** Decode the pixels of a frame (`null` when the frame is not valid). */
    fun decodeFramePixels(bytes: ByteArray): IntArray? {
        val header = decodeFrameHeader(bytes) ?: return null
        val pixels = IntArray(header.payloadLen / 4)
        for (i in pixels.indices) {
            pixels[i] = getInt(bytes, FRAME_HEADER_LEN + i * 4)
        }
        return pixels
    }

    /** Encode AWT event records the way the JVM-side bridge expects them. */
    fun encodeEventRecords(records: List<AwtEventRecord>): ByteArray {
        val out = ByteArray(records.size * EVENT_RECORD_LEN)
        var offset = 0
        for (record in records) {
            putInt(out, offset, record.id)
            putInt(out, offset + 4, record.x)
            putInt(out, offset + 8, record.y)
            putInt(out, offset + 12, record.button)
            putInt(out, offset + 16, record.keyCode)
            putInt(out, offset + 20, record.keyChar)
            putInt(out, offset + 24, record.modifiers)
            putInt(out, offset + 28, record.wheel)
            offset += EVENT_RECORD_LEN
        }
        return out
    }

    /**
     * Decode a batch of AWT event records (`RustBridge.awtDrainEvents`).
     * Returns an empty list for an empty batch and `null` for a buffer whose
     * length is not a record multiple.
     */
    fun decodeEventRecords(bytes: ByteArray): List<AwtEventRecord>? {
        if (bytes.size % EVENT_RECORD_LEN != 0) return null
        val out = ArrayList<AwtEventRecord>(bytes.size / EVENT_RECORD_LEN)
        var offset = 0
        while (offset < bytes.size) {
            out.add(
                AwtEventRecord(
                    id = getInt(bytes, offset),
                    x = getInt(bytes, offset + 4),
                    y = getInt(bytes, offset + 8),
                    button = getInt(bytes, offset + 12),
                    keyCode = getInt(bytes, offset + 16),
                    keyChar = getInt(bytes, offset + 20),
                    modifiers = getInt(bytes, offset + 24),
                    wheel = getInt(bytes, offset + 28),
                ),
            )
            offset += EVENT_RECORD_LEN
        }
        return out
    }

    /**
     * A deterministic ARGB test pattern (checkerboard + gradient + border) used
     * by the diagnostics self-test: every quadrant differs, so a wrong stride,
     * a flipped row order or a swapped colour channel is visible at a glance.
     */
    fun testPattern(width: Int, height: Int, tile: Int = 16): IntArray {
        require(width in 1..MAX_CANVAS_DIM && height in 1..MAX_CANVAS_DIM) {
            "invalid pattern size ${width}x$height"
        }
        val step = maxOf(1, tile)
        val pixels = IntArray(width * height)
        for (y in 0 until height) {
            val green = if (height > 1) 255 * y / (height - 1) else 0
            for (x in 0 until width) {
                val red = if (width > 1) 255 * x / (width - 1) else 0
                val checker = ((x / step) + (y / step)) % 2 == 0
                val blue = if (checker) 96 else 32
                val edge = x == 0 || y == 0 || x == width - 1 || y == height - 1
                pixels[y * width + x] = if (edge) {
                    0xFFFFFFFF.toInt()
                } else {
                    (0xFF shl 24) or (red shl 16) or (green shl 8) or blue
                }
            }
        }
        return pixels
    }

    // ---- little-endian helpers ---------------------------------------------

    private fun putInt(dst: ByteArray, offset: Int, value: Int) {
        dst[offset] = (value and 0xFF).toByte()
        dst[offset + 1] = ((value ushr 8) and 0xFF).toByte()
        dst[offset + 2] = ((value ushr 16) and 0xFF).toByte()
        dst[offset + 3] = ((value ushr 24) and 0xFF).toByte()
    }

    private fun putShort(dst: ByteArray, offset: Int, value: Int) {
        dst[offset] = (value and 0xFF).toByte()
        dst[offset + 1] = ((value ushr 8) and 0xFF).toByte()
    }

    private fun getInt(src: ByteArray, offset: Int): Int =
        (src[offset].toInt() and 0xFF) or
            ((src[offset + 1].toInt() and 0xFF) shl 8) or
            ((src[offset + 2].toInt() and 0xFF) shl 16) or
            ((src[offset + 3].toInt() and 0xFF) shl 24)

    private fun getShort(src: ByteArray, offset: Int): Int =
        (src[offset].toInt() and 0xFF) or ((src[offset + 1].toInt() and 0xFF) shl 8)
}

/** Header of one `RCAF` frame (see [AwtWire]). */
data class AwtFrameHeader(
    val seq: Int,
    val width: Int,
    val height: Int,
    val format: Int,
    val damage: AwtRect,
    val payloadLen: Int,
    val full: Boolean,
)

/**
 * One AWT event record the JVM-side bridge turns into a real
 * `java.awt.event.*Event`. Ids follow `java.awt.event` (`MOUSE_PRESSED` = 501,
 * `KEY_PRESSED` = 401, …).
 */
data class AwtEventRecord(
    val id: Int,
    val x: Int,
    val y: Int,
    val button: Int,
    val keyCode: Int,
    val keyChar: Int,
    val modifiers: Int,
    val wheel: Int,
) {
    /** `java.awt.event.*Event` ids we produce, for readable diagnostics. */
    companion object {
        const val KEY_TYPED = 400
        const val KEY_PRESSED = 401
        const val KEY_RELEASED = 402
        const val MOUSE_CLICKED = 500
        const val MOUSE_PRESSED = 501
        const val MOUSE_RELEASED = 502
        const val MOUSE_MOVED = 503
        const val MOUSE_DRAGGED = 506
        const val MOUSE_WHEEL = 507
        const val COMPONENT_RESIZED = 101
        const val FOCUS_GAINED = 1004
        const val FOCUS_LOST = 1005

        /** Human-readable name of an event id (`"MOUSE_PRESSED"`, …). */
        fun nameOf(id: Int): String = when (id) {
            KEY_TYPED -> "KEY_TYPED"
            KEY_PRESSED -> "KEY_PRESSED"
            KEY_RELEASED -> "KEY_RELEASED"
            MOUSE_CLICKED -> "MOUSE_CLICKED"
            MOUSE_PRESSED -> "MOUSE_PRESSED"
            MOUSE_RELEASED -> "MOUSE_RELEASED"
            MOUSE_MOVED -> "MOUSE_MOVED"
            MOUSE_DRAGGED -> "MOUSE_DRAGGED"
            MOUSE_WHEEL -> "MOUSE_WHEEL"
            COMPONENT_RESIZED -> "COMPONENT_RESIZED"
            FOCUS_GAINED -> "FOCUS_GAINED"
            FOCUS_LOST -> "FOCUS_LOST"
            else -> "EVENT_$id"
        }
    }
}
