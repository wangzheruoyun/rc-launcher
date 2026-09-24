package com.rc.launcher.ui.screen

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.DriveFileMove
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.ProvideRcWindowInfo
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.InstanceRepository
import com.rc.launcher.ui.rcWindowInfo
import com.rc.launcher.ui.viewmodel.FileManagerViewModel
import com.rc.launcher.ui.viewmodel.FileManagerDialogState
import com.rc.launcher.ui.viewmodel.FsEntry
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import java.io.File

/**
 * In-app small file manager (task 19).
 *
 * The screen is deliberately *small*: it can only touch the directories the
 * caller has put on its allowed-roots list. The Rust core rejects any
 * attempt to read or write outside them (path-traversal guard), so a
 * malicious UI cannot trick the bridge into escaping the sandbox.
 *
 * Features:
 *   * Browse the per-instance game directory, the game root or any other
 *     allowed root (e.g. `saves/`, `mods/`, `resourcepacks/`,
 *     `shaderpacks/`).
 *   * Multi-select rows → request delete (two-step confirmation) → confirm.
 *   * Create a new directory.
 *   * Rename an entry.
 *   * Copy a file (duplicate a save world, a mod, etc.).
 *   * Extract a `.zip` archive into the current directory (used by the mod
 *     / resource-pack / shader-pack import flow).
 *   * Import a file from a `content://` URI (system file picker).
 *
 * Adaptive (task 9): the list caps its width on a wide tablet; the path
 * bar uses a mono font so long absolute paths are still readable.
 */
@Composable
fun FileManagerScreen(
    navController: NavHostController? = null,
    instanceId: String? = null,
    initialSubdir: String? = null,
    baseDir: String = "games/RC",
    vm: FileManagerViewModel = viewModel(),
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val snackbar = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val window = rcWindowInfo()
    val pad = window.contentPaddingDp.dp

    LaunchedEffect(instanceId, initialSubdir) {
        if (instanceId != null) {
            val inst: GameInstance? = InstanceRepository.getById(instanceId)
            if (inst != null) {
                vm.startForInstance(inst, baseDir, initialSubdir)
                return@LaunchedEffect
            }
        }
        // Fallback: open the default game root.
        vm.start(roots = listOf(baseDir), startPath = baseDir, title = "文件")
    }

    LaunchedEffect(state.errorMessage) {
        val msg = state.errorMessage
        if (msg != null) {
            snackbar.showSnackbar(msg)
            vm.consumeError()
        }
    }

    val pickFile = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.GetContent(),
    ) { uri: Uri? ->
        if (uri != null) {
            scope.launch {
                val bytes = withContext(Dispatchers.IO) {
                    context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                }
                if (bytes == null) {
                    snackbar.showSnackbar("无法读取文件")
                    return@launch
                }
                val name = uri.lastPathSegment?.substringAfterLast('/') ?: "imported"
                val dest = "${state.currentPath.trimEnd('/')}/$name"
                vm.importBytes(dest, bytes)
            }
        }
    }

    Box(modifier = Modifier.fillMaxSize().padding(pad)) {
        Column(modifier = Modifier.fillMaxSize()) {
            HeaderBar(
                title = state.title.ifBlank { "文件" },
                path = state.currentPath,
                canGoUp = state.parent.isNotBlank() &&
                    state.roots.none { canonicalize(it) == state.parent },
                busy = state.busy,
                onBack = { navController?.popBackStack() },
                onUp = { vm.goUp() },
                onRefresh = { vm.refresh() },
                onImport = { pickFile.launch("*/*") },
            )
            PathBar(path = state.currentPath)
            if (state.busy) {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            }
            Row(
                modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                FilledTonalButton(onClick = { vm.requestCreateDialog() }) {
                    Icon(Icons.Filled.CreateNewFolder, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.size(6.dp))
                    Text("新建文件夹")
                }
                FilledTonalButton(onClick = { pickFile.launch("*/*") }) {
                    Icon(Icons.Filled.FileDownload, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.size(6.dp))
                    Text("导入")
                }
                FilledTonalButton(
                    onClick = { vm.requestDelete(state.entries.filter { it.isSelected }.map { it.path }) },
                    enabled = state.entries.any { it.isSelected },
                ) {
                    Icon(Icons.Filled.Delete, contentDescription = null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.size(6.dp))
                    Text("删除")
                }
            }
            HorizontalDivider()
            if (state.entries.isEmpty() && !state.busy) {
                EmptyState()
            } else {
                LazyColumn(
                    modifier = Modifier.fillMaxSize().widthIn(max = window.maxContentWidthDp.dp),
                    contentPadding = PaddingValues(vertical = 4.dp),
                ) {
                    items(state.entries, key = { it.path }) { e ->
                        EntryRow(
                            entry = e,
                            selected = e.isSelected,
                            onToggle = { e.isSelected = !e.isSelected },
                            onOpen = { vm.open(e) },
                            onRename = { vm.requestRenameDialog(e) },
                            onCopy = { vm.requestCopyDialog(e) },
                            onExtractHere = { vm.extractZip(e.path, state.currentPath) },
                            onDelete = { vm.requestDelete(listOf(e.path)) },
                        )
                    }
                }
            }
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp),
        ) { data -> Snackbar { Text(data.visuals.message) } }
    }

    // Dialogs driven by the ViewModel.
    val dialog = state.dialog
    if (dialog != null) {
        when (dialog) {
            is FileManagerDialogState.Create ->
                NameInputDialog(
                    title = "新建文件夹",
                    label = "名称",
                    initial = "",
                    onConfirm = { vm.createDirectory(it) },
                    onDismiss = { vm.dismissDialog() },
                )
            is FileManagerDialogState.Rename -> {
                val target = state.entries.firstOrNull { it.path == dialog.path }
                NameInputDialog(
                    title = "重命名",
                    label = "新名称",
                    initial = target?.name ?: "",
                    onConfirm = { vm.rename(dialog.path, it) },
                    onDismiss = { vm.dismissDialog() },
                )
            }
            is FileManagerDialogState.Copy -> {
                val target = state.entries.firstOrNull { it.path == dialog.source }
                NameInputDialog(
                    title = "复制到…",
                    label = "新名称",
                    initial = target?.name?.let { addCopySuffix(it) } ?: "",
                    onConfirm = { vm.copyFile(dialog.source, "${state.currentPath.trimEnd('/')}/$it") },
                    onDismiss = { vm.dismissDialog() },
                )
            }
        }
    }

    state.pendingDelete?.let { pending ->
        ConfirmDeleteDialog(
            paths = pending.paths,
            totalBytes = pending.totalBytes,
            hasDirectories = pending.hasDirectories,
            onConfirm = { vm.confirmPendingDelete() },
            onDismiss = { vm.cancelPendingDelete() },
        )
    }
    state.pendingMove?.let { pending ->
        AlertDialog(
            onDismissRequest = { vm.cancelPendingMove() },
            title = { Text("覆盖目标？") },
            text = {
                Text(
                    "目标 ${pending.destination} 已存在，合计 ${prettyBytes(pending.totalBytes)}。" +
                        "继续操作将覆盖原文件。",
                )
            },
            confirmButton = {
                TextButton(onClick = { vm.confirmPendingMove() }) { Text("覆盖") }
            },
            dismissButton = {
                TextButton(onClick = { vm.cancelPendingMove() }) { Text("取消") }
            },
        )
    }
}

@Composable
private fun HeaderBar(
    title: String,
    path: String,
    canGoUp: Boolean,
    busy: Boolean,
    onBack: () -> Unit,
    onUp: () -> Unit,
    onRefresh: () -> Unit,
    onImport: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        IconButton(onClick = onBack) {
            Icon(Icons.Filled.ArrowBack, contentDescription = "返回")
        }
        Column(modifier = Modifier.weight(1f)) {
            Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                path,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                fontFamily = FontFamily.Monospace,
            )
        }
        IconButton(onClick = onUp, enabled = canGoUp) {
            Icon(Icons.Filled.ArrowBack, contentDescription = "上一级")
        }
        IconButton(onClick = onRefresh, enabled = !busy) {
            Icon(Icons.Filled.Refresh, contentDescription = "刷新")
        }
    }
}

@Composable
private fun PathBar(path: String) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceVariant,
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 2.dp),
        shape = RoundedCornerShape(8.dp),
    ) {
        Text(
            text = path,
            modifier = Modifier.fillMaxWidth().padding(8.dp),
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun EmptyState() {
    Box(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            "此目录为空。\n点击「新建文件夹」或「导入」开始。",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun EntryRow(
    entry: FsEntry,
    selected: Boolean,
    onToggle: () -> Unit,
    onOpen: () -> Unit,
    onRename: () -> Unit,
    onCopy: () -> Unit,
    onExtractHere: () -> Unit,
    onDelete: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable {
                if (entry.isDirectory) {
                    onOpen()
                } else {
                    onToggle()
                }
            }
            .background(
                if (selected) MaterialTheme.colorScheme.secondaryContainer
                else androidx.compose.ui.graphics.Color.Transparent,
            )
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            imageVector = when {
                entry.isDirectory -> Icons.Filled.Folder
                entry.isSymlink -> Icons.Filled.Description
                else -> Icons.Filled.Description
            },
            contentDescription = null,
            tint = if (entry.isDirectory) MaterialTheme.colorScheme.primary
            else MaterialTheme.colorScheme.onSurface,
        )
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = entry.name,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                text = when {
                    entry.isDirectory -> "文件夹"
                    entry.isSymlink -> "链接"
                    else -> prettyBytes(entry.size)
                },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Box {
            IconButton(onClick = { menuOpen = true }) {
                Icon(Icons.Filled.MoreVert, contentDescription = "更多")
            }
            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                if (entry.isDirectory) {
                    DropdownMenuItem(
                        text = { Text("打开") },
                        onClick = { menuOpen = false; onOpen() },
                        leadingIcon = { Icon(Icons.Filled.Visibility, contentDescription = null) },
                    )
                }
                DropdownMenuItem(
                    text = { Text("重命名") },
                    onClick = { menuOpen = false; onRename() },
                    leadingIcon = { Icon(Icons.Filled.Edit, contentDescription = null) },
                )
                if (entry.isFile) {
                    DropdownMenuItem(
                        text = { Text("复制…") },
                        onClick = { menuOpen = false; onCopy() },
                        leadingIcon = { Icon(Icons.Filled.ContentCopy, contentDescription = null) },
                    )
                    if (entry.name.endsWith(".zip", ignoreCase = true)) {
                        DropdownMenuItem(
                            text = { Text("解压到当前目录") },
                            onClick = { menuOpen = false; onExtractHere() },
                            leadingIcon = { Icon(Icons.Filled.DriveFileMove, contentDescription = null) },
                        )
                    }
                }
                DropdownMenuItem(
                    text = { Text("删除") },
                    onClick = { menuOpen = false; onDelete() },
                    leadingIcon = { Icon(Icons.Filled.Delete, contentDescription = null) },
                )
            }
        }
    }
}

@Composable
private fun NameInputDialog(
    title: String,
    label: String,
    initial: String,
    onConfirm: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    var value by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = value,
                onValueChange = { value = it },
                label = { Text(label) },
                singleLine = true,
            )
        },
        confirmButton = {
            TextButton(
                onClick = { onConfirm(value) },
                enabled = value.isNotBlank(),
            ) { Text("确定") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("取消") } },
    )
}

@Composable
private fun ConfirmDeleteDialog(
    paths: List<String>,
    totalBytes: Long,
    hasDirectories: Boolean,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("确认删除？") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(
                    if (hasDirectories) {
                        "将删除 ${paths.size} 项（含子目录），合计 ${prettyBytes(totalBytes)}。"
                    } else {
                        "将删除 ${paths.size} 项，合计 ${prettyBytes(totalBytes)}。"
                    },
                )
                Text(
                    text = paths.joinToString(separator = "\n") { "· $it" },
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 6,
                )
                Text(
                    "此操作不可撤销。",
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text("删除", color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("取消") } },
    )
}

private fun canonicalize(path: String): String {
    val f = File(path)
    return try {
        f.canonicalPath
    } catch (_: Throwable) {
        f.absolutePath
    }
}

private fun addCopySuffix(name: String): String {
    val dot = name.lastIndexOf('.')
    return if (dot <= 0) "$name-copy" else "${name.substring(0, dot)}-copy${name.substring(dot)}"
}

private fun prettyBytes(bytes: Long): String = when {
    bytes < 1024 -> "$bytes B"
    bytes < 1024L * 1024 -> "%.1f KB".format(bytes / 1024.0)
    bytes < 1024L * 1024 * 1024 -> "%.1f MB".format(bytes / (1024.0 * 1024))
    else -> "%.2f GB".format(bytes / (1024.0 * 1024 * 1024))
}
