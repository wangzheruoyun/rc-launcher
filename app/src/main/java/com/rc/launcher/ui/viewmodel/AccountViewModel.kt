package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import com.rc.launcher.ui.model.Account
import com.rc.launcher.ui.model.AccountRepositories
import com.rc.launcher.ui.model.AccountRepository
import com.rc.launcher.ui.model.DeviceCodeChallenge
import com.rc.launcher.ui.model.MicrosoftAccount
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

    /** Step 1 of the Microsoft login: fetch a device-code challenge. */
    suspend fun beginMicrosoftLogin() {
        _loginState.value = LoginState.SigningIn
        try {
            val challenge = repository.beginMicrosoft()
            _loginState.value = LoginState.AwaitingDeviceCode(challenge)
        } catch (e: Throwable) {
            _loginState.value = LoginState.Error(e.message ?: "获取设备码失败")
        }
    }

    /** Step 2 of the Microsoft login: complete the flow for the pending challenge. */
    suspend fun completeMicrosoftLogin() {
        val challenge = (_loginState.value as? LoginState.AwaitingDeviceCode)?.challenge ?: return
        _loginState.value = LoginState.SigningIn
        try {
            val account = repository.completeMicrosoft(challenge)
            _loginState.value = LoginState.Idle
            selectAccount(account.uuid)
            loadAccounts()
        } catch (e: Throwable) {
            _loginState.value = LoginState.Error(e.message ?: "微软登录失败")
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
            _error.value = e.message ?: "刷新令牌失败"
        }
    }

    /** Ensure a Microsoft account has a fresh token (refresh only if expiring). */
    suspend fun ensureFresh(uuid: String) {
        try {
            repository.ensureFresh(uuid)
            loadAccounts()
        } catch (e: Throwable) {
            _error.value = e.message ?: "更新令牌失败"
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
}
