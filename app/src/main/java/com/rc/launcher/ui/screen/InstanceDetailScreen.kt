package com.rc.launcher.ui.screen

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.FolderOpen
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Speed
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.outlined.StarBorder
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Palette
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.component.ProgressRing
import com.rc.launcher.ui.component.ResourceSummary
import com.rc.launcher.ui.model.GameDirectoryType
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.effectiveGameDir
import com.rc.launcher.ui.navigation.InstanceDetailRoute
import com.rc.launcher.ui.navigation.FileManagerRoute
import com.rc.launcher.ui.navigation.WorldManagerRoute
import com.rc.launcher.ui.navigation.ResourcePackManagerRoute
import com.rc.launcher.ui.navigation.ShaderPackManagerRoute
import com.rc.launcher.ui.model.lastPlayedLabel
import com.rc.launcher.ui.resource.rememberFps
import com.rc.launcher.ui.resource.rememberResourceUsage
import com.rc.launcher.ui.viewmodel.DashboardViewModel
import com.rc.launcher.ui.viewmodel.InstanceDetailViewModel
import com.rc.launcher.ui.viewmodel.LaunchState
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString

/** Base path used only for the on-screen preview of the resolved game directory. */
private const val PREVIEW_BASE_DIR = "games/RC"

private val ICON_COLORS = listOf(
    0xFF4CAF50, 0xFF42A5F5, 0xFFEF5350, 0xFFAB47BC,
    0xFF8D6E63, 0xFFFFB300, 0xFF26C6DA, 0xFFEC407A,
)
private val JAVA_VERSIONS = listOf(8, 17, 21)

/**
 * Instance detail & settings editor (task 13 + task 18 visual redesign).
 *
 * Surfaces the resolved version metadata (loader family, game/loader version,
 * Java requirement, last-played), an **integrated resource-usage panel**
 * (mirrors the home dashboard's [ResourceSummary] + live FPS, so the user sees
 * the device pressure before launching), the launcher settings (per-instance
 * name, notes, cover color, Java version, version-isolation strategy), and
 * one-tap launch / favourite / duplicate / delete actions.
 *
 * Every edit is written straight back to the
 * [com.rc.launcher.ui.model.InstanceRepository] through
 * [InstanceDetailViewModel] (mirrors FCL's `VersionSetting`).
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun InstanceDetailScreen(
    id: String,
    navController: NavHostController? = null,
    vm: InstanceDetailViewModel = viewModel(),
    dashboard: DashboardViewModel = viewModel(),
) {
    val instance by vm.instance.collectAsStateWithLifecycle()
    val launchState by dashboard.launchState.collectAsStateWithLifecycle()
    var confirmDelete by remember { mutableStateOf(false) }

    LaunchedEffect(id) { vm.load(id) }

    val launchStateVal = launchState
    val launching = when (launchStateVal) {
        is LaunchState.Launching -> launchStateVal.instanceId == id
        is LaunchState.Running -> launchStateVal.instanceId == id
        else -> false
    }

    if (instance == null) {
        NotFoundState(onBack = { navController?.popBackStack() })
        return
    }

    val inst = instance!!
    val fps by rememberFps()
    val usage by rememberResourceUsage()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        // Modern hero header (task 18): cover with gradient + version watermark
        // + favourite star, name, loader chip, and a primary launch button.
        DetailHero(
            inst = inst,
            launching = launching,
            onLaunch = { dashboard.launch(inst.id) },
            onToggleFavorite = { vm.toggleFavorite() },
            onRename = { vm.setName(it) },
            navController = navController,
        )

        // Live resource usage overview (task 18 "resource usage overview
        // (ResourceMonitor/FpsTracker)"). The same `rememberResourceUsage()` +
        // `rememberFps()` hooks the home dashboard uses, so the user sees
        // exactly the same numbers they would see *after* launch.
        ElevatedCard(
            modifier = Modifier.fillMaxWidth(),
            elevation = CardDefaults.elevatedCardElevation(defaultElevation = 1.dp),
            shape = RoundedCornerShape(14.dp),
        ) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Icon(
                        imageVector = Icons.Filled.Speed,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(18.dp),
                    )
                    Text(
                        text = "\u8d44\u6e90\u4e0e\u6027\u80fd",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Spacer(Modifier.weight(1f))
                    FpsBadge(fps)
                }
                ResourceSummary(usage = usage)
            }
        }

        // Version metadata card
        ElevatedCard(
            modifier = Modifier.fillMaxWidth(),
            elevation = CardDefaults.elevatedCardElevation(defaultElevation = 1.dp),
            shape = RoundedCornerShape(14.dp),
        ) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                SectionHeader("\u7248\u672c\u4fe1\u606f")
                InfoRow("\u6e38\u620f\u7248\u672c", inst.version)
                InfoRow("\u52a0\u8f7d\u5668", inst.modLoader.label)
                InfoRow("\u52a0\u8f7d\u5668\u7248\u672c", inst.loaderVersion ?: "\u2014")
                InfoRow("Java \u7248\u672c", inst.javaLabel)
                InfoRow("\u4e0a\u6b21\u6e38\u73a9", inst.lastPlayedLabel())
                InfoRow("\u6e38\u620f\u76ee\u5f55", inst.effectiveGameDir(PREVIEW_BASE_DIR))
            }
        }

        // Editable settings
        ElevatedCard(
            modifier = Modifier.fillMaxWidth(),
            elevation = CardDefaults.elevatedCardElevation(defaultElevation = 1.dp),
            shape = RoundedCornerShape(14.dp),
        ) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                SectionHeader("\u8bbe\u7f6e")

                OutlinedTextField(
                    value = inst.name,
                    onValueChange = vm::setName,
                    label = { Text("\u5b9e\u4f8b\u540d\u79f0") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )

                OutlinedTextField(
                    value = inst.notes,
                    onValueChange = vm::setNotes,
                    label = { Text("\u5907\u6ce8") },
                    minLines = 2,
                    maxLines = 4,
                    modifier = Modifier.fillMaxWidth(),
                )

                Text("\u5c01\u9762\u989c\u8272", style = MaterialTheme.typography.labelLarge)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    for (c in ICON_COLORS) {
                        val selected = inst.iconColor == c
                        Box(
                            modifier = Modifier.size(36.dp)
                                .clip(RoundedCornerShape(10.dp))
                                .background(Color(c))
                                .clickable { vm.setIconColor(c) }
                                .then(
                                    if (selected) Modifier.border(
                                        2.dp,
                                        MaterialTheme.colorScheme.outline,
                                        RoundedCornerShape(10.dp),
                                    ) else Modifier,
                                ),
                            contentAlignment = Alignment.Center,
                        ) {
                            if (selected) {
                                Icon(
                                    Icons.Filled.Check,
                                    contentDescription = null,
                                    tint = Color.White,
                                )
                            }
                        }
                    }
                }

                Text("Java \u7248\u672c", style = MaterialTheme.typography.labelLarge)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Chip(
                        selected = inst.javaVersion == null,
                        onClick = { vm.setJavaVersion(null) },
                        label = "\u81ea\u52a8",
                    )
                    for (jv in JAVA_VERSIONS) {
                        Chip(
                            selected = inst.javaVersion == jv,
                            onClick = { vm.setJavaVersion(jv) },
                            label = "Java $jv",
                        )
                    }
                }

                Text("\u7248\u672c\u9694\u79bb", style = MaterialTheme.typography.labelLarge)
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (type in GameDirectoryType.entries) {
                        val selected = inst.gameDirectoryType == type
                        Surface(
                            modifier = Modifier.fillMaxWidth()
                                .clip(RoundedCornerShape(10.dp))
                                .clickable { vm.setGameDirectoryType(type) },
                            color = if (selected) MaterialTheme.colorScheme.primaryContainer
                            else MaterialTheme.colorScheme.surface,
                            tonalElevation = 1.dp,
                        ) {
                            Column(
                                modifier = Modifier.fillMaxWidth().padding(14.dp),
                                verticalArrangement = Arrangement.spacedBy(2.dp),
                            ) {
                                Text(type.label, style = MaterialTheme.typography.titleMedium)
                                Text(
                                    type.description,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
                if (inst.gameDirectoryType == GameDirectoryType.CUSTOM) {
                    OutlinedTextField(
                        value = inst.customGameDir ?: "",
                        onValueChange = vm::setCustomGameDir,
                        label = { Text("\u81ea\u5b9a\u4e49\u76ee\u5f55\u8def\u5f84") },
                        placeholder = { Text("/sdcard/games/my-instance") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            }
        }

        // Primary actions row: a prominent launch button + secondary actions
        // (duplicate / delete) so the user does not have to scroll back up
        // for the actual entry point.
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Button(
                onClick = { dashboard.launch(inst.id) },
                enabled = !launching,
                modifier = Modifier.fillMaxWidth().height(52.dp),
            ) {
                Icon(Icons.Filled.PlayArrow, contentDescription = null)
                Text(
                    text = if (launching) "\u542f\u52a8\u4e2d\u2026" else "\u4e00\u952e\u542f\u52a8",
                    modifier = Modifier.padding(start = 8.dp),
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                FilledTonalButton(
                    onClick = {
                        vm.duplicate()?.let { newId ->
                            navController?.navigate(InstanceDetailRoute(newId))
                        }
                    },
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(Icons.Filled.ContentCopy, contentDescription = null)
                    Text("\u590d\u5236", modifier = Modifier.padding(start = 6.dp))
                }
                FilledTonalButton(
                    onClick = { confirmDelete = true },
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(Icons.Filled.Delete, contentDescription = null)
                    Text("\u5220\u9664", modifier = Modifier.padding(start = 6.dp))
                }
            }
        }
    }

    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text("\u5220\u9664\u5b9e\u4f8b\uff1f") },
            text = { Text("\u5c06\u79fb\u9664\u300c${inst.name}\u300d\u53ca\u5176\u7248\u672c\u9694\u79bb\u76ee\u5f55\u7684\u5f15\u7528\uff0c\u6b64\u64cd\u4f5c\u4e0d\u53ef\u64a4\u9500\u3002") },
            confirmButton = {
                TextButton(onClick = {
                    vm.delete()
                    confirmDelete = false
                    navController?.popBackStack()
                }) { Text("\u5220\u9664") }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text("\u53d6\u6d88") }
            },
        )
    }
}

@Composable
private fun NotFoundState(onBack: () -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
    ) {
        Text("\u672a\u627e\u5230\u5b9e\u4f8b", style = MaterialTheme.typography.headlineSmall)
        Text(
            "\u8be5\u5b9e\u4f8b\u53ef\u80fd\u5df2\u88ab\u5220\u9664\u3002",
            style = MaterialTheme.typography.bodyMedium,
        )
        TextButton(onClick = onBack) { Text("\u8fd4\u56de") }
    }
}

@Composable
private fun DetailHero(
    inst: GameInstance,
    launching: Boolean,
    onLaunch: () -> Unit,
    onToggleFavorite: () -> Unit,
    onRename: (String) -> Unit,
    navController: NavHostController? = null,
) {
    val cover = remember(inst.iconColor) {
        Brush.linearGradient(
            listOf(
                Color(inst.iconColor).copy(
                    red = (Color(inst.iconColor).red * 0.72f).coerceIn(0f, 1f),
                    green = (Color(inst.iconColor).green * 0.72f).coerceIn(0f, 1f),
                    blue = (Color(inst.iconColor).blue * 0.72f).coerceIn(0f, 1f),
                    alpha = 1f,
                ),
                Color(inst.iconColor),
            ),
        )
    }
    ElevatedCard(
        modifier = Modifier.fillMaxWidth(),
        elevation = CardDefaults.elevatedCardElevation(defaultElevation = 3.dp),
        shape = RoundedCornerShape(18.dp),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(132.dp)
                .background(cover),
        ) {
            Text(
                text = inst.version.take(10),
                modifier = Modifier
                    .align(Alignment.CenterStart)
                    .padding(start = 18.dp),
                color = Color.White.copy(alpha = 0.92f),
                fontWeight = FontWeight.Black,
                style = MaterialTheme.typography.displaySmall,
            )
            IconButton(
                onClick = onToggleFavorite,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(8.dp)
                    .clip(RoundedCornerShape(50))
                    .background(Color.Black.copy(alpha = 0.18f)),
            ) {
                Icon(
                    imageVector = if (inst.isFavorite) Icons.Filled.Star else Icons.Outlined.StarBorder,
                    contentDescription = if (inst.isFavorite) "\u53d6\u6d88\u6536\u85cf" else "\u6536\u85cf\u5b9e\u4f8b",
                    tint = if (inst.isFavorite) MaterialTheme.colorScheme.tertiary
                    else Color.White.copy(alpha = 0.85f),
                )
            }
            if (launching) {
                ProgressRing(
                    progress = 0.5f,
                    color = Color.White,
                    track = Color.White.copy(alpha = 0.28f),
                    modifier = Modifier
                        .align(Alignment.BottomEnd)
                        .padding(12.dp)
                        .size(46.dp),
                )
            }
        }
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                text = inst.name,
                style = MaterialTheme.typography.titleLarge,
                fontWeight = FontWeight.SemiBold,
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Surface(
                    color = inst.modLoader.color.copy(alpha = 0.18f),
                    contentColor = inst.modLoader.color,
                    shape = RoundedCornerShape(6.dp),
                ) {
                    Text(
                        text = inst.loaderLabel,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                        style = MaterialTheme.typography.labelSmall,
                    )
                }
                AssistChip(
                    onClick = { /* long-press would open directory picker */ },
                    label = { Text(inst.gameDirectoryType.label) },
                    leadingIcon = {
                        Icon(
                            Icons.Filled.FolderOpen,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                )
                // Task 19: deep-link into the in-app small file manager,
                // rooted at the per-instance effective game dir. From there
                // the user can browse saves/mods/resourcepacks/shaderpacks
                // and import / export files without leaving the launcher.
                AssistChip(
                    onClick = {
                        navController?.navigate(FileManagerRoute(instanceId = inst.id))
                    },
                    label = { Text(rcString(RcStringKeys.SCREEN_FILE_MANAGER)) },
                    leadingIcon = {
                        Icon(
                            Icons.Filled.FolderOpen,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                )
                // Task 26: world / save archive management
                AssistChip(
                    onClick = {
                        navController?.navigate(WorldManagerRoute(instanceId = inst.id))
                    },
                    label = { Text(rcString(RcStringKeys.SCREEN_WORLD_MANAGER_TITLE)) },
                    leadingIcon = {
                        Icon(
                            Icons.Filled.Save,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                )
                // Task 27: resource-pack management
                AssistChip(
                    onClick = {
                        navController?.navigate(ResourcePackManagerRoute(instanceId = inst.id))
                    },
                    label = { Text(rcString(RcStringKeys.SCREEN_RESOURCE_PACK_TITLE)) },
                    leadingIcon = {
                        Icon(
                            Icons.Filled.Palette,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                )
                // Task 27: shader-pack management
                AssistChip(
                    onClick = {
                        navController?.navigate(ShaderPackManagerRoute(instanceId = inst.id))
                    },
                    label = { Text(rcString(RcStringKeys.SCREEN_SHADER_PACK_TITLE)) },
                    leadingIcon = {
                        Icon(
                            Icons.Filled.AutoAwesome,
                            contentDescription = null,
                            modifier = Modifier.size(16.dp),
                        )
                    },
                )
            }
            if (inst.notes.isNotBlank()) {
                Text(
                    text = inst.notes,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun FpsBadge(fps: Int) {
    Surface(
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        shape = RoundedCornerShape(50),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
        ) {
            Icon(
                imageVector = Icons.Filled.Memory,
                contentDescription = null,
                modifier = Modifier.size(14.dp),
            )
            Text(
                text = "FPS ${fps.coerceAtLeast(0)}",
                style = MaterialTheme.typography.labelSmall,
            )
        }
    }
}

@Composable
private fun SectionHeader(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
    )
}

@Composable
private fun InfoRow(label: String, value: String) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun Chip(selected: Boolean, onClick: () -> Unit, label: String) {
    FilterChip(
        selected = selected,
        onClick = onClick,
        label = { Text(label) },
    )
}
