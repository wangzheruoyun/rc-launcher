package com.rc.launcher.ui.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the account model + JSON (de)serialization (task 16). */
class AccountModelTest {

    @Test
    fun offlineUuid_isDeterministicAndWellFormed() {
        val a = offlineUuid("Steve")
        val b = offlineUuid("Steve")
        val c = offlineUuid("Alex")
        assertEquals(a, b)
        assertFalse(a == c)
        assertTrue(a.matches(Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")))
    }

    @Test
    fun parseAccount_microsoft_matchesRustShape() {
        val json = """{"type":"microsoft","uuid":"u1","username":"Notch","client_id":"cid",""" +
            """"access_token":"","refresh_token":"","xuid":"123","expires_at":100,"ms_expires_at":90}"""
        val acc = parseAccount(json)
        assertNotNull(acc)
        assertTrue(acc is MicrosoftAccount)
        val m = acc as MicrosoftAccount
        assertEquals("u1", m.uuid)
        assertEquals("Notch", m.username)
        assertEquals("cid", m.clientId)
        assertEquals("123", m.xuid)
        assertEquals(100, m.expiresAt)
        assertEquals(90, m.msExpiresAt)
        assertEquals(AccountKind.MICROSOFT, m.kind)
    }

    @Test
    fun parseAccount_offline_matchesRustShape() {
        val json = """{"type":"offline","uuid":"u2","username":"Steve"}"""
        val acc = parseAccount(json)
        assertTrue(acc is OfflineAccount)
        val o = acc as OfflineAccount
        assertEquals("u2", o.uuid)
        assertEquals("Steve", o.username)
        assertEquals(AccountKind.OFFLINE, o.kind)
    }

    @Test
    fun parseAccountList_parsesArray() {
        val json = """[{"type":"offline","uuid":"u1","username":"A"},""" +
            """{"type":"microsoft","uuid":"u2","username":"B","client_id":"c",""" +
            """"access_token":"","refresh_token":"","xuid":null,"expires_at":1,"ms_expires_at":1}]"""
        val list = parseAccountList(json)
        assertEquals(2, list.size)
        assertTrue(list[0] is OfflineAccount)
        assertTrue(list[1] is MicrosoftAccount)
    }

    @Test
    fun parseAccountList_handlesErrorObject() {
        // The Rust core returns {"error": "..."} on failure; treat as empty.
        assertEquals(0, parseAccountList("""{"error":"boom"}""").size)
        assertNull(parseAccount("""{"error":"boom"}"""))
    }

    @Test
    fun parseDeviceCode_roundTrips() {
        val challenge = DeviceCodeChallenge(
            userCode = "ABCD-EFGH",
            deviceCode = "dc",
            verificationUrl = "https://x",
            expiresIn = 900,
            interval = 5,
            message = "msg",
        )
        val round = parseDeviceCode(challenge.toJsonString())
        assertNotNull(round)
        assertEquals("ABCD-EFGH", round!!.userCode)
        assertEquals("https://x", round.verificationUrl)
        assertEquals(900, round.expiresIn)
        assertEquals(5, round.interval)
        assertEquals("msg", round.message)
    }

    @Test
    fun tokenStatus_classify() {
        assertEquals(TokenStatus.EXPIRED, TokenStatus.classify(100, 90, 100))
        // within threshold -> EXPIRING
        assertEquals(TokenStatus.EXPIRING, TokenStatus.classify(100, 90, 50, 300))
        // far in the future -> VALID
        assertEquals(TokenStatus.VALID, TokenStatus.classify(1000, 1000, 50, 300))
        // no timestamps -> UNKNOWN
        assertEquals(TokenStatus.UNKNOWN, TokenStatus.classify(0, 0, 50))
    }

    @Test
    fun microsoft_tokenStatusReflectsExpiry() {
        val expired = MicrosoftAccount(uuid = "x", username = "y", expiresAt = 10, msExpiresAt = 10)
        val valid = MicrosoftAccount(uuid = "x", username = "y", expiresAt = nowSecs() + 9999, msExpiresAt = nowSecs() + 9999)
        assertEquals(TokenStatus.EXPIRED, expired.tokenStatus)
        assertTrue(valid.isExpired.not())
        assertEquals(TokenStatus.VALID, valid.tokenStatus)
    }

    @Test
    fun formatDuration_formatsVariousMagnitudes() {
        assertEquals("已过期", formatDuration(0))
        assertEquals("已过期", formatDuration(-5))
        assertEquals("45秒", formatDuration(45))
        assertEquals("3分4秒", formatDuration(3 * 60 + 4))
        assertEquals("5小时12分", formatDuration(5 * 3600 + 12 * 60))
        assertEquals("2天3小时", formatDuration(2 * 86400 + 3 * 3600))
    }

    @Test
    fun microsoft_remainingSecs_reflectsExpiry() {
        val future = MicrosoftAccount(uuid = "x", username = "y", expiresAt = nowSecs() + 120, msExpiresAt = nowSecs() + 120)
        assertTrue(future.remainingSecs in 100..130)
        val past = MicrosoftAccount(uuid = "x", username = "y", expiresAt = nowSecs() - 100, msExpiresAt = nowSecs() - 100)
        assertEquals(0, past.remainingSecs)
    }
}
