package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import com.rc.launcher.ui.model.Account
import com.rc.launcher.ui.model.AccountRepositories
import com.rc.launcher.ui.model.AccountRepository
import com.rc.launcher.ui.model.DeviceCodeChallenge
import com.rc.launcher.ui.model.SkinModel
import com.rc.launcher.ui.model.MicrosoftAccount
import com.rc.launcher.ui.model.ThirdPartyLogin
import com.rc.launcher.ui.model.ThirdPartyServerInfo
import com.rc.launcher.ui.model.TokenStatus

/**
 * State container for the account-management screen (task 16).
 *
 * It owns the account list, the active-account selection and a small state
 * machine for the Microsoft device-code login flow. Every mutator runs the
 * backend call through the injected [AccountRepository] and keeps the UI purely
 * a function of the exposed [StateFlow]s -- so login status is always
 * visualisable (account cards show the active badge + token health) and the
 * whole screen stays unit-testable on the JVM with an
 * [com.rc.launcher.ui.model.InMemoryAccountRepository].
 *
 * The Microsoft flows block inside the Rust core; they are therefore `suspend`
 * and the screen launches them on [Dispatchers.IO] (mirrors [MainViewModel]'s
 * IO dispatch for native calls).
 */
class AccountViewModel(
    private val repository: AccountRepository = AccountRepositories.default,
) : ViewModel() {

    private val _accounts = MutableStateFlow<List<Account>>(emptyList())
    val accounts: StateFlow<List<Account>> = _accounts.asStateFlow()

    private val _activeId = MutableStateFlow<String?>(repository.getActiveId())
    val activeId: StateFlow<String?> = _activeId.asStateFlow()

    private val _activeAccount = MutableStateFlow<Account?>(null)
    val activeAccount: StateFlow<Account?> = _activeAccount.asStateFlow()

    private val _loginState = MutableStateFlow<LoginState>(LoginState.Idle)
    val loginState: StateFlow<LoginState> = _loginState.asStateFlow()

    /** Last operation error to surface in the UI (cleared via [clearError]). */
    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    /**
     * Reload the account list and reconcile the active selection. Any Microsoft
     * account whose token is EXPIRING/EXPIRED is proactively healed via
     * [AccountRepository.ensureFresh] so the displayed token status stays
     * truthful and the active identity never silently lapses (task 16).
     */
    suspend fun loadAccounts() {
        val result = runCatching { repository.list() }
        if (result.isFailure) {
            _error.value = result.exceptionOrNull()?.message ?: "加载账户失败"
            return
        }
        val list = result.getOrDefault(emptyList())
        val healed = list.map { acc ->
            if (acc is MicrosoftAccount && acc.tokenStatus != TokenStatus.VALID) {
                runCatching { repository.ensureFresh(acc.uuid) }.getOrNull() ?: acc
            } else {
                acc
            }
        }
        _accounts.value = healed
        reconcileActive(healed)
    }

    private fun reconcileActive(list: List<Account>) {
        val cur = _activeId.value
        if (cur == null || list.none { it.uuid == cur }) {
            // Prefer the first premium account, then any account.
            _activeId.value = (list.firstOrNull { it is MicrosoftAccount } ?: list.firstOrNull())?.uuid
        }
        _activeAccount.value = list.firstOrNull { it.uuid == _activeId.value }
        repository.setActiveId(_activeId.value)
    }

    /** Mark [uuid] as the active (selected) account. */
    fun selectAccount(uuid: String) {
        _activeId.value = uuid
        _activeAccount.value = _accounts.value.firstOrNull { it.uuid == uuid }
        repository.setActiveId(uuid)
    }

    /** Add an offline account (suspend; runs on the caller's dispatcher). */
    suspend fun addOffline(name: String) {
        try {
            if (name.isBlank()) throw IllegalArgumentException("用户名不能为空")
            repository.addOffline(name)
            loadAccounts()
        } catch (e: Throwable) {
            _error.value = e.message ?: "添加离线账号失败"
        }
    }

    /**
     * Step 1 of the Microsoft login: fetch a device-code challenge.
     * `redirectUri` (task 28) optionally specifies a custom callback address,
     * e.g. pointing to the embedded `microsoft_auth.html` asset.
     */
    suspend fun beginMicrosoftLogin(redirectUri: String? = null) {
        _loginState.value = LoginState.SigningIn
        try {
            val challenge = repository.beginMicrosoft(redirectUri)
            _loginState.value = LoginState.AwaitingDeviceCode(challenge)
        } catch (e: Throwable) {
            // Surface the mainland-China network fallback hint (task 28) when the
            // device-code request fails due to a network-level error.
            val hint = (e as? com.rc.launcher.ui.model.AuthLoginException)?.cnFallbackHint
            val msg = e.message ?: "获取设备码失败"
            _loginState.value = LoginState.Error(
                if (hint != null) "$msg\n\n$hint" else msg
            )
        }
    }

    /**
     * Step 2 of the Microsoft login: complete the flow for the pending challenge.
     * If the failure is a network-level error, the [AuthLoginException]'s
     * `cn_fallbackHint` is appended to the message so the user sees a proxy /
     * mirror suggestion (task 28).
     */
    suspend fun completeMicrosoftLogin() {
        val challenge = (_loginState.value as? LoginState.AwaitingDeviceCode)?.challenge ?: return
        _loginState.value = LoginState.SigningIn
        try {
            val account = repository.completeMicrosoft(challenge)
            _loginState.value = LoginState.Idle
            selectAccount(account.uuid)
            loadAccounts()
        } catch (e: Throwable) {
            val hint = (e as? com.rc.launcher.ui.model.AuthLoginException)?.cnFallbackHint
            val msg = e.message ?: "微软登录失败"
            _loginState.value = LoginState.Error(
                if (hint != null) "$msg\n\n$hint" else msg
            )
        }
    }

    /** Step 1 of third-party login: discover the external auth server's metadata. */
    suspend fun beginThirdPartyDiscovery(serverUrl: String) {
        _loginState.value = LoginState.ThirdPartySigningIn
        try {
            val info = repository.beginThirdParty(serverUrl)
            _loginState.value = LoginState.ThirdPartyServer(info)
        } catch (e: Throwable) {
            val hint = (e as? com.rc.launcher.ui.model.AuthLoginException)?.cnFallbackHint
            val msg = e.message ?: "获取第三方验证服务器信息失败"
            _loginState.value = LoginState.Error(
                if (hint != null) "$msg\n\n$hint" else msg
            )
        }
    }

    /** Step 2 of third-party login: authenticate against the external server. */
    suspend fun completeThirdPartyLogin(login: ThirdPartyLogin) {
        _loginState.value = LoginState.ThirdPartySigningIn
        try {
            val account = repository.completeThirdParty(login)
            if (account != null) {
                _loginState.value = LoginState.Idle
                selectAccount(account.uuid)
                loadAccounts()
            } else {
                _loginState.value = LoginState.Error("第三方登录失败")
            }
        } catch (e: Throwable) {
            val hint = (e as? com.rc.launcher.ui.model.AuthLoginException)?.cnFallbackHint
            val msg = e.message ?: "第三方登录失败"
            _loginState.value = LoginState.Error(
                if (hint != null) "$msg\n\n$hint" else msg
            )
        }
    }

    /** Dismiss the login flow and return to the idle state. */
    fun cancelLogin() {
        _loginState.value = LoginState.Idle
    }

    /** Remove an account by uuid. */
    suspend fun removeAccount(uuid: String) {
        try {
            val removed = repository.remove(uuid)
            if (removed && _activeId.value == uuid) _activeId.value = null
            loadAccounts()
        } catch (e: Throwable) {
            _error.value = e.message ?: "删除账号失败"
        }
    }

    /** Force-refresh a Microsoft account's token. */
    suspend fun refreshAccount(uuid: String) {
        try {
            repository.refresh(uuid)
            loadAccounts()
        } catch (e: Throwable) {
            val hint = (e as? com.rc.launcher.ui.model.AuthLoginException)?.cnFallbackHint
            val msg = e.message ?: "刷新令牌失败"
            _error.value = if (hint != null) "$msg\n\n$hint" else msg
        }
    }

    /** Ensure a Microsoft account has a fresh token (refresh only if expiring). */
    suspend fun ensureFresh(uuid: String) {
        try {
            repository.ensureFresh(uuid)
            loadAccounts()
        } catch (e: Throwable) {
            val hint = (e as? com.rc.launcher.ui.model.AuthLoginException)?.cnFallbackHint
            val msg = e.message ?: "更新令牌失败"
            _error.value = if (hint != null) "$msg\n\n$hint" else msg
        }
    }

    /** Refresh every Microsoft account's token at once (toolbar action). */
    suspend fun refreshAllMicrosoft() {
        val microsoft = _accounts.value.filterIsInstance<MicrosoftAccount>()
        if (microsoft.isEmpty()) return
        var failure: String? = null
        for (acc in microsoft) {
            runCatching { repository.refresh(acc.uuid) }.onFailure { failure = it.message ?: "刷新令牌失败" }
        }
        loadAccounts()
        failure?.let { _error.value = it }
    }

    /** Clear the last [error] message. */
    fun clearError() {
        _error.value = null
    }

    // === Skin preview state (task 22) =======================================

    /** Current skin model for the selected account, or null when not loaded. */
    private val _skinModel = MutableStateFlow<SkinModel?>(null)
    val skinModel: StateFlow<SkinModel?> = _skinModel.asStateFlow()

    /** Error message for skin fetch/upload operations. */
    private val _skinError = MutableStateFlow<String?>(null)
    val skinError: StateFlow<String?> = _skinError.asStateFlow()

    /** Whether a skin upload is in flight. */
    private val _isUploading = MutableStateFlow(false)
    val isUploading: StateFlow<Boolean> = _isUploading.asStateFlow()

    /** Whether skin data is being fetched from the network. */
    private val _isFetchingSkin = MutableStateFlow(false)
    val isFetchingSkin: StateFlow<Boolean> = _isFetchingSkin.asStateFlow()

    /**
     * Fetch the skin model for [uuid] from Mojang's session profile API.
     * Runs on [Dispatchers.IO]; the result is cached in the ViewModel's
     * state so the UI can show it immediately on the next load (offline).
     */
    suspend fun loadSkin(uuid: String) {
        _isFetchingSkin.value = true
        _skinError.value = null
        try {
            val model = repository.fetchSkin(uuid)
            _skinModel.value = model
        } catch (e: Throwable) {
            _skinError.value = e.message ?: "获取皮肤失败"
        } finally {
            _isFetchingSkin.value = false
        }
    }

    /**
     * Upload a custom skin for [uuid]. `model` is "slim" or "classic";
     * `skinBase64` is the PNG bytes base64-encoded.
     */
    suspend fun uploadSkin(uuid: String, model: String, skinBase64: String) {
        _isUploading.value = true
        _skinError.value = null
        try {
            val ok = repository.uploadSkin(uuid, model, skinBase64)
            if (!ok) {
                _skinError.value = "皮肤上传失败"
            } else {
                // Re-fetch the updated skin model after upload.
                loadSkin(uuid)
            }
        } catch (e: Throwable) {
            _skinError.value = e.message ?: "皮肤上传失败"
        } finally {
            _isUploading.value = false
        }
    }

    /** Clear the skin-specific error message. */
    fun clearSkinError() {
        _skinError.value = null
    }

    /** Clear the skin model (e.g. when switching accounts). */
    fun clearSkin() {
        _skinModel.value = null
        _skinError.value = null
    }
}

/** State of the Microsoft device-code login flow surfaced by [AccountViewModel]. */
sealed interface LoginState {
    /** No login in progress. */
    data object Idle : LoginState

    /** A backend call is in flight (getting the code / completing the login). */
    data object SigningIn : LoginState

    /** Waiting for the user to authenticate in the browser. */
    data class AwaitingDeviceCode(val challenge: DeviceCodeChallenge) : LoginState

    /** The flow failed; [message] is shown to the user. */
    data class Error(val message: String) : LoginState

    /** Discovering a third-party auth server (network call in flight). */
    data object ThirdPartySigningIn : LoginState

    /** A third-party auth server was discovered; show its name + register link. */
    data class ThirdPartyServer(val info: ThirdPartyServerInfo) : LoginState
}
