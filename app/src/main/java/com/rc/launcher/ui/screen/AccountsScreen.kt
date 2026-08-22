package com.rc.launcher.ui.screen

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.model.Account
import com.rc.launcher.ui.model.AccountKind
import com.rc.launcher.ui.model.InMemoryAccountRepository
import com.rc.launcher.ui.model.MicrosoftAccount
import com.rc.launcher.ui.model.OfflineAccount
import com.rc.launcher.ui.model.TokenStatus
import com.rc.launcher.ui.model.nowSecs
import com.rc.launcher.ui.model.offlineUuid
import com.rc.launcher.ui.viewmodel.AccountViewModel
import com.rc.launcher.ui.viewmodel.LoginState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

/**
 * Account-management screen (task 16).
 *
 * The Compose counterpart of FCLCore/auth: it surfaces every account held by the
 * [com.rc.launcher.ui.viewmodel.AccountViewModel] (Microsoft + offline), lets the
 * user switch the active identity, add/remove accounts, preview skins and watch
 * each premium account's token health. The whole screen is a pure function of
 * the ViewModel's [androidx.lifecycle.flow] StateFlows, so login status is
 * always visualisable.
 *
 * Microsoft login is a two-step device-code flow: [AccountViewModel
 * .beginMicrosoftLogin] obtains a challenge (rendered by [MicrosoftLoginDialog])
 * and [AccountViewModel.completeMicrosoftLogin] finishes it after the user
 * authenticates in a browser. Both are `suspend` and launched on
 * [Dispatchers.IO] because they block inside the Rust core's JNI boundary.
 */
@Composable
fun AccountsScreen(
    viewModel: AccountViewModel = viewModel(),
) {
    val accounts by viewModel.accounts.collectAsStateWithLifecycle()
    val activeId by viewModel.activeId.collectAsStateWithLifecycle()
    val activeAccount by viewModel.activeAccount.collectAsStateWithLifecycle()
    val loginState by viewModel.loginState.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()

    val scope = rememberCoroutineScope()
    var showAddOffline by remember { mutableStateOf(false) }

    // Load the account list as soon as the screen appears (the ViewModel keeps
    // the list in a StateFlow; this just seeds it from the repository).
    LaunchedEffect(Unit) { viewModel.loadAccounts() }

    LazyColumn(
        modifier = Modifier.fillMaxSize().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 24.dp),
    ) {
        item {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text("账户管理", style = MaterialTheme.typography.headlineSmall)
                Text(
                    "管理正版 (Microsoft) 与离线账号，查看令牌状态并切换当前登录身份。",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        item { activeAccount?.let { ActiveAccountCard(it) } }

        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = { scope.launch(Dispatchers.IO) { viewModel.beginMicrosoftLogin() } },
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(Icons.Filled.AccountCircle, contentDescription = null)
                    Text("微软登录")
                }
                OutlinedButton(
                    onClick = { showAddOffline = true },
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(Icons.Filled.PersonAdd, contentDescription = null)
                    Text("添加离线账号")
                }
            }
        }

        item {
            error?.let { msg ->
                Surface(
                    color = MaterialTheme.colorScheme.errorContainer,
                    shape = RoundedCornerShape(10.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            msg,
                            modifier = Modifier.weight(1f),
                            color = MaterialTheme.colorScheme.onErrorContainer,
                        )
                        IconButton(onClick = viewModel::clearError) {
                            Icon(Icons.Filled.Close, contentDescription = "关闭")
                        }
                    }
                }
            }
        }

        if (accounts.isEmpty()) {
            item { EmptyAccounts() }
        } else {
            items(accounts, key = { it.uuid }) { account ->
                AccountRow(
                    account = account,
                    isActive = account.uuid == activeId,
                    onSelect = { viewModel.selectAccount(account.uuid) },
                    onRemove = { scope.launch(Dispatchers.IO) { viewModel.removeAccount(account.uuid) } },
                    onRefresh = { scope.launch(Dispatchers.IO) { viewModel.refreshAccount(account.uuid) } },
                )
            }
        }
    }

    if (loginState !is LoginState.Idle) {
        MicrosoftLoginDialog(
            state = loginState,
            onConfirm = {
                scope.launch(Dispatchers.IO) {
                    when (loginState) {
                        is LoginState.AwaitingDeviceCode -> viewModel.completeMicrosoftLogin()
                        is LoginState.Error -> viewModel.beginMicrosoftLogin()
                        else -> { /* signing in: ignore until it settles */ }
                    }
                }
            },
            onDismiss = viewModel::cancelLogin,
        )
    }

    if (showAddOffline) {
        AddOfflineDialog(
            onConfirm = { name ->
                scope.launch(Dispatchers.IO) { viewModel.addOffline(name) }
                showAddOffline = false
            },
            onDismiss = { showAddOffline = false },
        )
    }
}

// ============================================================================
// Account rows / cards
// ============================================================================

@Composable
private fun ActiveAccountCard(account: Account) {
    Surface(tonalElevation = 2.dp, shape = RoundedCornerShape(14.dp), modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SkinAvatar(account.uuid, Modifier.size(56.dp).clip(CircleShape))
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(
                    "当前登录",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(account.username.ifBlank { "(无名)" }, style = MaterialTheme.typography.titleLarge)
                Text(account.kind.label, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun AccountRow(
    account: Account,
    isActive: Boolean,
    onSelect: () -> Unit,
    onRemove: () -> Unit,
    onRefresh: () -> Unit,
) {
    Surface(
        tonalElevation = if (isActive) 3.dp else 1.dp,
        shape = RoundedCornerShape(14.dp),
        color = if (isActive) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                SkinAvatar(account.uuid, Modifier.size(48.dp).clip(CircleShape))
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(account.username.ifBlank { "(无名)" }, style = MaterialTheme.typography.titleMedium)
                        if (isActive) {
                            Surface(color = MaterialTheme.colorScheme.primary, shape = RoundedCornerShape(6.dp)) {
                                Text(
                                    "当前",
                                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 1.dp),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onPrimary,
                                )
                            }
                        }
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        KindBadge(account.kind)
                        if (account is MicrosoftAccount) TokenBadge(account.tokenStatus)
                    }
                }
                IconButton(onClick = onRemove) {
                    Icon(Icons.Filled.Delete, contentDescription = "删除账号", tint = MaterialTheme.colorScheme.error)
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (!isActive) {
                    OutlinedButton(onClick = onSelect, modifier = Modifier.weight(1f)) {
                        Icon(Icons.Filled.Check, contentDescription = null)
                        Text("设为当前")
                    }
                }
                if (account is MicrosoftAccount) {
                    OutlinedButton(
                        onClick = onRefresh,
                        modifier = Modifier.weight(1f),
                        enabled = account.tokenStatus != TokenStatus.VALID,
                    ) {
                        Icon(Icons.Filled.Refresh, contentDescription = null)
                        Text("刷新令牌")
                    }
                }
            }
        }
    }
}

@Composable
private fun KindBadge(kind: AccountKind) {
    Surface(color = MaterialTheme.colorScheme.secondaryContainer, shape = RoundedCornerShape(6.dp)) {
        Text(
            kind.label,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
    }
}

@Composable
private fun TokenBadge(status: TokenStatus) {
    val color = when (status) {
        TokenStatus.VALID -> Color(0xFF2E7D32)
        TokenStatus.EXPIRING -> Color(0xFFF9A825)
        TokenStatus.EXPIRED -> Color(0xFFC62828)
        TokenStatus.UNKNOWN -> MaterialTheme.colorScheme.outline
    }
    Surface(color = color.copy(alpha = 0.16f), shape = RoundedCornerShape(6.dp)) {
        Text(
            status.label,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            color = color,
        )
    }
}

@Composable
private fun EmptyAccounts() {
    Surface(tonalElevation = 1.dp, shape = RoundedCornerShape(14.dp), modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(
                Icons.Filled.AccountCircle,
                contentDescription = null,
                modifier = Modifier.size(40.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text("还没有账号", style = MaterialTheme.typography.titleMedium)
            Text(
                "添加一个离线账号，或使用微软登录接入正版 Minecraft。",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

// ============================================================================
// Skin preview (network avatar with a graceful letter fallback)
// ============================================================================

@Composable
private fun SkinAvatar(uuid: String, modifier: Modifier = Modifier) {
    var bitmap by remember(uuid) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(uuid) {
        launch(Dispatchers.IO) {
            runCatching {
                val url = "https://mc-heads.net/avatar/$uuid/64?overlay"
                val conn = java.net.URL(url).openConnection() as java.net.HttpURLConnection
                conn.connectTimeout = 8000
                conn.readTimeout = 8000
                conn.inputStream.use { stream ->
                    android.graphics.BitmapFactory.decodeStream(stream)?.asImageBitmap()
                }
            }.onSuccess { b -> if (b != null) bitmap = b }
        }
    }
    val bmp = bitmap
    if (bmp != null) {
        Image(bitmap = bmp, contentDescription = null, modifier = modifier)
    } else {
        Surface(modifier = modifier, shape = CircleShape, color = MaterialTheme.colorScheme.primaryContainer) {
            Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text(
                    text = if (uuid.isEmpty()) "?" else uuid.first().uppercase(),
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    style = MaterialTheme.typography.titleMedium,
                )
            }
        }
    }
}

// ============================================================================
// Dialogs
// ============================================================================

@Composable
private fun MicrosoftLoginDialog(
    state: LoginState,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val challenge = (state as? LoginState.AwaitingDeviceCode)?.challenge
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            Button(onClick = onConfirm, enabled = state !is LoginState.SigningIn) {
                Text(if (state is LoginState.Error) "重试" else "我已登录")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("取消") }
        },
        title = { Text("微软账户登录") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                when {
                    state is LoginState.SigningIn -> {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 3.dp)
                            Text("正在与 Microsoft 通信…")
                        }
                    }
                    state is LoginState.Error -> {
                        Text(state.message, color = MaterialTheme.colorScheme.error)
                    }
                    challenge != null -> {
                        Text(challenge.message.ifBlank { "请在浏览器中完成登录后点击“我已登录”。" })
                        Surface(
                            color = MaterialTheme.colorScheme.secondaryContainer,
                            shape = RoundedCornerShape(12.dp),
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Column(
                                modifier = Modifier.fillMaxWidth().padding(16.dp),
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Text(
                                    "验证码",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                                Text(
                                    challenge.userCode,
                                    style = MaterialTheme.typography.headlineSmall,
                                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                                Text(
                                    challenge.verificationUrl,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                            }
                        }
                        Text(
                            "在浏览器中打开上述网址并输入验证码，然后返回此处点击“我已登录”。",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        },
    )
}

@Composable
private fun AddOfflineDialog(
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var name by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            Button(
                onClick = { if (name.isNotBlank()) onConfirm(name.trim()) },
                enabled = name.isNotBlank(),
            ) { Text("添加") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("取消") } },
        title = { Text("添加离线账号") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("离线账号无需联网，可用于启动未加密的离线整合包。")
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    label = { Text("用户名") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
    )
}

@Preview(showBackground = true)
@Composable
private fun AccountsScreenPreview() {
    val sample = listOf(
        MicrosoftAccount(
            uuid = "069a79f4-44e9-4726-a5be-fca90e38aaf5",
            username = "Notch",
            expiresAt = nowSecs() + 86400,
            msExpiresAt = nowSecs() + 3600,
        ),
        OfflineAccount(uuid = offlineUuid("Steve"), username = "Steve"),
    )
    AccountsScreen(viewModel = AccountViewModel(InMemoryAccountRepository(sample)))
}
