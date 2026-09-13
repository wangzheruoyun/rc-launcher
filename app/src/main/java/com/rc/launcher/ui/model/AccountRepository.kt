package com.rc.launcher.ui.model

import android.content.Context
import android.content.SharedPreferences
import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File

/**
 * Persistence / backend contract for the account-management UI (task 16).
 *
 * The production implementation ([RustAccountRepository]) delegates to the Rust
 * core through [com.rc.launcher.core.RustBridge] (which owns encrypted token
 * storage, the Microsoft device-code flow and proactive token refresh -- task
 * 5). An [InMemoryAccountRepository] keeps the [com.rc.launcher.ui.viewmodel
 * .AccountViewModel] fully unit-testable on the JVM, mirroring the repository
 * split used by tasks 14 / 15.
 */
interface AccountRepository {
    /** Load redacted accounts (no secrets). */
    suspend fun list(): List<Account>

    /** Add an offline account. Throws on empty input. */
    suspend fun addOffline(name: String): Account

    /**
     * Begin the Microsoft device-code flow; returns the challenge to display.
     * `redirectUri` (task 28) optionally specifies a custom callback address,
     * e.g. pointing to the embedded `microsoft_auth.html` asset.
     */
    suspend fun beginMicrosoft(redirectUri: String? = null): DeviceCodeChallenge

    /** Complete the Microsoft device-code flow for [challenge]; returns the account. */
    suspend fun completeMicrosoft(challenge: DeviceCodeChallenge): Account

    /** Remove an account by uuid; returns true if something was removed. */
    suspend fun remove(uuid: String): Boolean

    /** Force-refresh a Microsoft token; returns the refreshed account, or null. */
    suspend fun refresh(uuid: String): Account?

    /** Return a fresh account, transparently refreshing if expiring; null if absent. */
    suspend fun ensureFresh(uuid: String): Account?

    /** Discover an external auth server's metadata by URL (task 10). */
    suspend fun beginThirdParty(serverUrl: String): ThirdPartyServerInfo

    /** Complete a third-party login (Authlib-Injector / token relay); returns the account. */
    suspend fun completeThirdParty(login: ThirdPartyLogin): Account?

    /** The persisted active-account uuid (UI selection), or null. */
    fun getActiveId(): String?

    /** Persist the active-account uuid. */
    fun setActiveId(id: String?)

    // === Skin preview (task 22) ============================================

    /** Fetch skin + cape metadata for [uuid] from Mojang's session API. */
    suspend fun fetchSkin(uuid: String): SkinModel?

    /**
     * Upload a custom skin for [uuid]. `model` is "slim" or "classic".
     * `skinBase64` is the PNG bytes base64-encoded.
     */
    suspend fun uploadSkin(uuid: String, model: String, skinBase64: String): Boolean
}

/**
 * Process-local account store used by previews and unit tests. It re-implements
 * the small subset of the Rust `AccountManager` behaviour the UI needs (offline
 * add, a simulated device-code flow, remove, refresh, ensure-fresh) so the
 * ViewModel can be exercised without the native library.
 */
class InMemoryAccountRepository(
    initial: List<Account> = emptyList(),
) : AccountRepository {
    private val store = LinkedHashMap<String, Account>().apply { for (a in initial) put(a.uuid, a) }
    private var activeId: String? = null

    override suspend fun list(): List<Account> = store.values.toList()

    override suspend fun addOffline(name: String): Account {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) throw IllegalArgumentException("offline username must not be empty")
        val acc = OfflineAccount(uuid = offlineUuid(trimmed), username = trimmed)
        store[acc.uuid] = acc
        return acc
    }

    override suspend fun beginMicrosoft(redirectUri: String?): DeviceCodeChallenge = DeviceCodeChallenge(
        userCode = "ABCD-EFGH",
        deviceCode = "simulated-device-code",
        verificationUrl = "https://microsoft.com/devicelogin",
        expiresIn = 900,
        interval = 5,
        message = "请在浏览器中打开 https://microsoft.com/devicelogin 并输入验证码 ABCD-EFGH 完成登录。",
        redirectUri = redirectUri,
    )

    override suspend fun completeMicrosoft(challenge: DeviceCodeChallenge): Account {
        val acc = MicrosoftAccount(
            uuid = "11111111-1111-1111-1111-111111111111",
            username = "Player",
            clientId = "00000000402b5328",
            xuid = "2535414195331971",
            expiresAt = nowSecs() + 86400,
            msExpiresAt = nowSecs() + 3600,
        )
        store[acc.uuid] = acc
        return acc
    }

    override suspend fun remove(uuid: String): Boolean {
        val had = store.remove(uuid) != null
        if (had && activeId == uuid) activeId = null
        return had
    }

    override suspend fun refresh(uuid: String): Account? {
        val cur = store[uuid] ?: return null
        if (cur !is MicrosoftAccount) return null
        val refreshed = cur.copy(expiresAt = nowSecs() + 86400, msExpiresAt = nowSecs() + 3600)
        store[uuid] = refreshed
        return refreshed
    }

    override suspend fun ensureFresh(uuid: String): Account? {
        val cur = store[uuid] ?: return null
        return if (cur is MicrosoftAccount && cur.tokenStatus != TokenStatus.VALID) refresh(uuid) else cur
    }

    override suspend fun beginThirdParty(serverUrl: String): ThirdPartyServerInfo =
        ThirdPartyServerInfo(serverUrl = serverUrl, serverName = serverUrl, links = emptyList())

    override suspend fun completeThirdParty(login: ThirdPartyLogin): Account? {
        val uuid = offlineUuid(login.username.ifBlank { "thirdparty" })
        val acc = ThirdPartyAccount(
            uuid = uuid,
            username = login.username.ifBlank { "Player" },
            provider = login.provider,
            serverUrl = login.serverUrl,
            serverName = login.serverName,
        )
        store[uuid] = acc
        return acc
    }

    override suspend fun fetchSkin(uuid: String): SkinModel? {
        val acc = store[uuid] ?: return null
        if (acc !is MicrosoftAccount) return null
        // Simulate a skin response for the in-memory test backend.
        return SkinModel(
            uuid = acc.uuid,
            skinUrl = "https://textures.minecraft.net/texture/test",
            capeUrl = null,
            fetchedAt = nowSecs(),
            cachedAt = nowSecs(),
            source = SkinSource.OFFICIAL,
            hash = null,
            model = "default",
        )
    }

    override suspend fun uploadSkin(uuid: String, model: String, skinBase64: String): Boolean {
        // In-memory: just succeed if the account exists and is Microsoft.
        val acc = store[uuid] ?: return false
        return acc is MicrosoftAccount
    }

    override fun getActiveId(): String? = activeId
    override fun setActiveId(id: String?) {
        activeId = id?.takeIf { store.containsKey(it) }
    }
}

/**
 * [RustBridge]-backed [AccountRepository] (task 5 / task 16).
 *
 * Every call runs on [Dispatchers.IO] (the Microsoft flows block inside the JNI
 * boundary) and is wrapped in [runCatching] so a missing / failed native library
 * degrades gracefully to an empty account list instead of crashing the UI
 * (task 19). The active-account selection is a UI concern, so it is persisted in
 * a private [SharedPreferences] rather than in the (encrypted) Rust token store.
 */
class RustAccountRepository(
    context: Context,
) : AccountRepository {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)
    private var activeId: String? = prefs.getString(KEY_ACTIVE, null)

    /**
     * The redirect URI for the Microsoft OAuth browser callback. Written into
     * the cache directory at init time so the page is always properly translated
     * and never shows raw template placeholders (task 28).
     */
    private val redirectUri: String

    init {
        // (Re)configure the global Rust account store. A real product build
        // passes a Keystore-backed `key_hex` + on-disk `path` here (task 5 /
        // FCL); we use the in-memory store so the bridge stays crash-free when
        // no encrypted vault is provisioned yet.
        //
        // Task 28: generate the callback page from the Rust core (which has the
        // i18n template + current language) and write it to a writable cache
        // file. The `file:///android_asset/microsoft_auth.html` asset is
        // read-only and may contain unresolved template placeholders in stripped
        // builds, so we prefer the cache copy. The cache file URI is then
        // registered as the OAuth redirect_uri so Microsoft redirects back to a
        // properly localised "you may close this page" page after sign-in.
        // When a proxy is configured in Settings, pass it through so the
        // Microsoft/Xbox/Mojang token-exchange calls can punch through the
        // Great Firewall.
        val defaultUri = runCatching { RustBridge.authDefaultRedirectUri() }
            .getOrDefault("file:///android_asset/microsoft_auth.html")
        val cacheFile = File(context.cacheDir, "microsoft_auth.html")
        val cacheUri = runCatching { cacheFile.toURI().toString() }.getOrNull()
        if (cacheUri != null) {
            runCatching {
                val html = RustBridge.authGetCallbackHtml()
                if (html.isNotBlank()) {
                    cacheFile.writeText(html)
                }
            }
            redirectUri = cacheUri
        } else {
            redirectUri = defaultUri
        }
        val proxyUrl = context.applicationContext
            .getSharedPreferences("rc_settings", Context.MODE_PRIVATE)
            .getString("proxy_url", "")
        val config = JSONObject().apply {
            put("redirect_uri", redirectUri)
            if (!proxyUrl.isNullOrBlank()) put("proxy", proxyUrl)
        }.toString()
        runCatching { RustBridge.authInit(config) }
    }


    override suspend fun list(): List<Account> = withContext(Dispatchers.IO) {
        runCatching { parseAccountList(RustBridge.authListAccounts()) }.getOrDefault(emptyList())
    }

    override suspend fun addOffline(name: String): Account = withContext(Dispatchers.IO) {
        if (name.isBlank()) throw IllegalArgumentException("offline username must not be empty")
        val json = runCatching { RustBridge.authAddOfflineAccount(name) }
            .getOrElse { e -> throw IllegalStateException(e.message ?: "addOffline failed", e) }
        parseAccount(json) ?: throw IllegalStateException("malformed account from core: $json")
    }

    override suspend fun beginMicrosoft(redirectUri: String?): DeviceCodeChallenge = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authBeginMicrosoft() }
            .getOrElse { e -> throw IllegalStateException("beginMicrosoft failed", e) }
        // Check for error JSON before parsing as a device code challenge.
        AuthLoginException.fromErrorJson(json)?.let { throw it }
        parseDeviceCode(json) ?: throw IllegalStateException("malformed device code from core: $json")
    }

    override suspend fun completeMicrosoft(challenge: DeviceCodeChallenge): Account = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authCompleteMicrosoft(challenge.toJsonString()) }
            .getOrElse { e -> throw IllegalStateException("completeMicrosoft failed", e) }
        // Check for error JSON (carries cn_fallback_hint on network failures — task 28).
        AuthLoginException.fromErrorJson(json)?.let { throw it }
        parseAccount(json) ?: throw IllegalStateException("malformed account from core: $json")
    }

    override suspend fun remove(uuid: String): Boolean = withContext(Dispatchers.IO) {
        runCatching {
            val json = RustBridge.authRemoveAccount(uuid)
            (parseJson(json) as? JsonValue.Obj)
                ?.let { (it.entries["removed"] as? JsonValue.Bool)?.value } ?: false
        }.getOrDefault(false)
    }

    override suspend fun refresh(uuid: String): Account? = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authRefreshAccount(uuid) }
            .getOrElse { e -> throw IllegalStateException("refresh failed", e) }
        // Surface cn_fallback_hint on network failures (task 28).
        AuthLoginException.fromErrorJson(json)?.let { throw it }
        if (json.contains("\"error\"")) null else parseAccount(json)
    }

    override suspend fun ensureFresh(uuid: String): Account? = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authEnsureFresh(uuid) }
            .getOrElse { e -> throw IllegalStateException("ensureFresh failed", e) }
        // Surface cn_fallback_hint on network failures (task 28).
        AuthLoginException.fromErrorJson(json)?.let { throw it }
        if (json.contains("\"error\"")) null else parseAccount(json)
    }

    override suspend fun beginThirdParty(serverUrl: String): ThirdPartyServerInfo = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authBeginThirdParty(serverUrl) }
            .getOrElse { e -> throw IllegalStateException("beginThirdParty failed", e) }
        AuthLoginException.fromErrorJson(json)?.let { throw it }
        parseThirdPartyServerInfo(json) ?: throw IllegalStateException("malformed server info from core: $json")
    }

    override suspend fun completeThirdParty(login: ThirdPartyLogin): Account? = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authCompleteThirdParty(login.toJsonString()) }
            .getOrElse { e -> throw IllegalStateException("completeThirdParty failed", e) }
        AuthLoginException.fromErrorJson(json)?.let { throw it }
        if (json.contains("\"error\"")) null else parseAccount(json)
    }

    override suspend fun fetchSkin(uuid: String): SkinModel? = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authFetchSkin(uuid) }
            .getOrElse { return@withContext null }
        parseSkinModel(json)
    }

    override suspend fun uploadSkin(uuid: String, model: String, skinBase64: String): Boolean = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authUploadSkin(uuid, model, skinBase64) }
            .getOrElse { return@withContext false }
        // Success is {"ok":true}; anything with "error" is a failure.
        !json.contains("\"error\"")
    }

    override fun getActiveId(): String? = activeId

    override fun setActiveId(id: String?) {
        activeId = id
        prefs.edit().putString(KEY_ACTIVE, id).apply()
    }

    companion object {
        private const val NAME = "rc_accounts"
        private const val KEY_ACTIVE = "active_account_uuid"
    }
}

/**
 * Process-wide account repository holder, mirroring [SettingsRepositories].
 * The real implementation is installed from [com.rc.launcher.RcApplication
 * .onCreate]; until then (previews / unit tests) a throwaway
 * [InMemoryAccountRepository] is used so the UI never crashes for lack of the
 * native core.
 */
object AccountRepositories {
    @Volatile
    private var _default: AccountRepository? = null

    val default: AccountRepository
        get() = _default ?: InMemoryAccountRepository().also { _default = it }

    fun install(repository: AccountRepository) {
        _default = repository
    }
}
