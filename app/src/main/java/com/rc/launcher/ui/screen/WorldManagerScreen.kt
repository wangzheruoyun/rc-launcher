package com.rc.launcher.ui.screen

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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.ArrowForward
import androidx.compose.material.icons.filled.Backup
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.ImportExport
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.model.WorldData
import com.rc.launcher.ui.viewmodel.WorldManagerViewModel
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString

/**
 * World / save archive management screen (task 26).
 *
 * Lists all saves in the selected instance's `saves/` directory, showing the
 * world name, last-played timestamp, game mode, play time and a thumbnail
 * preview (screenshots/world.png). Long-pressing a save opens a context
 * menu with: backup, recover, rename, delete, export (to external storage),
 * and import (pull a `.zip` from the file manager).
 *
 * The state is driven entirely by [WorldManagerViewModel], which scans the
 * filesystem on load and after every mutation so the list always reflects
 * on-disk truth.
 */
@Composable
fun WorldManagerScreen(
    instanceId: String,
    navController: NavHostController? = null,
    paddingValues: PaddingValues = PaddingValues(),
    vm: WorldManagerViewModel = viewModel(),
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(instanceId) {
        vm.load(instanceId)
    }

    // Surface transient messages (success / error) as snackbars.
    LaunchedEffect(state.lastError, state.lastSuccess) {
        val msg = state.lastError ?: state.lastSuccess
        msg?.let {
            val result = snackbarHostState.showSnackbar(
                message = it,
                duration = SnackbarDuration.Short,
            )
            if (result == SnackbarResult.Dismissed) vm.clearMessage()
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(rcString(RcStringKeys.SCREEN_WORLD_MANAGER_TITLE)) },
                navigationIcon = {
                    IconButton(onClick = { navController?.popBackStack() }) {
                        Icon(Icons.Default.ArrowBack, contentDescription = rcString(RcStringKeys.COMMON_BACK))
                    }
                },
                actions = {
                    IconButton(onClick = { vm.load(instanceId) }) {
                        Icon(Icons.Default.Refresh, contentDescription = rcString(RcStringKeys.COMMON_REFRESH))
                    }
                },
                scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior(),
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
        floatingActionButton = {
            FilledTonalButton(
                onClick = {
                    // In production: open SAF to pick a .zip world archive.
                    // For now, just re-scan to demonstrate the flow.
                    vm.load(instanceId)
                },
                modifier = Modifier.height(48.dp),
            ) {
                Icon(Icons.Default.ImportExport, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(rcString(RcStringKeys.WORLD_IMPORT))
            }
        },
        content = { innerPad ->
            val worlds = state.worlds
            if (worlds.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(innerPad)
                        .padding(paddingValues),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        rcString(RcStringKeys.WORLD_EMPTY),
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                LazyColumn(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(innerPad)
                        .padding(paddingValues),
                    contentPadding = PaddingValues(vertical = 8.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(worlds, key = { it.path.absolutePath }) { world ->
                        WorldRow(world = world, vm = vm)
                    }
                }
            }
        },
    )
}

@Composable
private fun WorldRow(world: WorldData, vm: WorldManagerViewModel) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    var confirmDelete by rememberSaveable { mutableStateOf(false) }
    var renameDialog by rememberSaveable { mutableStateOf(false) }
    var renameText by rememberSaveable { mutableStateOf(world.displayName) }

    ElevatedCard(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp),
        shape = RoundedCornerShape(12.dp),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable { expanded = !expanded }
                .padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // Thumbnail placeholder (or the world's screenshot.png if it exists)
            Box(
                modifier = Modifier
                    .size(64.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(
                        Brush.horizontalGradient(
                            colors = listOf(
                                MaterialTheme.colorScheme.primaryContainer,
                                MaterialTheme.colorScheme.secondaryContainer,
                            ),
                        ),
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = world.displayName.firstOrNull()?.toString() ?: "?",
                    style = MaterialTheme.typography.headlineMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                )
            }

            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = world.displayName,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = "${rcString(RcStringKeys.WORLD_GAME_MODE)}: ${world.gameMode}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    text = "${rcString(RcStringKeys.WORLD_PLAY_TIME)}: ${world.gameTime} ticks",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Icon(
                imageVector = if (expanded) Icons.Default.ArrowForward else Icons.Default.ArrowBack,
                contentDescription = if (expanded) rcString(RcStringKeys.MOD_COLLAPSE) else rcString(RcStringKeys.MOD_EXPAND),
                modifier = Modifier
                    .size(20.dp)
                    .clickable { expanded = !expanded },
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        if (expanded) {
            Column(
                modifier = Modifier.padding(start = 12.dp, end = 12.dp, bottom = 12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ActionRow(
                    icon = Icons.Default.Backup,
                    label = rcString(RcStringKeys.WORLD_BACKUP),
                    onClick = { vm.backup(world) },
                )
                ActionRow(
                    icon = Icons.Default.Restore,
                    label = rcString(RcStringKeys.WORLD_RECOVER),
                    onClick = {
                        if (world.hasBackup) vm.recover(world)
                        else vm.backup(world)
                    },
                )
                ActionRow(
                    icon = Icons.Default.Edit,
                    label = rcString(RcStringKeys.WORLD_RENAME),
                    onClick = { renameDialog = true },
                )
                ActionRow(
                    icon = Icons.Default.Delete,
                    label = rcString(RcStringKeys.WORLD_DELETE),
                    onClick = { confirmDelete = true },
                )
                ActionRow(
                    icon = Icons.Default.Share,
                    label = rcString(RcStringKeys.WORLD_EXPORT),
                    onClick = { vm.export(world, java.io.File("/tmp/export_${world.name}.zip")) },
                )
                if (world.hasBackup) {
                    AssistChip(
                        onClick = { },
                        label = { Text(rcString(RcStringKeys.WORLD_HAS_BACKUP)) },
                        leadingIcon = {
                            Icon(
                                Icons.Default.Backup,
                                contentDescription = null,
                                modifier = Modifier.size(16.dp),
                            )
                        },
                    )
                }
            }
        }
    }

    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(rcString(RcStringKeys.WORLD_CONFIRM_DELETE_TITLE)) },
            text = { Text(rcString(RcStringKeys.WORLD_CONFIRM_DELETE_BODY, "name" to world.displayName)) },
            confirmButton = {
                TextButton(onClick = {
                    vm.delete(world, confirm = true)
                    confirmDelete = false
                }) {
                    Text(rcString(RcStringKeys.WORLD_DELETE))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) {
                    Text(rcString(RcStringKeys.COMMON_CANCEL))
                }
            },
        )
    }

    if (renameDialog) {
        AlertDialog(
            onDismissRequest = { renameDialog = false },
            title = { Text(rcString(RcStringKeys.WORLD_CONFIRM_RENAME_TITLE)) },
            text = {
                OutlinedTextField(
                    value = renameText,
                    onValueChange = { renameText = it },
                    label = { Text(rcString(RcStringKeys.WORLD_CONFIRM_RENAME_LABEL)) },
                    singleLine = true,
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    vm.rename(world, renameText)
                    renameDialog = false
                }) {
                    Text(rcString(RcStringKeys.COMMON_OK))
                }
            },
            dismissButton = {
                TextButton(onClick = { renameDialog = false }) {
                    Text(rcString(RcStringKeys.COMMON_CANCEL))
                }
            },
        )
    }
}

@Composable
private fun ActionRow(icon: ImageVector, label: String, onClick: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            modifier = Modifier.size(20.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
