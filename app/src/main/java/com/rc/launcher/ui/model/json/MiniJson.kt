package com.rc.launcher.ui.model.json

/**
 * A tiny, dependency-free JSON model + parser/serializer used to persist custom
 * control layouts (task 15) without pulling in an external serialization
 * library. It is intentionally minimal (objects, arrays, strings, numbers,
 * booleans, null) and *fail-soft*: [parseJson] returns `null` on any malformed
 * input so the repository can gracefully fall back to a built-in layout
 * (task 19 robustness).
 *
 * This file is pure Kotlin (no Android imports) so it is fully unit-testable on
 * the JVM, mirroring the theme engine / settings split used elsewhere.
 */
sealed interface JsonValue {
    data class Obj(val entries: Map<String, JsonValue>) : JsonValue
    data class Arr(val items: List<JsonValue>) : JsonValue
    data class Str(val value: String) : JsonValue
    data class Num(val value: Double) : JsonValue
    data class Bool(val value: Boolean) : JsonValue
    data object Null : JsonValue
}

/** Parse [text] into a [JsonValue], or `null` if it is not valid JSON. */
fun parseJson(text: String): JsonValue? = JsonParser(text).parse()

/** Serialize this value to a compact JSON string. */
fun JsonValue.toJsonString(): String {
    val sb = StringBuilder()
    write(this, sb)
    return sb.toString()
}

// ---- Parsing ----------------------------------------------------------------

private class JsonParser(private val src: String) {
    private var pos = 0

    fun parse(): JsonValue? {
        skipWs()
        val v = parseValue() ?: return null
        skipWs()
        return if (pos == src.length) v else null
    }

    private fun skipWs() {
        while (pos < src.length) {
            val c = src[pos]
            if (c == ' ' || c == '\t' || c == '\n' || c == '\r') pos++
            else break
        }
    }

    private fun peek(): Char? = if (pos < src.length) src[pos] else null

    private fun parseValue(): JsonValue? {
        skipWs()
        return when (peek()) {
            '{' -> parseObject()
            '[' -> parseArray()
            '"' -> parseString()
            't', 'f' -> parseBool()
            'n' -> parseNull()
            '-', in '0'..'9' -> parseNumber()
            else -> null
        }
    }

    private fun parseObject(): JsonValue? {
        pos++ // consume '{'
        skipWs()
        if (peek() == '}') { pos++; return JsonValue.Obj(emptyMap()) }
        val map = LinkedHashMap<String, JsonValue>()
        while (true) {
            val key = parseString() ?: return null
            skipWs()
            if (peek() != ':') return null
            pos++ // consume ':'
            val value = parseValue() ?: return null
            map[key.value] = value
            skipWs()
            when (peek()) {
                ',' -> { pos++; continue }
                '}' -> { pos++; break }
                else -> return null
            }
        }
        return JsonValue.Obj(map)
    }

    private fun parseArray(): JsonValue? {
        pos++ // consume '['
        skipWs()
        if (peek() == ']') { pos++; return JsonValue.Arr(emptyList()) }
        val items = ArrayList<JsonValue>()
        while (true) {
            val value = parseValue() ?: return null
            items.add(value)
            skipWs()
            when (peek()) {
                ',' -> { pos++; continue }
                ']' -> { pos++; break }
                else -> return null
            }
        }
        return JsonValue.Arr(items)
    }

    private fun parseString(): JsonValue? {
        if (peek() != '"') return null
        pos++ // consume opening quote
        val sb = StringBuilder()
        while (pos < src.length) {
            val c = src[pos++]
            when {
                c == '"' -> return JsonValue.Str(sb.toString())
                c == '\\' -> {
                    if (pos >= src.length) return null
                    when (val ec = src[pos++]) {
                        '"' -> sb.append('"')
                        '\\' -> sb.append('\\')
                        '/' -> sb.append('/')
                        'b' -> sb.append('\b')
                        'f' -> sb.append('\u000C')
                        'n' -> sb.append('\n')
                        'r' -> sb.append('\r')
                        't' -> sb.append('\t')
                        'u' -> {
                            if (pos + 4 > src.length) return null
                            val hex = src.substring(pos, pos + 4)
                            val code = hex.toIntOrNull(16) ?: return null
                            sb.append(code.toChar())
                            pos += 4
                        }
                        else -> return null
                    }
                }
                c.code < 0x20 -> return null // control char inside string
                else -> sb.append(c)
            }
        }
        return null // unterminated
    }

    private fun parseNumber(): JsonValue? {
        val start = pos
        if (peek() == '-') pos++
        while (pos < src.length && src[pos] in '0'..'9') pos++
        if (pos < src.length && src[pos] == '.') {
            pos++
            while (pos < src.length && src[pos] in '0'..'9') pos++
        }
        if (pos < src.length && (src[pos] == 'e' || src[pos] == 'E')) {
            pos++
            if (pos < src.length && (src[pos] == '+' || src[pos] == '-')) pos++
            while (pos < src.length && src[pos] in '0'..'9') pos++
        }
        if (pos == start) return null
        val raw = src.substring(start, pos)
        return raw.toDoubleOrNull()?.let { JsonValue.Num(it) }
    }

    private fun parseBool(): JsonValue? = when {
        src.startsWith("true", pos) -> { pos += 4; JsonValue.Bool(true) }
        src.startsWith("false", pos) -> { pos += 5; JsonValue.Bool(false) }
        else -> null
    }

    private fun parseNull(): JsonValue? = if (src.startsWith("null", pos)) {
        pos += 4; JsonValue.Null
    } else null
}

// ---- Writing ----------------------------------------------------------------

private fun write(v: JsonValue, sb: StringBuilder) {
    when (v) {
        is JsonValue.Obj -> {
            sb.append('{')
            v.entries.entries.forEachIndexed { i, (k, value) ->
                if (i > 0) sb.append(',')
                writeString(k, sb)
                sb.append(':')
                write(value, sb)
            }
            sb.append('}')
        }
        is JsonValue.Arr -> {
            sb.append('[')
            v.items.forEachIndexed { i, item ->
                if (i > 0) sb.append(',')
                write(item, sb)
            }
            sb.append(']')
        }
        is JsonValue.Str -> writeString(v.value, sb)
        is JsonValue.Num -> {
            val d = v.value
            sb.append(
                if (d == d.toLong().toDouble() && d.let { it >= Long.MIN_VALUE && it <= Long.MAX_VALUE }) {
                    d.toLong().toString()
                } else {
                    d.toString()
                },
            )
        }
        is JsonValue.Bool -> sb.append(if (v.value) "true" else "false")
        JsonValue.Null -> sb.append("null")
    }
}

private fun writeString(s: String, sb: StringBuilder) {
    sb.append('"')
    for (c in s) {
        when (c) {
            '"' -> sb.append("\\\"")
            '\\' -> sb.append("\\\\")
            '\n' -> sb.append("\\n")
            '\r' -> sb.append("\\r")
            '\t' -> sb.append("\\t")
            '\b' -> sb.append("\\b")
            '\u000C' -> sb.append("\\u000C")
            else -> if (c.code < 0x20) sb.append("\\u%04x".format(c.code))
            else sb.append(c)
        }
    }
    sb.append('"')
}
