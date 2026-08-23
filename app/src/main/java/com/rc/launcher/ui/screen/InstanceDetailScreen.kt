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
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.model.GameDirectoryType
import com.rc.launcher.ui.model.GameInstance
import com.rc.launcher.ui.model.effectiveGameDir
import com.rc.launcher.ui.model.lastPlayedLabel
import com.rc.launcher.ui.viewmodel.DashboardViewModel
import com.rc.launcher.ui.viewmodel.InstanceDetailViewModel
import com.rc.launcher.ui.viewmodel.LaunchState

/** Base path used only for the on-screen preview of the resolved game directory. */
private const val PREVIEW_BASE_DIR = "games/RC"

private val ICON_COLORS = listOf(
    0xFF4CAF50, 0xFF42A5F5, 0xFFEF5350, 0xFFAB47BC,
    0xFF8D6E63, 0xFFFFB300, 0xFF26C6DA, 0xFFEC407A,
)
private val JAVA_VERSIONS = listOf(8, 17, 21)

/**
 * Instance detail & settings editor (task 13).
 *
 * Surfaces the resolved version metadata (loader family, game/loader version,
 * Java requirement, last-played) and lets the user edit the per-instance
 * settings that drive version isolation and launch behaviour. Every edit is
 * written straight back to the [com.rc.launcher.ui.model.InstanceRepository]
 * through [InstanceDetailViewModel] (mirrors FCL's `VersionSetting`). A launch
 * button reuses the shared [DashboardViewModel] state machine, and a guarded
 * delete removes the instance.
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

    val launching = when (val ls = launchState) {
        is LaunchState.Launching -> ls.instanceId == id
        is LaunchState.Running -> ls.instanceId == id
        else -> false
    }

    if (instance == null) {
        Column(
            modifier = Modifier.fillMaxSize().padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        ) {
            Text("未找到实例", style = MaterialTheme.typography.headlineSmall)
            Text("该实例可能已被删除。", style = MaterialTheme.typography.bodyMedium)
            TextButton(onClick = { navController?.popBackStack() }) { Text("返回") }
        }
        return
    }

    val inst = instance!!
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // Header: cover + name + loader badge
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Box(
                modifier = Modifier.size(64.dp).clip(RoundedCornerShape(14.dp)).background(Color(inst.iconColor)),
                contentAlignment = Alignment.Center,
            ) {
                Text(inst.version.take(5), color = Color.White, fontWeight = FontWeight.Bold)
            }
            Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(inst.name, style = MaterialTheme.typography.titleLarge)
                Surface(
                    color = inst.modLoader.color.copy(alpha = 0.18f),
                    shape = RoundedCornerShape(6.dp),
                ) {
                    Text(
                        inst.loaderLabel,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                        style = MaterialTheme.typography.labelSmall,
                        color = inst.modLoader.color,
                    )
                }
            }
        }

        // Version metadata card
        Surface(tonalElevation = 1.dp, shape = RoundedCornerShape(12.dp), modifier = Modifier.fillMaxWidth()) {
            Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("版本信息", style = MaterialTheme.typography.titleMedium)
                InfoRow("游戏版本", inst.version)
                InfoRow("加载器", inst.modLoader.label)
                InfoRow("加载器版本", inst.loaderVersion ?: "—")
                InfoRow("Java 版本", inst.javaLabel)
                InfoRow("上次游玩", inst.lastPlayedLabel())
                InfoRow("游戏目录", inst.effectiveGameDir(PREVIEW_BASE_DIR))
            }
        }

        // Editable settings
        OutlinedTextField(
            value = inst.name,
            onValueChange = vm::setName,
            label = { Text("实例名称") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = inst.notes,
            onValueChange = vm::setNotes,
            label = { Text("备注") },
            minLines = 2,
            maxLines = 4,
            modifier = Modifier.fillMaxWidth(),
        )

        Text("封面颜色", style = MaterialTheme.typography.labelLarge)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            for (c in ICON_COLORS) {
                val selected = inst.iconColor == c
                Box(
                    modifier = Modifier.size(36.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(Color(c))
                        .clickable { vm.setIconColor(c) }
                        .then(if (selected) Modifier.border(2.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp)) else Modifier),
                    contentAlignment = Alignment.Center,
                ) {
                    if (selected) Icon(Icons.Filled.Check, contentDescription = null, tint = Color.White)
                }
            }
        }

        Text("Java 版本", style = MaterialTheme.typography.labelLarge)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Chip(selected = inst.javaVersion == null, onClick = { vm.setJavaVersion(null) }) { Text("自动") }
            for (jv in JAVA_VERSIONS) {
                Chip(selected = inst.javaVersion == jv, onClick = { vm.setJavaVersion(jv) }) { Text("Java $jv") }
            }
        }

        Text("版本隔离", style = MaterialTheme.typography.labelLarge)
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (type in GameDirectoryType.entries) {
                val selected = inst.gameDirectoryType == type
                Surface(
                    modifier = Modifier.fillMaxWidth()
                        .clip(RoundedCornerShape(10.dp))
                        .clickable { vm.setGameDirectoryType(type) },
                    color = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
                    tonalElevation = 1.dp,
                ) {
                    Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(type.label, style = MaterialTheme.typography.titleMedium)
                        Text(type.description, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
        if (inst.gameDirectoryType == GameDirectoryType.CUSTOM) {
            OutlinedTextField(
                value = inst.customGameDir ?: "",
                onValueChange = vm::setCustomGameDir,
                label = { Text("自定义目录路径") },
                placeholder = { Text("/sdcard/games/my-instance") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
        }

        // Actions
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(
                onClick = { dashboard.launch(inst.id) },
                enabled = !launching,
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.PlayArrow, contentDescription = null)
                Text(if (launching) "启动中…" else "启动")
            }
            TextButton(
                onClick = { confirmDelete = true },
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.Delete, contentDescription = null)
                Text("删除实例")
            }
        }
    }

    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text("删除实例？") },
            text = { Text("将移除「${inst.name}」及其版本隔离目录的引用，此操作不可撤销。") },
            confirmButton = {
                TextButton(onClick = {
                    vm.delete()
                    confirmDelete = false
                    navController?.popBackStack()
                }) { Text("删除") }
            },
            dismissButton = {
                TextButton(onClick = { confirmDelete = false }) { Text("取消") }
            },
        )
    }
}

@Composable
private fun InfoRow(label: String, value: String) {
    Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun Chip(selected: Boolean, onClick: () -> Unit, content: @Composable () -> Unit) {
    Surface(
        modifier = Modifier.clip(RoundedCornerShape(20.dp)).clickable { onClick() },
        color = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp,
    ) {
        Box(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) { content() }
    }
}
