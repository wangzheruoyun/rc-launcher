package com.rc.launcher.ui.viewmodel

import com.rc.launcher.ui.model.Account
import com.rc.launcher.ui.model.AccountRepository
import com.rc.launcher.ui.model.InMemoryAccountRepository
import com.rc.launcher.ui.model.MicrosoftAccount
import com.rc.launcher.ui.model.OfflineAccount
import com.rc.launcher.ui.model.TokenStatus
import com.rc.launcher.ui.model.nowSecs
import com.rc.launcher.ui.viewmodel.LoginState
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Unit tests for the account-management view model (task 16). */
class AccountViewModelTest {

    private fun vm(repo: AccountRepository = InMemoryAccountRepository()): AccountViewModel =
        AccountViewModel(repo)

    @Test
    fun loadAccounts_emptyInitially() = runBlocking {
        val v = vm()
        v.loadAccounts()
        assertTrue(v.accounts.value.isEmpty())
        assertNull(v.activeId.value)
    }

    @Test
    fun addOffline_addsAndSelectsActive() = runBlocking {
        val v = vm()
        v.addOffline("Steve")
        assertEquals(1, v.accounts.value.size)
        val acc = v.accounts.value[0]
        assertTrue(acc is OfflineAccount)
        assertEquals("Steve", acc.username)
        assertNotNull(v.activeId.value)
        assertEquals(acc.uuid, v.activeId.value)
        assertEquals(acc.uuid, v.activeAccount.value?.uuid)
    }

    @Test
    fun addOffline_blank_reportsError() = runBlocking {
        val v = vm()
        v.addOffline("   ")
        assertTrue(v.accounts.value.isEmpty())
        assertNotNull(v.error.value)
        v.clearError()
        assertNull(v.error.value)
    }

    @Test
    fun microsoftFlow_beginThenComplete() = runBlocking {
        val v = vm()
        v.beginMicrosoftLogin()
        val state = v.loginState.value
        assertTrue(state is LoginState.AwaitingDeviceCode)
        assertEquals("ABCD-EFGH", (state as LoginState.AwaitingDeviceCode).challenge.userCode)

        v.completeMicrosoftLogin()
        assertTrue(v.loginState.value is LoginState.Idle)
        val acc = v.accounts.value.first()
        assertTrue(acc is MicrosoftAccount)
        assertEquals("Player", acc.username)
        assertEquals(acc.uuid, v.activeId.value)
    }

    @Test
    fun removeAccount_removesAndClearsActive() = runBlocking {
        val v = vm()
        v.addOffline("Steve")
        val id = v.activeId.value!!
        v.removeAccount(id)
        assertTrue(v.accounts.value.isEmpty())
        assertNull(v.activeId.value)
    }

    @Test
    fun selectAccount_changesActive() = runBlocking {
        val v = vm()
        v.addOffline("A")
        v.addOffline("B")
        val ids = v.accounts.value.map { it.uuid }
        assertEquals(2, ids.size)
        v.selectAccount(ids[1])
        assertEquals(ids[1], v.activeId.value)
    }

    @Test
    fun refreshAccount_keepsPremiumValid() = runBlocking {
        val v = vm()
        v.beginMicrosoftLogin()
        v.completeMicrosoftLogin()
        val before = v.accounts.value.first() as MicrosoftAccount
        v.refreshAccount(before.uuid)
        val after = v.accounts.value.first() as MicrosoftAccount
        assertEquals(TokenStatus.VALID, after.tokenStatus)
    }

    @Test
    fun ensureFresh_idempotentForValidToken() = runBlocking {
        val v = vm()
        v.beginMicrosoftLogin()
        v.completeMicrosoftLogin()
        val before = v.accounts.value.first()
        v.ensureFresh(before.uuid)
        val after = v.accounts.value.first()
        assertEquals(before, after)
    }

    @Test
    fun loadAccounts_autoRefreshesExpiringMicrosoft() = runBlocking {
        val expired = MicrosoftAccount(
            uuid = "exp",
            username = "Old",
            expiresAt = nowSecs() - 10,
            msExpiresAt = nowSecs() - 10,
        )
        val v = vm(InMemoryAccountRepository(listOf(expired)))
        v.loadAccounts()
        val acc = v.accounts.value.first() as MicrosoftAccount
        assertEquals(TokenStatus.VALID, acc.tokenStatus)
        assertTrue(acc.expiresAt > nowSecs())
        // The active selection should settle on the (now valid) premium account.
        assertEquals(acc.uuid, v.activeId.value)
    }

    @Test
    fun refreshAllMicrosoft_refreshesEveryMicrosoft() = runBlocking {
        val a = MicrosoftAccount(uuid = "a", username = "A", expiresAt = nowSecs() - 5, msExpiresAt = nowSecs() - 5)
        val b = MicrosoftAccount(uuid = "b", username = "B", expiresAt = nowSecs() + 9999, msExpiresAt = nowSecs() + 9999)
        val v = vm(InMemoryAccountRepository(listOf(a, b)))
        v.loadAccounts()
        v.refreshAllMicrosoft()
        val microsoft = v.accounts.value.filterIsInstance<MicrosoftAccount>()
        assertEquals(2, microsoft.size)
        assertTrue(microsoft.all { it.tokenStatus == TokenStatus.VALID })
    }

    @Test
    fun loadAccounts_surfacesErrorOnFailure() = runBlocking {
        val failing = object : AccountRepository by InMemoryAccountRepository() {
            override suspend fun list(): List<Account> = throw RuntimeException("boom")
        }
        val v = vm(failing)
        v.loadAccounts()
        assertNotNull(v.error.value)
        assertTrue(v.accounts.value.isEmpty())
    }
}
