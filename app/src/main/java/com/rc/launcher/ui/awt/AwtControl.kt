package com.rc.launcher.ui.awt

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import com.rc.launcher.ui.model.json.toJsonString

/*
 * The AWT **control plane** (task 18), Kotlin side.
 *
 * Pixels alone do not make an AWT desktop usable. caciocavallo's peers also
 * implement the clipboard, the cursor, window titles and the input-method
 * plumbing, and all of those need the host that owns the real screen:
 *
 * | the JVM does | this app must |
 * |---|---|
 * | `setCursor(TEXT_CURSOR)` | draw an I-beam, not an arrow |
 * | `JFrame.setTitle(…)` | label the canvas |
 * | `Clipboard.setContents(…)` | push it onto the Android clipboard |
 * | `Clipboard.getContents()` | **answer** with the Android clipboard |
 * | a text field gains focus | show the soft keyboard at the caret |
 * | `Toolkit.beep()` | a haptic tick |
 *
 * The core parses the wire, folds it into a projection and hands both over as
 * JSON (`RustBridge.awtDrainControl`); this file is the Kotlin view of it. Two
 * things are also implemented natively here:
 *
 * * [AwtControlWire] encodes / decodes the `RCAC` record, so the diagnostics
 *   self-test can inject a cursor change (and a Kotlin-owned transport could
 *   carry the channel itself);
 * * [AwtControlWire.decodeReply] re-assembles a chunked answer, which is the
 *   *JVM side's* algorithm — having it here keeps the contract honest and
 *   unit-testable without a JVM.
 *
 * Parsing is fail-soft throughout: a truncated / unexpected document yields an
 * empty batch, never an exception (task 19 — the AWT canvas must not be able to
 * crash the launcher UI).
 *
 * Pure Kotlin (no Android imports): unit-testable on the JVM.
 */

/**
 * The pointer shape the JVM asked for (`java.awt.Cursor` types).
 *
 * Android has no cursor to hand to a window manager, so the launcher draws its
 * own - but *which* one matters: an I-beam is the only cue that a Swing text
 * field is under the finger, and a hand the only cue that a `JLabel` is a link.
 */
enum class AwtCursorKind(val id: String, val awtType: Int) {
    DEFAULT("default", 0),
    CROSSHAIR("crosshair", 1),
    TEXT("text", 2),
    WAIT("wait", 3),
    SW_RESIZE("sw_resize", 4),
    SE_RESIZE("se_resize", 5),
    NW_RESIZE("nw_resize", 6),
    NE_RESIZE("ne_resize", 7),
    N_RESIZE("n_resize", 8),
    S_RESIZE("s_resize", 9),
    W_RESIZE("w_resize", 10),
    E_RESIZE("e_resize", 11),
    HAND("hand", 12),
    MOVE("move", 13),
    ;

    /** Whether this shape means "an editable text field is under the finger". */
    val isText: Boolean get() = this == TEXT

    /** Whether the pointer is over something draggable / resizable. */
    val isGrip: Boolean get() = id.endsWith("_resize") || this == MOVE

    /** Short Chinese label for the diagnostics panel. */
    val label: String
        get() = when (this) {
            DEFAULT -> "箭头"
            CROSSHAIR -> "十字"
            TEXT -> "文本"
            WAIT -> "忙碌"
            HAND -> "手型"
            MOVE -> "移动"
            else -> "缩放"
        }

    companion object {
        /** Map a core id; anything unknown degrades to [DEFAULT] (never throws). */
        fun fromId(id: String?): AwtCursorKind =
            AwtCursorKind.entries.firstOrNull { it.id == id } ?: DEFAULT

        /** Map a `java.awt.Cursor` type; a custom/bitmap cursor becomes [DEFAULT]. */
        fun fromAwtType(type: Int): AwtCursorKind =
            AwtCursorKind.entries.firstOrNull { it.awtType == type } ?: DEFAULT
    }
}

/** What a control message says (mirrors `AwtControlKind` in the core). */
enum class AwtControlKind(val id: String, val code: Int) {
    CURSOR("cursor", 1),
    TITLE("title", 2),
    CLIPBOARD_SET("clipboard_set", 3),
    CLIPBOARD_REQUEST("clipboard_request", 4),
    BEEP("beep", 5),
    SCREEN_SIZE("screen_size", 6),
    IME_SHOW("ime_show", 7),
    IME_HIDE("ime_hide", 8),
    WINDOW_OPENED("window_opened", 9),
    WINDOW_CLOSED("window_closed", 10),
    BYE("bye", 11),
    ;

    companion object {
        fun fromId(id: String?): AwtControlKind? = AwtControlKind.entries.firstOrNull { it.id == id }
        fun fromCode(code: Int): AwtControlKind? = AwtControlKind.entries.firstOrNull { it.code == code }
    }
}

/** What a control *record* (launcher → JVM) answers. */
enum class AwtReplyKind(val id: String, val code: Int) {
    CLIPBOARD("clipboard", 1),
    CLIPBOARD_EMPTY("clipboard_empty", 2),
    PONG("pong", 3),
    ;

    companion object {
        fun fromCode(code: Int): AwtReplyKind? = AwtReplyKind.entries.firstOrNull { it.code == code }
    }
}

/** Caret of the focused text component, in *desktop* pixels. */
data class AwtImeCaret(val x: Int, val y: Int, val lineHeight: Int)

/** One window / dialog cacio reported. */
data class AwtWindowInfo(val id: Int, val title: String)

/**
 * One control message, as parsed from `awtDrainControl`.
 *
 * The kind-specific fields are pre-extracted by the core, so the UI never has to
 * know what the raw `a` / `b` / `c` integers mean.
 */
data class AwtControlMessage(
    val kind: AwtControlKind,
    val seq: Int = 0,
    /** Title / clipboard text / goodbye reason. */
    val text: String = "",
    val cursor: AwtCursorKind = AwtCursorKind.DEFAULT,
    /** `screen_size` width, or `ime_show` x. */
    val width: Int = 0,
    val height: Int = 0,
    val caret: AwtImeCaret? = null,
    val window: Int = 0,
) {
    companion object {
        internal fun parse(obj: JsonValue.Obj): AwtControlMessage? {
            val kind = AwtControlKind.fromId(obj.str("kind")) ?: return null
            return AwtControlMessage(
                kind = kind,
                seq = obj.int("seq") ?: 0,
                text = obj.str("text") ?: "",
                cursor = AwtCursorKind.fromId(obj.str("cursor")),
                width = obj.int("width") ?: 0,
                height = obj.int("height") ?: 0,
                caret = if (kind == AwtControlKind.IME_SHOW) {
                    AwtImeCaret(
                        x = obj.int("x") ?: 0,
                        y = obj.int("y") ?: 0,
                        lineHeight = obj.int("line_height") ?: 0,
                    )
                } else {
                    null
                },
                window = obj.int("window") ?: 0,
            )
        }
    }
}

/**
 * Last-write-wins projection of the control stream: what the UI *renders* every
 * frame (as opposed to the messages, whose side effects fire exactly once).
 */
data class AwtControlState(
    val cursor: AwtCursorKind = AwtCursorKind.DEFAULT,
    val title: String? = null,
    val caret: AwtImeCaret? = null,
    val wantsKeyboard: Boolean = false,
    /** Text the JVM copied and the UI has not pushed to Android yet. */
    val clipboardOut: String? = null,
    /** Unanswered `Clipboard.getContents()` calls. */
    val clipboardRequests: Int = 0,
    val windows: List<AwtWindowInfo> = emptyList(),
    val beeps: Long = 0,
    /** Why the JVM-side bridge said goodbye (`null` while alive). */
    val bye: String? = null,
) {
    /** One-line summary for the diagnostics card. */
    fun describe(): String = buildString {
        append(cursor.label)
        title?.let { append(" · ").append(it) }
        if (wantsKeyboard) append(" · 需要键盘")
        if (windows.isNotEmpty()) append(" · ${windows.size} 窗口")
        bye?.let { append(" · 已结束：").append(it) }
    }

    companion object {
        val EMPTY = AwtControlState()

        internal fun parse(obj: JsonValue.Obj?): AwtControlState {
            if (obj == null) return EMPTY
            val ime = obj.obj("ime")
            return AwtControlState(
                cursor = AwtCursorKind.fromId(obj.str("cursor")),
                title = obj.str("title"),
                caret = ime?.let {
                    AwtImeCaret(
                        x = it.int("x") ?: 0,
                        y = it.int("y") ?: 0,
                        lineHeight = it.int("line_height") ?: 0,
                    )
                },
                wantsKeyboard = obj.bool("wants_keyboard") ?: (ime != null),
                clipboardOut = obj.str("clipboard_out"),
                clipboardRequests = obj.int("clipboard_requests") ?: 0,
                windows = (obj.entries["windows"] as? JsonValue.Arr)
                    ?.items
                    ?.mapNotNull { it as? JsonValue.Obj }
                    ?.map { AwtWindowInfo(it.int("id") ?: 0, it.str("title") ?: "") }
                    ?: emptyList(),
                beeps = obj.long("beeps") ?: 0,
                bye = obj.str("bye"),
            )
        }
    }
}

/** Result of one `awtDrainControl`: the messages *and* the projection. */
data class AwtControlBatch(
    val messages: List<AwtControlMessage> = emptyList(),
    val state: AwtControlState = AwtControlState.EMPTY,
    val clipboardRequests: Int = 0,
    val error: String? = null,
) {
    val isEmpty: Boolean get() = messages.isEmpty()

    /** Seq of the newest pending clipboard request, if the JVM wants a paste. */
    val clipboardRequestSeq: Int?
        get() = messages.lastOrNull { it.kind == AwtControlKind.CLIPBOARD_REQUEST }?.seq

    /** Text the JVM copied in this batch (push it onto the Android clipboard). */
    val clipboardSet: String?
        get() = messages.lastOrNull { it.kind == AwtControlKind.CLIPBOARD_SET }?.text

    /** How many `Toolkit.beep()` calls this batch carries. */
    val beeps: Int get() = messages.count { it.kind == AwtControlKind.BEEP }

    companion object {
        val EMPTY = AwtControlBatch()

        fun failed(message: String): AwtControlBatch = AwtControlBatch(error = message)

        /** Parse `RustBridge.awtDrainControl`. Malformed input is never fatal. */
        fun parse(json: String?): AwtControlBatch {
            val root = parseJson(json.orEmpty()) as? JsonValue.Obj
                ?: return failed("无法解析核心返回的 AWT 控制信息")
            root.str("error")?.let { return failed(it) }
            val messages = (root.entries["control"] as? JsonValue.Arr)
                ?.items
                ?.mapNotNull { it as? JsonValue.Obj }
                ?.mapNotNull { AwtControlMessage.parse(it) }
                ?: emptyList()
            return AwtControlBatch(
                messages = messages,
                state = AwtControlState.parse(root.obj("state")),
                clipboardRequests = root.int("clipboard_requests") ?: 0,
            )
        }
    }
}

/** A control answer the UI sends back (`RustBridge.awtControl`). */
data class AwtControlRequest(
    /** Clipboard text, or `null` together with [clipboardEmpty] for "no text". */
    val clipboard: String? = null,
    /** Answer "the Android clipboard holds no text" (still an answer!). */
    val clipboardEmpty: Boolean = false,
    /** Answer only this request (`null` = every pending one). */
    val clipboardSeq: Int? = null,
    val pong: Int? = null,
    /** Forget the projection (arrow cursor, no keyboard). */
    val reset: Boolean = false,
) {
    fun toJson(): String {
        val entries = linkedMapOf<String, JsonValue>()
        if (clipboardEmpty) {
            entries["clipboard_empty"] = JsonValue.Bool(true)
        } else if (clipboard != null) {
            entries["clipboard"] = JsonValue.Str(clipboard)
        }
        clipboardSeq?.let { entries["clipboard_seq"] = JsonValue.Num(it.toDouble()) }
        pong?.let { entries["pong"] = JsonValue.Num(it.toDouble()) }
        if (reset) entries["reset"] = JsonValue.Bool(true)
        return JsonValue.Obj(entries).toJsonString()
    }
}

/** Result of one `awtControl` call. */
data class AwtControlResult(
    val queued: Int = 0,
    val clipboardRequests: Int = 0,
    val state: AwtControlState = AwtControlState.EMPTY,
    val error: String? = null,
) {
    companion object {
        val EMPTY = AwtControlResult()

        fun parse(json: String?): AwtControlResult {
            val root = parseJson(json.orEmpty()) as? JsonValue.Obj
                ?: return AwtControlResult(error = "无法解析核心返回的 AWT 控制结果")
            root.str("error")?.let { return AwtControlResult(error = it) }
            return AwtControlResult(
                queued = root.int("queued") ?: 0,
                clipboardRequests = root.int("clipboard_requests") ?: 0,
                state = AwtControlState.parse(root.obj("state")),
            )
        }
    }
}

/**
 * The `RCAC` control record on the wire (little-endian), mirroring
 * `launch::awt::AwtControl`:
 *
 * ```text
 * 0  u32 magic("RCAC")   4 u16 version   6 u16 kind
 * 8  u32 seq            12 i32 a        16 i32 b       20 i32 c
 * 24 u32 payload_len    28 u32 flags
 * 32 …  payload_len bytes of UTF-8 text
 * ```
 *
 * The header has the same shape as a frame header (version at 4, payload length
 * at 24) but a different magic, which is what lets one stream reader demultiplex
 * both without ever losing alignment.
 */
object AwtControlWire {
    /** `"RCAC"` — RC launcher AWT control. */
    const val CONTROL_MAGIC: Int = 0x5243_4143

    const val CONTROL_VERSION: Int = 1

    const val CONTROL_HEADER_LEN: Int = 32

    /** Largest text payload one control message may carry. */
    const val MAX_CONTROL_TEXT: Int = 64 * 1024

    /** Reserved event-record id marking a control record (launcher → JVM). */
    const val CONTROL_EVENT_ID: Int = 0x7263_0001

    /** Text bytes one control record carries. */
    const val CONTROL_CHUNK_BYTES: Int = 8

    /** Encode a control message. */
    fun encode(
        kind: AwtControlKind,
        seq: Int = 0,
        a: Int = 0,
        b: Int = 0,
        c: Int = 0,
        flags: Int = 0,
        text: String = "",
    ): ByteArray {
        val payload = text.toByteArray(Charsets.UTF_8)
        require(payload.size <= MAX_CONTROL_TEXT) {
            "control text of ${payload.size} bytes exceeds $MAX_CONTROL_TEXT"
        }
        val out = ByteArray(CONTROL_HEADER_LEN + payload.size)
        putInt(out, 0, CONTROL_MAGIC)
        putShort(out, 4, CONTROL_VERSION)
        putShort(out, 6, kind.code)
        putInt(out, 8, seq)
        putInt(out, 12, a)
        putInt(out, 16, b)
        putInt(out, 20, c)
        putInt(out, 24, payload.size)
        putInt(out, 28, flags)
        payload.copyInto(out, CONTROL_HEADER_LEN)
        return out
    }

    /** `setCursor` — the diagnostics self-test uses this. */
    fun encodeCursor(cursor: AwtCursorKind): ByteArray =
        encode(AwtControlKind.CURSOR, a = cursor.awtType)

    /** The JVM copied something. */
    fun encodeClipboardSet(text: String): ByteArray =
        encode(AwtControlKind.CLIPBOARD_SET, text = text)

    /** The JVM wants to paste. */
    fun encodeClipboardRequest(seq: Int): ByteArray =
        encode(AwtControlKind.CLIPBOARD_REQUEST, seq = seq)

    /** A text component wants input at a desktop pixel. */
    fun encodeImeShow(x: Int, y: Int, lineHeight: Int): ByteArray =
        encode(AwtControlKind.IME_SHOW, a = x, b = y, c = lineHeight)

    /** The managed screen size cacio really uses. */
    fun encodeScreenSize(width: Int, height: Int): ByteArray =
        encode(AwtControlKind.SCREEN_SIZE, a = width, b = height)

    /**
     * Parse a control message, or `null` when [bytes] is not a valid one (bad
     * magic / version / kind, absurd or truncated payload, invalid UTF-8).
     * Never throws.
     */
    fun decode(bytes: ByteArray): AwtControlMessage? {
        if (bytes.size < CONTROL_HEADER_LEN) return null
        if (getInt(bytes, 0) != CONTROL_MAGIC) return null
        if (getShort(bytes, 4) != CONTROL_VERSION) return null
        val kind = AwtControlKind.fromCode(getShort(bytes, 6)) ?: return null
        val payloadLen = getInt(bytes, 24)
        if (payloadLen < 0 || payloadLen > MAX_CONTROL_TEXT) return null
        if (bytes.size - CONTROL_HEADER_LEN < payloadLen) return null
        val a = getInt(bytes, 12)
        val b = getInt(bytes, 16)
        val c = getInt(bytes, 20)
        val raw = bytes.copyOfRange(CONTROL_HEADER_LEN, CONTROL_HEADER_LEN + payloadLen)
        val text = raw.toString(Charsets.UTF_8)
        // `String(bytes, UTF_8)` replaces invalid sequences instead of failing, so
        // check by re-encoding: a title is eventually shown to a human.
        if (!text.toByteArray(Charsets.UTF_8).contentEquals(raw)) return null
        return AwtControlMessage(
            kind = kind,
            seq = getInt(bytes, 8),
            text = text,
            cursor = if (kind == AwtControlKind.CURSOR) {
                AwtCursorKind.fromAwtType(a)
            } else {
                AwtCursorKind.DEFAULT
            },
            width = a,
            height = b,
            caret = if (kind == AwtControlKind.IME_SHOW) AwtImeCaret(a, b, c) else null,
            window = a,
        )
    }

    /** Whether an event record is a control record rather than an AWT event. */
    fun isControlRecord(record: AwtEventRecord): Boolean = record.id == CONTROL_EVENT_ID

    /**
     * Encode a reply as fixed-length control records — the launcher → JVM
     * direction, which stays a `readFully(32)` loop on the JVM side.
     *
     * Field mapping: `x` = reply kind, `y` = seq, `button` = chunk index,
     * `keyCode` = chunk count, `keyChar` = bytes valid in this chunk,
     * `modifiers`/`wheel` = the 8 text bytes.
     */
    fun encodeReply(kind: AwtReplyKind, seq: Int, text: String): List<AwtEventRecord> {
        val bytes = text.toByteArray(Charsets.UTF_8)
        val total = maxOf(1, (bytes.size + CONTROL_CHUNK_BYTES - 1) / CONTROL_CHUNK_BYTES)
        return (0 until total).map { index ->
            val start = index * CONTROL_CHUNK_BYTES
            val len = minOf(CONTROL_CHUNK_BYTES, bytes.size - start).coerceAtLeast(0)
            val chunk = ByteArray(CONTROL_CHUNK_BYTES)
            if (len > 0) bytes.copyInto(chunk, 0, start, start + len)
            AwtEventRecord(
                id = CONTROL_EVENT_ID,
                x = kind.code,
                y = seq,
                button = index,
                keyCode = total,
                keyChar = len,
                modifiers = getInt(chunk, 0),
                wheel = getInt(chunk, 4),
            )
        }
    }

    /**
     * Re-assemble a reply (the JVM-side algorithm). Returns `null` for an
     * inconsistent run — a wrong order, a missing chunk, a bogus length — rather
     * than handing a torn string back.
     */
    fun decodeReply(records: List<AwtEventRecord>): AwtControlReply? {
        val first = records.firstOrNull() ?: return null
        if (!isControlRecord(first)) return null
        val kind = AwtReplyKind.fromCode(first.x) ?: return null
        val total = first.keyCode
        if (total <= 0 || total != records.size) return null
        val out = ByteArray(total * CONTROL_CHUNK_BYTES)
        var length = 0
        records.forEachIndexed { index, record ->
            if (!isControlRecord(record) || record.x != first.x || record.y != first.y) return null
            if (record.button != index || record.keyCode != total) return null
            val len = record.keyChar
            if (len < 0 || len > CONTROL_CHUNK_BYTES) return null
            val chunk = ByteArray(CONTROL_CHUNK_BYTES)
            putInt(chunk, 0, record.modifiers)
            putInt(chunk, 4, record.wheel)
            chunk.copyInto(out, length, 0, len)
            length += len
        }
        val bytes = out.copyOf(length)
        val text = bytes.toString(Charsets.UTF_8)
        if (!text.toByteArray(Charsets.UTF_8).contentEquals(bytes)) return null
        return AwtControlReply(kind, first.y, text)
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

/** A reassembled launcher → JVM reply. */
data class AwtControlReply(val kind: AwtReplyKind, val seq: Int, val text: String)
