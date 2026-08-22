package com.rc.launcher.ui.model

import android.content.Context
import android.content.SharedPreferences
import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

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

    /** Begin the Microsoft device-code flow; returns the challenge to display. */
    suspend fun beginMicrosoft(): DeviceCodeChallenge

    /** Complete the Microsoft device-code flow for [challenge]; returns the account. */
    suspend fun completeMicrosoft(challenge: DeviceCodeChallenge): Account

    /** Remove an account by uuid; returns true if something was removed. */
    suspend fun remove(uuid: String): Boolean

    /** Force-refresh a Microsoft token; returns the refreshed account, or null. */
    suspend fun refresh(uuid: String): Account?

    /** Return a fresh account, transparently refreshing if expiring; null if absent. */
    suspend fun ensureFresh(uuid: String): Account?

    /** The persisted active-account uuid (UI selection), or null. */
    fun getActiveId(): String?

    /** Persist the active-account uuid. */
    fun setActiveId(id: String?)
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

    override suspend fun beginMicrosoft(): DeviceCodeChallenge = DeviceCodeChallenge(
        userCode = "ABCD-EFGH",
        deviceCode = "simulated-device-code",
        verificationUrl = "https://microsoft.com/devicelogin",
        expiresIn = 900,
        interval = 5,
        message = "请在浏览器中打开 https://microsoft.com/devicelogin 并输入代码 ABCD-EFGH 完成登录。",
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

    init {
        // (Re)configure the global Rust account store. A real product build
        // passes a Keystore-backed `key_hex` + on-disk `path` here (task 5 /
        // FCL); we use the in-memory store so the bridge stays crash-free when
        // no encrypted vault is provisioned yet.
        runCatching { RustBridge.authInit("{}") }
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

    override suspend fun beginMicrosoft(): DeviceCodeChallenge = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authBeginMicrosoft() }
            .getOrElse { e -> throw IllegalStateException(e.message ?: "beginMicrosoft failed", e) }
        parseDeviceCode(json) ?: throw IllegalStateException("malformed device code from core: $json")
    }

    override suspend fun completeMicrosoft(challenge: DeviceCodeChallenge): Account = withContext(Dispatchers.IO) {
        val json = runCatching { RustBridge.authCompleteMicrosoft(challenge.toJsonString()) }
            .getOrElse { e -> throw IllegalStateException(e.message ?: "completeMicrosoft failed", e) }
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
        runCatching {
            val json = RustBridge.authRefreshAccount(uuid)
            if (json.contains("\"error\"")) null else parseAccount(json)
        }.getOrNull()
    }

    override suspend fun ensureFresh(uuid: String): Account? = withContext(Dispatchers.IO) {
        runCatching {
            val json = RustBridge.authEnsureFresh(uuid)
            if (json.contains("\"error\"")) null else parseAccount(json)
        }.getOrNull()
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
