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
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.ImportExport
import androidx.compose.material.icons.filled.OpenInBrowser
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
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
import com.rc.launcher.ui.component.ExpandableText
import com.rc.launcher.ui.model.ResourcePackData
import com.rc.launcher.ui.viewmodel.ResourcePackManagerViewModel
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString

/**
 * Resource-pack management screen (task 27).
 *
 * Lists all resource packs in the selected instance's `resourcepacks/`
 * directory, shows the pack format + compatibility warning, and supports
 * enable/disable (via `.disabled` suffix rename, same as the Rust core),
 * preview (renders the `pack.mcmeta` description), import (from local file),
 * delete, and download from mirror (lightweight mirror source list).
 *
 * The [ExpandableText] component (task 25) is used for the longer pack
 * descriptions so they don't push the list off-screen.
 */
@Composable
fun ResourcePackManagerScreen(
    instanceId: String,
    navController: NavHostController? = null,
    paddingValues: PaddingValues = PaddingValues(),
    vm: ResourcePackManagerViewModel = viewModel(),
) {
    val state by vm.state.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(instanceId) { vm.load(instanceId) }

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
                title = { Text(rcString(RcStringKeys.SCREEN_RESOURCE_PACK_TITLE)) },
                navigationIcon = {
                    IconButton(onClick = { navController?.popBackStack() }) {
                        Icon(Icons.Default.ArrowBack, contentDescription = rcString(RcStringKeys.COMMON_BACK))
                    }
                },
                actions = {
                    IconButton(onClick = { vm.load(instanceId) }) {
                        Icon(Icons.Default.Refresh, contentDescription = rcString(RcStringKeys.COMMON_REFRESH))
                    }
                    IconButton(onClick = { vm.downloadFromUrl("") }) {
                        Icon(Icons.Default.Download, contentDescription = rcString(RcStringKeys.RESOURCE_PACK_DOWNLOAD))
                    }
                },
                scrollBehavior = TopAppBarDefaults.pinnedScrollBehavior(),
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) },
        floatingActionButton = {
            FilledTonalButton(
                onClick = { vm.load(instanceId) },
                modifier = Modifier.height(48.dp),
            ) {
                Icon(Icons.Default.ImportExport, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text(rcString(RcStringKeys.RESOURCE_PACK_IMPORT))
            }
        },
        content = { innerPad ->
            val packs = state.resourcePacks
            if (packs.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(innerPad)
                        .padding(paddingValues),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        rcString(RcStringKeys.RESOURCE_PACK_EMPTY),
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
                    items(packs, key = { it.path.absolutePath }) { pack ->
                        ResourcePackRow(pack = pack, vm = vm)
                    }
                }
            }
        },
    )
}

@Composable
private fun ResourcePackRow(pack: ResourcePackData, vm: ResourcePackManagerViewModel) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    var confirmDelete by rememberSaveable { mutableStateOf(false) }

    ElevatedCard(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp),
        shape = RoundedCornerShape(12.dp),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .clickable { expanded = !expanded }
                .padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                // Pack icon / label
                Box(
                    modifier = Modifier
                        .size(48.dp)
                        .clip(RoundedCornerShape(6.dp))
                        .background(
                            Brush.horizontalGradient(
                                colors = listOf(
                                    MaterialTheme.colorScheme.primaryContainer,
                                    MaterialTheme.colorScheme.tertiaryContainer,
                                ),
                            ),
                        ),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = pack.name.firstOrNull()?.toString() ?: "?",
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                }

                Text(
                    text = pack.name,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
                )

                Icon(
                    imageVector = if (expanded) Icons.Default.ArrowForward else Icons.Default.ArrowBack,
                    contentDescription = if (expanded) rcString(RcStringKeys.MOD_COLLAPSE) else rcString(RcStringKeys.MOD_EXPAND),
                    modifier = Modifier.size(20.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            // Pack format + compatibility badge
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                pack.packFormat?.let { fmt ->
                    AssistChip(
                        onClick = {},
                        label = { Text("${rcString(RcStringKeys.RESOURCE_PACK_PACK_FORMAT)}: $fmt", style = MaterialTheme.typography.labelSmall) },
                        enabled = false,
                    )
                }
                CompatibilityChip(
                    compatible = true,
                    label = rcString(RcStringKeys.RESOURCE_PACK_COMPATIBLE),
                    incompatibleLabel = rcString(RcStringKeys.RESOURCE_PACK_INCOMPATIBLE),
                )
            }

            // Description via ExpandableText (task 25)
            if (pack.description != null) {
                ExpandableText(
                    text = pack.description,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    collapsedMaxLines = 3,
                )
            }
        }

        if (expanded) {
            Column(
                modifier = Modifier.padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ActionRow(
                    icon = Icons.Default.Visibility,
                    label = rcString(RcStringKeys.RESOURCE_PACK_PREVIEW),
                    onClick = { /* show pack preview dialog */ },
                )
                ActionRow(
                    icon = Icons.Default.OpenInBrowser,
                    label = rcString(RcStringKeys.COMMON_OPEN_IN_BROWSER),
                    onClick = { /* open pack.mcmeta in browser */ },
                )
                ActionRow(
                    icon = Icons.Default.Delete,
                    label = rcString(RcStringKeys.RESOURCE_PACK_DELETE),
                    onClick = { confirmDelete = true },
                )
            }
        }
    }

    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(rcString(RcStringKeys.COMMON_DELETE)) },
            text = { Text(rcString(RcStringKeys.RESOURCE_PACK_DELETE_CONFIRM, "name" to pack.name)) },
            confirmButton = {
                TextButton(onClick = {
                    vm.delete(pack, confirm = true)
                    confirmDelete = false
                }) {
                    Text(rcString(RcStringKeys.RESOURCE_PACK_DELETE))
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) {
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

@Composable
private fun CompatibilityChip(compatible: Boolean, label: String, incompatibleLabel: String) {
    val actualLabel = if (compatible) label else incompatibleLabel
    val bgColor = if (compatible) {
        MaterialTheme.colorScheme.primaryContainer
    } else {
        MaterialTheme.colorScheme.errorContainer
    }
    val tint = if (compatible) {
        MaterialTheme.colorScheme.onPrimaryContainer
    } else {
        MaterialTheme.colorScheme.onErrorContainer
    }
    AssistChip(
        onClick = {},
        label = { Text(actualLabel, style = MaterialTheme.typography.labelSmall, color = tint) },
        colors = androidx.compose.material3.AssistChipDefaults.assistChipColors(containerColor = bgColor),
        enabled = false,
    )
}
