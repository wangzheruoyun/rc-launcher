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
    OFFLINE("offline", "离线 · Offline"),
    THIRD_PARTY("thirdparty", "第三方 · Third-Party");

    companion object {
        fun fromCode(code: String?): AccountKind =
            entries.firstOrNull { it.code == code } ?: OFFLINE
    }
}

/** Source of a player skin texture (mirrors Rust `auth::model::SkinSource`). */
enum class SkinSource(val code: String) {
    OFFICIAL("official"),
    CUSTOM("custom");

    companion object {
        fun fromCode(code: String?): SkinSource =
            entries.firstOrNull { it.code == code } ?: OFFICIAL
    }
}

/**
 * Skin (and cape) metadata for a Minecraft profile (task 22).
 *
 * Mirrors the Rust core's `crate::auth::model::SkinModel`. The `skinUrl` and
 * `capeUrl` point at Mojang's texture servers; the UI downloads the PNG and
 * caches it for offline display. `cachedAt` (epoch seconds) tracks when the
 * bytes were last written to the image cache.
 */
data class SkinModel(
    val uuid: String = "",
    val skinUrl: String = "",
    val capeUrl: String? = null,
    val fetchedAt: Long = 0,
    val cachedAt: Long = 0,
    val source: SkinSource = SkinSource.OFFICIAL,
    val hash: String? = null,
    val model: String = "",
) {
    /** True when the skin PNG bytes have been cached locally (offline displayable). */
    fun isCached(): Boolean = cachedAt > 0 && skinUrl.isNotBlank()

    /** Cache key for the on-disk PNG cache, derived from the UUID. */
    fun cacheKey(): String = "skin_" + uuid.replace("-", "")
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
 * Third-party account (external Yggdrasil auth server / Authlib-Injector, or a
 * third-party Microsoft-OAuth token relay). The UI only ever holds *redacted*
 * accounts, so [accessToken] / [clientToken] / [relayPayload] are empty here
 * (they live encrypted in the Rust core's token store). [serverUrl] is what the
 * game receives through the authlib-injector agent at launch (task 10).
 */
data class ThirdPartyAccount(
    override val uuid: String = "",
    override val username: String = "",
    val provider: String = "authlib_injector",
    val serverUrl: String = "",
    val serverName: String = "",
    val accessToken: String = "",
    val clientToken: String = "",
    val expiresAt: Long = 0,
    val relayPayload: String? = null,
) : Account {
    override val kind: AccountKind get() = AccountKind.THIRD_PARTY

    /** Proactive-refresh classification for the current time. */
    val tokenStatus: TokenStatus
        get() = if (expiresAt <= 0) TokenStatus.UNKNOWN
        else TokenStatus.classify(expiresAt, expiresAt, nowSecs())
}

/**
 * Device-code challenge shown to the user during the Microsoft login flow
 * (task 16). The `message` is a ready-to-display instruction string from the
 * identity provider; `userCode` / `verificationUrl` are surfaced as copyable
 * fields.
 *
 * `redirectUri` (task 28) carries an optional custom callback address set via
 * `authInit("redirect_uri")` or the Settings screen. When non-null the UI can
 * open the embedded `microsoft_auth.html` callback page at this address and
 * pass it through to `authCompleteMicrosoft`.
 */
data class DeviceCodeChallenge(
    val userCode: String = "",
    val deviceCode: String = "",
    val verificationUrl: String = "",
    val expiresIn: Long = 0,
    val interval: Long = 5,
    val message: String = "",
    /** Optional custom redirect URI for the OAuth browser redirect (task 28). */
    val redirectUri: String? = null,
) {
    /**
     * Serialize back to the Rust core's challenge JSON (for `authCompleteMicrosoft`).
     * `redirect_uri` is omitted when null so the JSON matches the 3.6 / 3.7+
     * challenge shapes without it.
     */
    fun toJsonString(): String {
        val entries = LinkedHashMap<String, JsonValue>(7)
        entries["user_code"] = JsonValue.Str(userCode)
        entries["device_code"] = JsonValue.Str(deviceCode)
        entries["verification_uri"] = JsonValue.Str(verificationUrl)
        entries["expires_in"] = JsonValue.Num(expiresIn.toDouble())
        entries["interval"] = JsonValue.Num(interval.toDouble())
        entries["message"] = JsonValue.Str(message)
        redirectUri?.let { entries["redirect_uri"] = JsonValue.Str(it) }
        return JsonValue.Obj(entries).toJsonString()
    }
}


/**
 * Metadata discovered from an external auth server (`RustBridge.authBeginThirdParty`),
 * surfaced before the user types credentials (task 10).
 */
data class ThirdPartyServerInfo(
    val serverUrl: String = "",
    val serverName: String = "",
    val links: List<ThirdPartyLink> = emptyList(),
)

/** A registration / homepage link surfaced from an auth server's metadata. */
data class ThirdPartyLink(
    val label: String = "",
    val url: String = "",
)

/**
 * Login request for a third-party (Authlib-Injector / token relay) account.
 * Mirrors the Rust core's `ThirdPartyLogin` serde shape (camelCase JSON) so the
 * [com.rc.launcher.core.RustBridge.authCompleteThirdParty] payload is 1:1 (task 10).
 */
data class ThirdPartyLogin(
    val provider: String = "authlib_injector",
    val serverUrl: String = "",
    val username: String = "",
    val password: String = "",
    val serverName: String = "",
    val relayCode: String? = null,
) {
    fun toJsonString(): String = JsonValue.Obj(
        mapOf(
            "provider" to JsonValue.Str(provider),
            "serverUrl" to JsonValue.Str(serverUrl),
            "serverName" to JsonValue.Str(serverName),
            "username" to JsonValue.Str(username),
            "password" to JsonValue.Str(password),
            "relayCode" to (relayCode?.let { JsonValue.Str(it) } ?: JsonValue.Null),
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
        "thirdparty" -> ThirdPartyAccount(
            uuid = str("uuid").orEmpty(),
            username = str("username").orEmpty(),
            provider = str("provider").orEmpty(),
            serverUrl = str("server_url").orEmpty(),
            serverName = str("server_name").orEmpty(),
            expiresAt = num("expires_at")?.toLong() ?: 0,
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

/** Parse an external auth server's metadata from JSON text (task 10). */
fun parseThirdPartyServerInfo(text: String): ThirdPartyServerInfo? {
    val root = parseJson(text) as? JsonValue.Obj ?: return null
    val links = (root.entries["links"] as? JsonValue.Arr)?.items?.mapNotNull { l ->
        if (l is JsonValue.Obj) ThirdPartyLink(l.str("label").orEmpty(), l.str("url").orEmpty()) else null
    } ?: emptyList()
    return ThirdPartyServerInfo(
        serverUrl = root.str("server_url").orEmpty(),
        serverName = root.str("server_name").orEmpty(),
        links = links,
    )
}

fun parseDeviceCode(text: String): DeviceCodeChallenge? {
    val root = parseJson(text) as? JsonValue.Obj ?: return null
    return DeviceCodeChallenge(
        userCode = root.str("user_code").orEmpty(),
        deviceCode = root.str("device_code").orEmpty(),
        verificationUrl = root.str("verification_uri").orEmpty(),
        expiresIn = root.num("expires_in")?.toLong() ?: 0,
        interval = root.num("interval")?.toLong() ?: 5,
        message = root.str("message").orEmpty(),
        // redirect_uri is optional (task 28); null when absent.
        redirectUri = root.str("redirect_uri"),
    )
}

/** Parse a [SkinModel] from JSON text (e.g. `authFetchSkin` result), or null on error. */
fun parseSkinModel(text: String): SkinModel? {
    val root = parseJson(text) as? JsonValue.Obj ?: return null
    // Error objects from the core should not be parsed as skin data.
    if (root.entries["error"] != null) return null
    return SkinModel(
        uuid = root.str("uuid").orEmpty(),
        skinUrl = root.str("skin_url").orEmpty(),
        capeUrl = root.str("cape_url"),
        fetchedAt = root.num("fetched_at")?.toLong() ?: 0,
        cachedAt = root.num("cached_at")?.toLong() ?: 0,
        source = SkinSource.fromCode(root.str("source")),
        hash = root.str("hash"),
        model = root.str("model").orEmpty(),
    )
}

/**
 * Error returned by a login / refresh / skin operation against the Rust core.
 * Carries the optional `cn_fallback_hint` (task 28) so the UI can surface a
 * mainland-China proxy / mirror suggestion instead of a bare "login failed"
 * toast. The original native error message (if any) is preserved as `cause`.
 */
class AuthLoginException(
    override val message: String,
    val cnFallbackHint: String? = null,
) : IllegalStateException(message) {
    companion object {
        /** Parse a Rust auth error JSON (`{"error":"...","cn_fallback_hint":"..."}`)
         *  into an [AuthLoginException], or null when [json] is not an error. */
        fun fromErrorJson(json: String): AuthLoginException? {
            val root = parseJson(json) as? JsonValue.Obj ?: return null
            val err = root.str("error") ?: return null
            return AuthLoginException(
                message = err,
                cnFallbackHint = root.str("cn_fallback_hint"),
            )
        }
    }
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
