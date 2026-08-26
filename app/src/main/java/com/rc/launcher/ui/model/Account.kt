package com.rc.launcher.ui.model

import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import com.rc.launcher.ui.model.json.toJsonString

/**
 * Account model for the account-management UI (task 16).
 *
 * Mirrors the Rust core's `crate::auth::model::{Account, MicrosoftAccount,
 * OfflineAccount, DeviceCodeChallenge}` serde shapes so the JSON emitted by
 * [com.rc.launcher.core.RustBridge] can be parsed without loss. The UI only ever
 * holds *redacted* accounts (no access / refresh tokens) -- exactly what
 * `AccountManager::summaries` returns -- so secrets never cross the FFI
 * boundary into the Compose layer.
 *
 * The file is pure Kotlin (no Android imports) and (de)serialises with the
 * project's dependency-free [com.rc.launcher.ui.model.json.MiniJson], keeping it
 * fully unit-testable on the JVM.
 */

/** Discriminator for the account type (mirrors Rust `AccountKind`). */
enum class AccountKind(val code: String, val label: String) {
    MICROSOFT("microsoft", "正版 · Microsoft"),
    OFFLINE("offline", "离线 · Offline");

    companion object {
        fun fromCode(code: String?): AccountKind =
            entries.firstOrNull { it.code == code } ?: OFFLINE
    }
}

/** Token health for a Microsoft account, visualised in the UI (task 16). */
enum class TokenStatus(val label: String) {
    VALID("有效"),
    EXPIRING("即将过期"),
    EXPIRED("已过期"),
    UNKNOWN("未知");

    companion object {
        /** Proactive-refresh classification (mirrors Rust `MicrosoftAccount::needs_refresh`). */
        fun classify(
            expiresAt: Long,
            msExpiresAt: Long,
            now: Long,
            thresholdSecs: Long = 300,
        ): TokenStatus {
            if (expiresAt <= 0 && msExpiresAt <= 0) return UNKNOWN
            if (now >= expiresAt || now >= msExpiresAt) return EXPIRED
            if (now + thresholdSecs >= expiresAt || now + thresholdSecs >= msExpiresAt) return EXPIRING
            return VALID
        }
    }
}

/** A unified account: either Microsoft-authenticated or offline. */
sealed interface Account {
    val uuid: String
    val username: String
    val kind: AccountKind

    /** Mojang-style avatar URL for a quick skin preview (task 16). */
    fun skinUrl(overlay: Boolean = true): String {
        val base = "https://mc-heads.net/avatar/$uuid/64"
        return if (overlay) "$base?overlay" else base
    }

    companion object {
        /** Fallback avatar shown when an account has no UUID yet. */
        const val DEFAULT_SKIN = "https://mc-heads.net/avatar/steve/64"
    }
}

/** Redacted Microsoft account (no access / refresh tokens). */
data class MicrosoftAccount(
    override val uuid: String = "",
    override val username: String = "",
    val clientId: String = "",
    val xuid: String? = null,
    val expiresAt: Long = 0,
    val msExpiresAt: Long = 0,
) : Account {
    override val kind: AccountKind get() = AccountKind.MICROSOFT

    /** Proactive-refresh classification for the current time. */
    val tokenStatus: TokenStatus
        get() = TokenStatus.classify(expiresAt, msExpiresAt, nowSecs())

    val isExpired: Boolean get() = tokenStatus == TokenStatus.EXPIRED
    val isExpiring: Boolean get() = tokenStatus == TokenStatus.EXPIRING
}

/** Offline (cracked / no-network) account. */
data class OfflineAccount(
    override val uuid: String = "",
    override val username: String = "",
) : Account {
    override val kind: AccountKind get() = AccountKind.OFFLINE
}

/**
 * Device-code challenge shown to the user during the Microsoft login flow
 * (task 16). The `message` is a ready-to-display instruction string from the
 * identity provider; `userCode` / `verificationUrl` are surfaced as copyable
 * fields.
 */
data class DeviceCodeChallenge(
    val userCode: String = "",
    val deviceCode: String = "",
    val verificationUrl: String = "",
    val expiresIn: Long = 0,
    val interval: Long = 5,
    val message: String = "",
) {
    /** Serialize back to the Rust core's challenge JSON (for `authCompleteMicrosoft`). */
    fun toJsonString(): String = JsonValue.Obj(
        mapOf(
            "user_code" to JsonValue.Str(userCode),
            "device_code" to JsonValue.Str(deviceCode),
            "verification_uri" to JsonValue.Str(verificationUrl),
            "expires_in" to JsonValue.Num(expiresIn.toDouble()),
            "interval" to JsonValue.Num(interval.toDouble()),
            "message" to JsonValue.Str(message),
        ),
    ).toJsonString()
}

// ============================================================================
// JSON (de)serialization via MiniJson -- shapes match the Rust core 1:1.
// ============================================================================

private fun JsonValue.Obj.str(key: String): String? = (entries[key] as? JsonValue.Str)?.value
private fun JsonValue.Obj.num(key: String): Double? = (entries[key] as? JsonValue.Num)?.value

private fun JsonValue.toAccount(): Account? {
    if (this !is JsonValue.Obj) return null
    return when (str("type")) {
        "microsoft" -> MicrosoftAccount(
            uuid = str("uuid").orEmpty(),
            username = str("username").orEmpty(),
            clientId = str("client_id").orEmpty(),
            xuid = str("xuid"),
            expiresAt = num("expires_at")?.toLong() ?: 0,
            msExpiresAt = num("ms_expires_at")?.toLong() ?: 0,
        )
        "offline" -> OfflineAccount(
            uuid = str("uuid").orEmpty(),
            username = str("username").orEmpty(),
        )
        else -> null
    }
}

/** Parse a single [Account] from JSON text, or null if [text] is malformed. */
fun parseAccount(text: String): Account? = (parseJson(text) as? JsonValue.Obj)?.toAccount()

/** Parse a JSON array of accounts (e.g. `authListAccounts`), or empty on error. */
fun parseAccountList(text: String): List<Account> {
    val root = parseJson(text) ?: return emptyList()
    if (root !is JsonValue.Arr) return emptyList()
    return root.items.mapNotNull { it.toAccount() }
}

/** Parse a [DeviceCodeChallenge] from JSON text, or null if [text] is malformed. */
fun parseDeviceCode(text: String): DeviceCodeChallenge? {
    val root = parseJson(text) as? JsonValue.Obj ?: return null
    return DeviceCodeChallenge(
        userCode = root.str("user_code").orEmpty(),
        deviceCode = root.str("device_code").orEmpty(),
        verificationUrl = root.str("verification_uri").orEmpty(),
        expiresIn = root.num("expires_in")?.toLong() ?: 0,
        interval = root.num("interval")?.toLong() ?: 5,
        message = root.str("message").orEmpty(),
    )
}

/** Current unix epoch seconds (mirrors Rust `auth::model::now_secs`). */
fun nowSecs(): Long = System.currentTimeMillis() / 1000L

/**
 * Deterministic offline UUID (mirrors Rust `offline_account_model` /
 * `UUID.nameUUIDFromBytes` -- MD5 name-based version 3 UUID). Kept pure-JVM so
 * the [InMemoryAccountRepository] produces stable ids without the native core.
 */
fun offlineUuid(username: String): String {
    val digest = java.security.MessageDigest.getInstance("MD5")
    val bytes = digest.digest("OfflinePlayer:$username".toByteArray(Charsets.UTF_8))
    bytes[6] = (bytes[6].toInt() and 0x0f or 0x30).toByte() // version 3
    bytes[8] = (bytes[8].toInt() and 0x3f or 0x80).toByte() // RFC 4122 variant
    val hex = bytes.joinToString("") { "%02x".format(it) }
    return "${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-" +
        "${hex.substring(16, 20)}-${hex.substring(20, 32)}"
}


/**
 * Seconds until the Microsoft access token expires. Returns 0 when the expiry is
 * unknown (<= 0) or already in the past. The clock is sampled at call time so it
 * is safe to read from Compose UI without a fixed "now" injection (task 16).
 */
val MicrosoftAccount.remainingSecs: Long
    get() {
        val delta = expiresAt - nowSecs()
        return if (delta < 0) 0 else delta
    }

/**
 * Human-readable duration, e.g. "2天3小时", "5小时12分", "3分4秒", "45秒".
 * A non-positive [totalSecs] renders as "已过期". Pure + unit-tested (task 16).
 */
fun formatDuration(totalSecs: Long): String {
    if (totalSecs <= 0) return "已过期"
    val days = totalSecs / 86400
    val hours = (totalSecs % 86400) / 3600
    val mins = (totalSecs % 3600) / 60
    val secs = totalSecs % 60
    return when {
        days > 0 -> "${days}天${hours}小时"
        hours > 0 -> "${hours}小时${mins}分"
        mins > 0 -> "${mins}分${secs}秒"
        else -> "${secs}秒"
    }
}
