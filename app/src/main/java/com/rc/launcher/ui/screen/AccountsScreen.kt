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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Upload
import androidx.compose.material.icons.filled.Visibility
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
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.model.Account
import com.rc.launcher.ui.model.AccountKind
import com.rc.launcher.ui.model.InMemoryAccountRepository
import com.rc.launcher.ui.model.MicrosoftAccount
import com.rc.launcher.ui.model.SkinModel
import com.rc.launcher.ui.model.SkinSource
import com.rc.launcher.ui.model.OfflineAccount
import com.rc.launcher.ui.model.TokenStatus
import com.rc.launcher.ui.model.ThirdPartyLogin
import com.rc.launcher.ui.model.ThirdPartyServerInfo
import com.rc.launcher.ui.model.nowSecs
import com.rc.launcher.ui.model.SkinTutorialStep
import com.rc.launcher.ui.model.offlineUuid
import com.rc.launcher.ui.viewmodel.AccountViewModel
import com.rc.launcher.ui.viewmodel.TutorialViewModel
import com.rc.launcher.ui.viewmodel.LoginState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.OpenInBrowser
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import android.content.Intent
import android.net.Uri
import android.graphics.BitmapFactory
import android.util.Base64
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import kotlinx.coroutines.delay
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.RcStrings
import com.rc.launcher.ui.i18n.LocalRcStrings
import com.rc.launcher.ui.i18n.rcString
import com.rc.launcher.ui.model.formatDuration
import com.rc.launcher.ui.model.remainingSecs

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
    val skinModel by viewModel.skinModel.collectAsStateWithLifecycle()
    val tutorialViewModel: TutorialViewModel = viewModel()
    val skinError by viewModel.skinError.collectAsStateWithLifecycle()
    val isFetchingSkin by viewModel.isFetchingSkin.collectAsStateWithLifecycle()
    val isUploading by viewModel.isUploading.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()

    val scope = rememberCoroutineScope()
    var showAddOffline by remember { mutableStateOf(false) }
    var showThirdParty by remember { mutableStateOf(false) }
    var previewAccount by remember { mutableStateOf<Account?>(null) }
    var pendingRemove by remember { mutableStateOf<Account?>(null) }

    // Load the skin model when the preview account changes (task 22).
    LaunchedEffect(previewAccount?.uuid) {
        val acc = previewAccount
        if (acc != null && acc is MicrosoftAccount) {
            viewModel.loadSkin(acc.uuid)
        } else {
            viewModel.clearSkin()
        }
    }

    // Load the account list as soon as the screen appears (the ViewModel keeps
    // the list in a StateFlow; this just seeds it from the repository).
    LaunchedEffect(Unit) { viewModel.loadAccounts() }

    LazyColumn(
        modifier = Modifier.fillMaxSize().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(bottom = 24.dp),
    ) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("账户管理", style = MaterialTheme.typography.headlineSmall)
                    Text(
                        "管理正版 (Microsoft) 与离线账户，查看令牌状态并切换当前登录身份。",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(onClick = { scope.launch(Dispatchers.IO) { viewModel.refreshAllMicrosoft() } }) {
                    Icon(Icons.Filled.Refresh, contentDescription = "刷新所有令牌")
                }
            }
        }

        item { activeAccount?.let { ActiveAccountCard(account = it, onAvatarClick = { previewAccount = it }) } }

        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = {
                        // Task 28: the redirect_uri is already registered with the
                        // Rust core via authInit (pointing to the dynamically written
                        // callback page in the cache directory). Query authDefaultRedirectUri()
                        // so the UI and the core always agree on the address.
                        val redirectUri = runCatching {
                            com.rc.launcher.core.RustBridge.authDefaultRedirectUri()
                        }.getOrDefault("file:///android_asset/microsoft_auth.html")
                        scope.launch(Dispatchers.IO) { viewModel.beginMicrosoftLogin(redirectUri) }
                    },
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
                OutlinedButton(
                    onClick = { showThirdParty = true },
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(Icons.Filled.PersonAdd, contentDescription = null)
                    Text("第三方账号")
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
                    onRemove = { pendingRemove = account },
                    onRefresh = { scope.launch(Dispatchers.IO) { viewModel.refreshAccount(account.uuid) } },
                    onAvatarClick = { previewAccount = account },
                )
            }
        }
    }

    if (showThirdParty) {
        ThirdPartyLoginDialog(
            state = loginState,
            onConnect = { url -> scope.launch(Dispatchers.IO) { viewModel.beginThirdPartyDiscovery(url) } },
            onLogin = { login -> scope.launch(Dispatchers.IO) { viewModel.completeThirdPartyLogin(login) } },
            onDismiss = { showThirdParty = false; viewModel.cancelLogin() },
        )
    } else if (loginState !is LoginState.Idle) {
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

    previewAccount?.let { acct ->
        SkinPreviewDialog(
            account = acct,
            skinModel = skinModel,
            isFetching = isFetchingSkin,
            isUploading = isUploading,
            skinError = skinError,
            viewModel = viewModel,
            tutorialViewModel = tutorialViewModel,
            onDismiss = { previewAccount = null },
        )
    }

    pendingRemove?.let { acc ->
        ConfirmRemoveDialog(
            account = acc,
            onConfirm = {
                scope.launch(Dispatchers.IO) { viewModel.removeAccount(acc.uuid) }
                pendingRemove = null
            },
            onDismiss = { pendingRemove = null },
        )
    }
}

// ============================================================================
// Account rows / cards
// ============================================================================

@Composable
private fun ActiveAccountCard(
    account: Account,
    onAvatarClick: () -> Unit,
) {
    Surface(tonalElevation = 2.dp, shape = RoundedCornerShape(14.dp), modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SkinAvatar(account.uuid, Modifier.size(56.dp).clip(CircleShape), onClick = onAvatarClick)
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(
                    "当前登录",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(account.username.ifBlank { "(无名)" }, style = MaterialTheme.typography.titleLarge)
                Text(account.kind.label, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                if (account is MicrosoftAccount) {
                    TokenBadge(account.tokenStatus)
                    Text(
                        "令牌剩余：${formatDuration(account.remainingSecs)}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
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
    onAvatarClick: () -> Unit,
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
                SkinAvatar(account.uuid, Modifier.size(48.dp).clip(CircleShape), onClick = onAvatarClick)
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
                        if (account is MicrosoftAccount) {
                            TokenBadge(account.tokenStatus)
                            Text(
                                "剩余 ${formatDuration(account.remainingSecs)}",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
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
private fun SkinAvatar(
    skinUrl: String,
    capeUrl: String,
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
) {
    var skinBmp by remember(skinUrl) { mutableStateOf<ImageBitmap?>(null) }
    var capeBmp by remember(capeUrl) { mutableStateOf<ImageBitmap?>(null) }
    var loadError by remember { mutableStateOf(false) }

    LaunchedEffect(skinUrl) {
        launch(Dispatchers.IO) {
            runCatching {
                val conn = java.net.URL(skinUrl).openConnection() as java.net.HttpURLConnection
                conn.connectTimeout = 8000
                conn.readTimeout = 8000
                conn.inputStream.use { stream ->
                    android.graphics.BitmapFactory.decodeStream(stream)?.asImageBitmap()
                }
            }.onSuccess { b ->
                if (b != null) {
                    skinBmp = b
                } else {
                    loadError = true
                }
            }.onFailure {
                loadError = true
            }
        }
    }
    if (capeUrl.isNotBlank()) {
        LaunchedEffect(capeUrl) {
            launch(Dispatchers.IO) {
                runCatching {
                    val conn = java.net.URL(capeUrl).openConnection() as java.net.HttpURLConnection
                    conn.connectTimeout = 8000
                    conn.readTimeout = 8000
                    conn.inputStream.use { stream ->
                        android.graphics.BitmapFactory.decodeStream(stream)?.asImageBitmap()
                    }
                }.onSuccess { b ->
                    if (b != null) capeBmp = b
                }
            }
        }
    }

    val bmp = skinBmp
    val decorated = if (onClick != null) modifier.clickable { onClick() } else modifier
    if (bmp != null) {
        Box(modifier = decorated) {
            Image(bitmap = bmp, contentDescription = null, modifier = Modifier.matchParentSize())
            if (capeBmp != null) {
                // Overlay the cape as a small badge in the top-right corner.
                Image(
                    bitmap = capeBmp!!,
                    contentDescription = null,
                    modifier = Modifier
                        .size(48.dp)
                        .clip(RoundedCornerShape(6.dp))
                        .align(Alignment.TopEnd),
                )
            }
        }
    } else if (loadError) {
        Surface(modifier = decorated, shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.secondaryContainer) {
            Box(modifier = Modifier.matchParentSize(), contentAlignment = Alignment.Center) {
                Text(
                    text = "!",
                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                    style = MaterialTheme.typography.titleMedium,
                )
            }
        }
    } else {
        Surface(modifier = decorated, shape = RoundedCornerShape(12.dp), color = MaterialTheme.colorScheme.primaryContainer) {
            Box(modifier = Modifier.matchParentSize(), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
            }
        }
    }
}

@Composable
private fun SkinAvatar(
    uuid: String,
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
) {
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
    val decorated = if (onClick != null) modifier.clickable { onClick() } else modifier
    if (bmp != null) {
        Image(bitmap = bmp, contentDescription = null, modifier = decorated)
    } else {
        Surface(modifier = decorated, shape = CircleShape, color = MaterialTheme.colorScheme.primaryContainer) {
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
    val clipboard = LocalClipboardManager.current
    val context = LocalContext.current

    // Live countdown to the device-code expiry so the user can see how much time
    // is left to authenticate in the browser (task 16).
    var remaining by remember(challenge?.userCode) { mutableStateOf(challenge?.expiresIn ?: 0L) }
    LaunchedEffect(challenge?.userCode) {
        while (remaining > 0) {
            delay(1000)
            remaining -= 1
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            Button(onClick = onConfirm, enabled = state !is LoginState.SigningIn) {
                Text(if (state is LoginState.Error) "重试" else "我已登录")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(rcString(RcStringKeys.COMMON_CANCEL)) }
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
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.SpaceBetween,
                                ) {
                                    Text(
                                        "验证码",
                                        style = MaterialTheme.typography.labelMedium,
                                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                                    )
                                    IconButton(onClick = { clipboard.setText(AnnotatedString(challenge.userCode)) }) {
                                        Icon(
                                            Icons.Filled.ContentCopy,
                                            contentDescription = "复制验证码",
                                            tint = MaterialTheme.colorScheme.onSecondaryContainer,
                                        )
                                    }
                                }
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
                                Text(
                                    "有效期剩余：${formatDuration(remaining)}",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSecondaryContainer,
                                )
                            }
                        }
                        OutlinedButton(
                            onClick = {
                                // Task 28: when a custom callback address is configured,
                                // append it as a redirect_uri param so Microsoft
                                // redirects to the embedded callback page after sign-in.
                                val url = if (!challenge.redirectUri.isNullOrBlank()) {
                                    val sep = if (challenge.verificationUrl.contains("?")) "&" else "?"
                                    "${challenge.verificationUrl}${sep}redirect_uri=${Uri.encode(challenge.redirectUri)}"
                                } else {
                                    challenge.verificationUrl
                                }
                                runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
                            },
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            Icon(Icons.Filled.OpenInBrowser, contentDescription = null)
                            Text("在浏览器中打开")
                        }
                        // Task 28: show the callback address hint when configured.
                        challenge.redirectUri?.let { uri ->
                            if (uri.isNotBlank()) {
                                Text(
                                    "浏览器将在完成登录后跳转到嵌入式回调页。如跳转失败，请在设置中更换代理或镜像后重试。",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                        if (challenge.redirectUri.isNullOrBlank()) {
                            Text(
                                "在浏览器中打开上述网址并输入验证码，然后返回此处点击\"我已登录\"。",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun SkinPreviewDialog(
    account: Account,
    skinModel: SkinModel?,
    isFetching: Boolean,
    isUploading: Boolean,
    skinError: String?,
    viewModel: AccountViewModel,
    tutorialViewModel: TutorialViewModel,
    onDismiss: () -> Unit,
) {
    val context = LocalContext.current
    var localBitmap by remember { mutableStateOf<ImageBitmap?>(null) }
    var localUri by remember { mutableStateOf<Uri?>(null) }
    var showUploadDialog by remember { mutableStateOf(false) }
    var uploadModel by remember { mutableStateOf("default") }
    var validationError by remember { mutableStateOf<String?>(null) }
    // Task 23: skin import tutorial toggle.
    var showSkinTutorial by remember { mutableStateOf(false) }

    // File picker for local image selection (task 22).
    val pickLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.GetContent(),
    ) { uri: Uri? ->
        if (uri == null) return@rememberLauncherForActivityResult
        runCatching {
            context.contentResolver.takePersistableUriPermission(
                uri, Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
        }
        localUri = uri
        validationError = null
        localBitmap = runCatching {
            context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) }?.asImageBitmap()
        }.getOrNull()
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(rcString(RcStringKeys.COMMON_CLOSE)) }
        },
        title = { Text(account.username.ifBlank { "(无名)" }) },
        text = {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 520.dp, max = 720.dp)
                    .verticalScroll(rememberScrollState()),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                // Fetch / upload progress banner
                if (isFetching) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                        Text(rcString(RcStringKeys.SKIN_FETCHING), style = MaterialTheme.typography.bodySmall)
                    }
                }
                if (isUploading) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                        Text(rcString(RcStringKeys.SKIN_UPLOADING), style = MaterialTheme.typography.bodySmall)
                    }
                }
                skinError?.let { msg ->
                    Text(msg, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }
                validationError?.let { msg ->
                    Text(msg, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
                }

                // Skin + cape 2D preview (lightweight rendering).
                if (localBitmap != null) {
                    // Show the locally selected image.
                    Box(
                        modifier = Modifier
                            .size(160.dp)
                            .clip(RoundedCornerShape(12.dp)),
                    ) {
                        Image(
                            bitmap = localBitmap!!,
                            contentDescription = rcString(RcStringKeys.SKIN_PREVIEW_TITLE),
                            contentScale = ContentScale.Crop,
                            modifier = Modifier.matchParentSize(),
                        )
                        if (localUri != null) {
                            Box(
                                modifier = Modifier
                                    .align(Alignment.TopEnd)
                                    .padding(4.dp)
                                    .background(MaterialTheme.colorScheme.primaryContainer, CircleShape)
                                    .padding(4.dp),
                            ) {
                                Icon(Icons.Filled.Image, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimaryContainer, modifier = Modifier.size(16.dp))
                            }
                        }
                    }
                } else if (skinModel != null && skinModel.isCached()) {
                    // Show the fetched skin from the model URL (cached/offline).
                    SkinAvatar(
                        skinUrl = skinModel.skinUrl,
                        capeUrl = skinModel.capeUrl ?: "",
                        modifier = Modifier.size(160.dp).clip(RoundedCornerShape(12.dp)),
                    )
                } else if (skinModel != null && !skinModel.isCached() && skinModel.skinUrl.isNotBlank()) {
                    // Show the skin even if not yet cached (first load).
                    SkinAvatar(
                        skinUrl = skinModel.skinUrl,
                        capeUrl = skinModel.capeUrl ?: "",
                        modifier = Modifier.size(160.dp).clip(RoundedCornerShape(12.dp)),
                    )
                } else {
                    // No skin model — show the default mc-heads avatar.
                    SkinAvatar(account.uuid, Modifier.size(160.dp).clip(RoundedCornerShape(12.dp)))
                }

                // Skin metadata row
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    KindBadge(account.kind)
                    Text(
                        text = rcString(if (skinModel?.model == "slim") RcStringKeys.SKIN_MODEL_ALEX else RcStringKeys.SKIN_MODEL_STEVE),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    skinModel?.let { sm ->
                        Text(
                            text = rcString(if (sm.source == SkinSource.OFFICIAL) RcStringKeys.SKIN_SOURCE_OFFICIAL else RcStringKeys.SKIN_SOURCE_CUSTOM),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        if (sm.capeUrl != null && sm.capeUrl.isNotBlank()) {
                            Text(
                                text = rcString(RcStringKeys.SKIN_HAS_CAPE),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (sm.isCached()) {
                            Text(
                                text = rcString(RcStringKeys.SKIN_CACHED_OFFLINE),
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.primary,
                            )
                        }
                    }
                    if (account is MicrosoftAccount) {
                        TokenBadge(account.tokenStatus)
                        Text(
                            "令牌剩余：${formatDuration(account.remainingSecs)}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }

                // Action buttons (only for Microsoft accounts)
                if (account is MicrosoftAccount) {
                    if (localBitmap != null) {
                        val strings = LocalRcStrings.current
                        val (valid, reason) = validateSkinBitmap(localBitmap!!, strings)
                        OutlinedButton(
                            onClick = {
                                if (valid) {
                                    showUploadDialog = true
                                    uploadModel = skinModel?.model?.takeIf { it == "slim" } ?: "default"
                                } else {
                                    validationError = reason
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                            enabled = !isUploading,
                        ) {
                            Icon(Icons.Filled.Upload, contentDescription = null)
                            Text(if (isUploading) rcString(RcStringKeys.SKIN_UPLOADING_LABEL) else rcString(RcStringKeys.SKIN_UPLOAD))
                        }
                    } else {
                        OutlinedButton(
                            onClick = { pickLauncher.launch("image/png") },
                            modifier = Modifier.fillMaxWidth(),
                            enabled = !isFetching && !isUploading,
                        ) {
                            Icon(Icons.Filled.Image, contentDescription = null)
                            Text(rcString(RcStringKeys.SKIN_PICK_LOCAL))
                        }
                        OutlinedButton(
                            onClick = { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(skinModel?.skinUrl ?: "https://mc-heads.net/avatar/${account.uuid}/64"))) },
                            modifier = Modifier.fillMaxWidth(),
                            enabled = skinModel?.skinUrl?.isNotBlank() == true,
                        ) {
                            Icon(Icons.Filled.OpenInBrowser, contentDescription = null)
                            Text(rcString(RcStringKeys.SKIN_VIEW_IN_BROWSER))
                        }
                    }
                } else {
                    Text(
                        rcString(RcStringKeys.SKIN_OFFLINE_ACCOUNT),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                // Task 23: skin import tutorial section.
                SkinTutorialSection(
                    tutorialViewModel = tutorialViewModel,
                    showTutorial = showSkinTutorial,
                    onToggleTutorial = { showSkinTutorial = !showSkinTutorial },
                    onPickImage = { pickLauncher.launch("image/png") },
                    isFetching = isFetching,
                    isUploading = isUploading,
                    account = account,
                )
            }
        },
    )

    // Upload confirmation dialog
    if (showUploadDialog) {
        AlertDialog(
            onDismissRequest = { showUploadDialog = false },
            confirmButton = {
                Button(
                    onClick = {
                        showUploadDialog = false
                        localUri?.let { uri ->
                            val pngBytes = runCatching {
                                context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                            }.getOrNull()
                            if (pngBytes != null) {
                                val b64 = Base64.encodeToString(pngBytes, Base64.NO_WRAP)
                                val modelParam = if (uploadModel == "slim") "slim" else "classic"
                                viewModel.uploadSkin(account.uuid, modelParam, b64)
                            } else {
                                validationError = rcString(RcStringKeys.SKIN_READ_ERROR)
                            }
                        }
                    },
                    enabled = !isUploading,
                ) { Text(rcString(RcStringKeys.SKIN_UPLOAD_CONFIRM)) }
            },
            dismissButton = { TextButton(onClick = { showUploadDialog = false }) { Text(rcString(RcStringKeys.COMMON_CANCEL)) } },
            title = { Text(rcString(RcStringKeys.SKIN_PREVIEW_TITLE)) },
            text = {
                Column {
                    Text(rcString(RcStringKeys.SKIN_SELECT_MODEL))
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        RadioButton(
                            selected = uploadModel == "classic",
                            onClick = { uploadModel = "classic" },
                            colors = RadioButtonDefaults.colors(),
                        )
                        Text(rcString(RcStringKeys.SKIN_MODEL_STEVE))
                        RadioButton(
                            selected = uploadModel == "slim",
                            onClick = { uploadModel = "slim" },
                        )
                        Text(rcString(RcStringKeys.SKIN_MODEL_ALEX))
                    }
                    Text(
                        rcString(RcStringKeys.SKIN_UPLOAD_NOTE),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            },
        )
    }
}

/**
 * Validate a skin PNG's dimensions against the known-good Minecraft sizes (task 23).
 * Returns (true, null) when valid, (false, reason) otherwise. The reason string
 * is localised via [rc] so the validation error respects the current language (task 20).
 */
private fun validateSkinBitmap(bitmap: ImageBitmap, rc: RcStrings): Pair<Boolean, String?> {
    val width = bitmap.width
    val height = bitmap.height
    val validSizes = setOf(
        64 to 64, 64 to 32, 128 to 128, 128 to 64,
        128 to 192, 256 to 256, 256 to 128,
    )
    if (width to height !in validSizes) {
        return Pair(
            false,
            rc.format(RcStringKeys.SKIN_INVALID_DIMENSIONS, "w" to width.toString(), "h" to height.toString()),
        )
    }
    return Pair(true, null)
}

@Composable
private fun ConfirmRemoveDialog(
    account: Account,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = { Button(onClick = onConfirm) { Text("删除") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(rcString(RcStringKeys.COMMON_CANCEL)) } },
        title = { Text("删除账户") },
        text = {
            Text(
                "确定要删除账户「${account.username.ifBlank { "(无名)" }}」吗？此操作不可撤销。",
                style = MaterialTheme.typography.bodyMedium,
            )
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
        dismissButton = { TextButton(onClick = onDismiss) { Text(rcString(RcStringKeys.COMMON_CANCEL)) } },
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


@Composable
private fun ThirdPartyLoginDialog(
    state: LoginState,
    onConnect: (String) -> Unit,
    onLogin: (ThirdPartyLogin) -> Unit,
    onDismiss: () -> Unit,
) {
    var serverUrl by remember { mutableStateOf("") }
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    val provider = "authlib_injector"
    val info = (state as? LoginState.ThirdPartyServer)?.info
    val context = LocalContext.current

    AlertDialog(
        onDismissRequest = onDismiss,
        confirmButton = {
            when {
                state is LoginState.ThirdPartySigningIn -> {
                    Button(onClick = {}, enabled = false) { Text("请稍候") }
                }
                info != null -> {
                    Button(
                        onClick = {
                            onLogin(
                                ThirdPartyLogin(
                                    provider = provider,
                                    serverUrl = info.serverUrl.ifBlank { serverUrl },
                                    username = username,
                                    password = password,
                                ),
                            )
                        },
                        enabled = username.isNotBlank() && password.isNotBlank(),
                    ) { Text("登录") }
                }
                else -> {
                    Button(
                        onClick = { if (serverUrl.isNotBlank()) onConnect(serverUrl.trim()) },
                        enabled = serverUrl.isNotBlank() && state !is LoginState.ThirdPartySigningIn,
                    ) { Text("连接服务器") }
                }
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(rcString(RcStringKeys.COMMON_CANCEL)) } },
        title = { Text("第三方账号登录") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                when {
                    state is LoginState.ThirdPartySigningIn -> {
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(12.dp),
                        ) {
                            CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 3.dp)
                            Text("正在与验证服务器通信…")
                        }
                    }
                    state is LoginState.Error -> {
                        Text(state.message, color = MaterialTheme.colorScheme.error)
                    }
                }
                if (info == null) {
                    Text("输入第三方验证服务器地址（Authlib-Injector / 外置验证服务器）。大陆网络下若无法直连微软，可改用此类服务登录。")
                    OutlinedTextField(
                        value = serverUrl,
                        onValueChange = { serverUrl = it },
                        label = { Text("服务器地址") },
                        placeholder = { Text("https://auth.example.com") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                } else {
                    Surface(
                        color = MaterialTheme.colorScheme.secondaryContainer,
                        shape = RoundedCornerShape(12.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Column(
                            modifier = Modifier.fillMaxWidth().padding(12.dp),
                            verticalArrangement = Arrangement.spacedBy(6.dp),
                        ) {
                            Text(
                                "服务器：${info.serverName.ifBlank { info.serverUrl }}",
                                style = MaterialTheme.typography.titleSmall,
                            )
                            info.links.firstOrNull { it.label == "register" }?.let { link ->
                                TextButton(onClick = { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link.url))) }) {
                                    Text("注册账号")
                                }
                            }
                        }
                    }
                    OutlinedTextField(
                        value = username,
                        onValueChange = { username = it },
                        label = { Text("用户名") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    OutlinedTextField(
                        value = password,
                        onValueChange = { password = it },
                        label = { Text("密码") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        },
    )
}



/**
 * Embedded skin-import tutorial (task 23).
 *
 * A collapsible, step-by-step walkthrough that is embedded in the [SkinPreviewDialog]
 * on the Accounts screen. When the user taps "Start tutorial" the card expands to
 * show one step at a time, with contextual hints (official / third-party notes,
 * file-format validation, one-click file picker) that mirror the live controls
 * below it.
 *
 * The step state lives in [TutorialState] / [TutorialViewModel] so it survives
 * process death and is linked to the task-14 onboarding system. The tutorial
 * auto-shows the file picker on the "Import" step via [onPickImage].
 *
 * **i18n (task 20).** Every visible string goes through [rcString] with a
 * [RcStringKeys.SKIN_TUTORIAL_*] key, so the tutorial is fully localised and
 * participates in the same i18n parity checks as the rest of the launcher.
 */
@Composable
private fun SkinTutorialSection(
    tutorialViewModel: TutorialViewModel,
    showTutorial: Boolean,
    onToggleTutorial: () -> Unit,
    onPickImage: () -> Unit,
    isFetching: Boolean,
    isUploading: Boolean,
    account: Account,
) {
    val tutorialState by tutorialViewModel.state.collectAsStateWithLifecycle()
    val skinStep = tutorialState.currentSkinTutorialStep
    val totalSteps = tutorialState.skinTutorialTotalSteps
    val isLast = SkinTutorialStep.entries.indexOf(skinStep) == totalSteps - 1
    val skinTutorialDone = tutorialState.skinTutorialCompleted

    // Step metadata: (title key, body key, hint key).
    val steps = listOf(
        Triple(
            RcStringKeys.SKIN_TUTORIAL_STEP_GET_TITLE,
            RcStringKeys.SKIN_TUTORIAL_STEP_GET_BODY,
            RcStringKeys.SKIN_TUTORIAL_OFFICIAL_NOTE,
        ),
        Triple(
            RcStringKeys.SKIN_TUTORIAL_STEP_FORMAT_TITLE,
            RcStringKeys.SKIN_TUTORIAL_STEP_FORMAT_BODY,
            RcStringKeys.SKIN_TUTORIAL_FORMAT_NOTE,
        ),
        Triple(
            RcStringKeys.SKIN_TUTORIAL_STEP_IMPORT_TITLE,
            RcStringKeys.SKIN_TUTORIAL_STEP_IMPORT_BODY,
            RcStringKeys.SKIN_TUTORIAL_PICK,
        ),
        Triple(
            RcStringKeys.SKIN_TUTORIAL_STEP_APPLY_TITLE,
            RcStringKeys.SKIN_TUTORIAL_STEP_APPLY_BODY,
            RcStringKeys.SKIN_TUTORIAL_LINK_TASK14,
        ),
        Triple(
            RcStringKeys.SKIN_TUTORIAL_STEP_DONE_TITLE,
            RcStringKeys.SKIN_TUTORIAL_STEP_DONE_BODY,
            null,
        ),
    )
    val (titleKey, bodyKey, hintKey) = steps[skinStep.ordinal]

    // Step indicator: "Step 2 of 5".
    val indicator = rcString(
        RcStringKeys.SKIN_TUTORIAL_STEP_INDICATOR,
        "current" to (skinStep.ordinal + 1).toString(),
        "total" to totalSteps.toString(),
    )

    // One-click file picker: when we reach the Import step and the user is on a
    // Microsoft account, auto-open the picker (only once per visit).
    var autoPicked by remember(skinStep, account.uuid) { mutableStateOf(false) }
    if (skinStep == SkinTutorialStep.IMPORT && !autoPicked && account is MicrosoftAccount) {
        LaunchedEffect(Unit) {
            onPickImage()
            autoPicked = true
        }
    }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        // Toggle button: start / collapse. When re-watching a completed
        // tutorial, reset to the first step so the user sees the full flow.
        OutlinedButton(
            onClick = {
                if (!showTutorial && skinTutorialDone) {
                    tutorialViewModel.skinStart()
                }
                onToggleTutorial()
            },
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (showTutorial) {
                Text("▲ " + rcString(RcStringKeys.SKIN_TUTORIAL_TITLE))
            } else if (skinTutorialDone) {
                Text(rcString(RcStringKeys.SKIN_TUTORIAL_REWATCH))
            } else {
                Text(rcString(RcStringKeys.SKIN_TUTORIAL_START))
            }
        }

        if (showTutorial) {
            Surface(
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surfaceVariant,
                tonalElevation = 1.dp,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(
                    modifier = Modifier.padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    // Step indicator.
                    Text(
                        text = indicator,
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )

                    // Step title.
                    Text(
                        text = rcString(titleKey),
                        style = MaterialTheme.typography.headlineSmall,
                        color = MaterialTheme.colorScheme.onSurface,
                    )

                    // Step body.
                    Text(
                        text = rcString(bodyKey),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )

                    // Contextual hint (official note, format note, etc.).
                    hintKey?.let { key ->
                        val hintText = rcString(key)
                        if (hintText.isNotBlank()) {
                            Surface(
                                color = MaterialTheme.colorScheme.primaryContainer,
                                shape = RoundedCornerShape(8.dp),
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text(
                                    text = hintText,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                                    modifier = Modifier.padding(8.dp),
                                )
                            }
                        }
                    }

                    // Navigation buttons.
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        OutlinedButton(
                            onClick = tutorialViewModel::skinPrevious,
                            modifier = Modifier.weight(1f),
                        ) {
                            Text(rcString(RcStringKeys.SKIN_TUTORIAL_PREVIOUS))
                        }
                        OutlinedButton(
                            onClick = tutorialViewModel::skinSkip,
                            modifier = Modifier.weight(1f),
                        ) {
                            Text(rcString(RcStringKeys.SKIN_TUTORIAL_SKIP))
                        }
                        Button(
                            onClick = {
                                if (isLast) {
                                    tutorialViewModel.skinFinish()
                                    onToggleTutorial()
                                } else {
                                    tutorialViewModel.skinNext()
                                }
                            },
                            modifier = Modifier.weight(1f),
                            enabled = !isFetching && !isUploading,
                        ) {
                            Text(
                                if (isLast) rcString(RcStringKeys.SKIN_TUTORIAL_FINISH)
                                else rcString(RcStringKeys.SKIN_TUTORIAL_NEXT),
                            )
                        }
                    }
                }
            }
        }
    }
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
