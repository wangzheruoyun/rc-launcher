package com.rc.launcher.ui.model.json

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the dependency-free [MiniJson] parser/serializer (task 15). */
class MiniJsonTest {

    @Test
    fun parse_fullGrammar() {
        val json = """{"a":1,"b":[true,false,null,"x"],"c":-3.5,"d":"hi\n","e":{"f":2}}"""
        val root = parseJson(json) as JsonValue.Obj
        assertEquals(1.0, (root.entries["a"] as JsonValue.Num).value, 0.0)
        val arr = root.entries["b"] as JsonValue.Arr
        assertEquals(4, arr.items.size)
        assertTrue(arr.items[0] is JsonValue.Bool)
        assertEquals(true, (arr.items[0] as JsonValue.Bool).value)
        assertTrue(arr.items[2] is JsonValue.Null)
        assertEquals("x", (arr.items[3] as JsonValue.Str).value)
        assertEquals(-3.5, (root.entries["c"] as JsonValue.Num).value, 0.0)
        assertEquals("hi\n", (root.entries["d"] as JsonValue.Str).value)
        val nested = root.entries["e"] as JsonValue.Obj
        assertEquals(2.0, (nested.entries["f"] as JsonValue.Num).value, 0.0)
    }

    @Test
    fun write_thenParse_roundTrips() {
        val value = JsonValue.Obj(
            mapOf(
                "id" to JsonValue.Str("custom_1"),
                "n" to JsonValue.Num(12.0),
                "arr" to JsonValue.Arr(listOf(JsonValue.Str("a"), JsonValue.Bool(false))),
            ),
        )
        val text = value.toJsonString()
        val back = parseJson(text) as JsonValue.Obj
        assertEquals("custom_1", (back.entries["id"] as JsonValue.Str).value)
        assertEquals(12.0, (back.entries["n"] as JsonValue.Num).value, 0.0)
        assertEquals(2, (back.entries["arr"] as JsonValue.Arr).items.size)
    }

    @Test
    fun integerNumbers_avoidScientificNotation() {
        val text = JsonValue.Num(1700.0).toJsonString()
        assertEquals("1700", text)
    }

    @Test
    fun parse_malformed_returnsNull() {
        assertNull(parseJson("not json"))
        assertNull(parseJson("{\"a\":}"))
        assertNull(parseJson("{\"a\":1"))
        assertNull(parseJson("[1,2,"))
        assertNull(parseJson("\"unterminated"))
        assertNull(parseJson("{\"a\":1} trailing"))
        assertNull(parseJson(""))
    }

    @Test
    fun parse_rejectsControlCharsInStrings() {
        assertNull(parseJson("{\"a\":\"\u0001\"}"))
    }

    @Test
    fun parse_topLevelScalar_isValid() {
        assertTrue(parseJson("42") is JsonValue.Num)
        assertTrue(parseJson("true") is JsonValue.Bool)
        assertTrue(parseJson("[1,2,3]") is JsonValue.Arr)
    }

    @Test
    fun escapes_roundTrip() {
        val original = "line1\nline2\ttab\"quote\\slash / 中文"
        val text = JsonValue.Str(original).toJsonString()
        assertEquals("\"line1\\nline2\\ttab\\\"quote\\\\slash / 中文\"", text)
        val parsed = (parseJson(text) as JsonValue.Str).value
        assertEquals(original, parsed)
    }

    @Test
    fun parsesPrettyPrintedObjectsWithWhitespaceAfterCommas() {
        // Regression: the object loop resumed straight after the `,` without
        // skipping whitespace, so any indented / pretty-printed payload was
        // rejected (a number followed by a newline was the common shape).
        val pretty = """
            {
              "a": 1,
              "b": 2.5,
              "c": true,
              "d": null,
              "e": "x",
              "f": [1, 2],
              "g": { "h": 3 }
            }
        """.trimIndent()
        val obj = parseJson(pretty) as JsonValue.Obj
        assertEquals(7, obj.entries.size)
        assertEquals(1.0, (obj.entries["a"] as JsonValue.Num).value, 0.0)
        assertEquals(2.5, (obj.entries["b"] as JsonValue.Num).value, 0.0)
        assertEquals(true, (obj.entries["c"] as JsonValue.Bool).value)
        assertEquals(JsonValue.Null, obj.entries["d"])
        assertEquals("x", (obj.entries["e"] as JsonValue.Str).value)
        assertEquals(2, (obj.entries["f"] as JsonValue.Arr).items.size)
        assertEquals(1, (obj.entries["g"] as JsonValue.Obj).entries.size)
        // Every whitespace flavour, and CRLF.
        assertNotNull(parseJson("{\"a\":1,\r\n\t\"b\":2}"))
        assertNotNull(parseJson("{\n\t\"a\" : 1 , \"b\" : 2\n}"))
        // Still rejects genuinely malformed input.
        assertNull(parseJson("{\"a\":1,}"))
        assertNull(parseJson("{\"a\":1 \"b\":2}"))
    }
}
