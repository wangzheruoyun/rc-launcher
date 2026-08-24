package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Speed
import androidx.compose.material3.AssistChip
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.component.FloatingHud
import com.rc.launcher.ui.component.InstanceCard
import com.rc.launcher.ui.component.ResourceSummary
import com.rc.launcher.ui.model.recentlyPlayed
import com.rc.launcher.ui.model.dashboardOrder
import com.rc.launcher.ui.navigation.RcRoutes
import com.rc.launcher.ui.resource.rememberFps
import com.rc.launcher.ui.resource.rememberResourceUsage
import com.rc.launcher.ui.viewmodel.DashboardViewModel
import com.rc.launcher.ui.viewmodel.LaunchState
import com.rc.launcher.ui.viewmodel.MainUiState
import com.rc.launcher.ui.viewmodel.MainViewModel

/**
 * Home / dashboard (task 12): a greeting header, a live resource-usage panel,
 * a "最近游玩" rail, the full instance grid, a one-tap launch flow and a
 * floating frame-rate HUD. State comes from [DashboardViewModel] (instances +
 * launch lifecycle + HUD) and [MainViewModel] (Rust core greeting).
 */
@Composable
fun HomeScreen(
    navController: NavHostController? = null,
    dashboard: DashboardViewModel = viewModel(),
    main: MainViewModel = viewModel(),
) {
    val instances by dashboard.instances.collectAsStateWithLifecycle()
    val launchState by dashboard.launchState.collectAsStateWithLifecycle()
    val hudOn by dashboard.hudVisible.collectAsStateWithLifecycle()
    val coreState by main.uiState.collectAsStateWithLifecycle()

    val recent = remember(instances) { instances.recentlyPlayed(4) }
    val fps by rememberFps()
    val usage by rememberResourceUsage()
    val showHud = hudOn || launchState is LaunchState.Running

    val launchStateVal = launchState
    val launchingId = when (launchStateVal) {
        is LaunchState.Launching -> launchStateVal.instanceId
        is LaunchState.Running -> launchStateVal.instanceId
        else -> null
    }

    val openInstance: (id: String) -> Unit = { id ->
        navController?.navigate(RcRoutes.instanceDetail(id))
    }

    Box(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // Header
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text("主页", style = MaterialTheme.typography.headlineSmall)
                    Text(
                        val coreStateVal = coreState
                        text = when (coreStateVal) {
                            is MainUiState.Ready -> "核心 ${coreStateVal.coreVersion} 已就绪"
                            is MainUiState.Loading -> "正在连接核心…"
                            is MainUiState.Error -> "核心不可用：${coreStateVal.message}"
                        },
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                AssistChip(
                    onClick = { dashboard.toggleHud() },
                    label = { Text(if (hudOn) "隐藏 HUD" else "性能 HUD") },
                    leadingIcon = {
                        Icon(Icons.Filled.Speed, contentDescription = null, modifier = Modifier.size(16.dp))
                    },
                )
            }

            ResourceSummary(usage)

            if (recent.isNotEmpty()) {
                SectionTitle("最近游玩")
                LazyRow(
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(vertical = 4.dp),
                ) {
                    items(recent, key = { it.id }) { inst ->
                        InstanceCard(
                            instance = inst,
                            launching = inst.id == launchingId,
                            onLaunch = { dashboard.launch(inst.id) },
                            onOpen = { openInstance(inst.id) },
                            modifier = Modifier.fillMaxWidth().widthIn(min = 280.dp),
                        )
                    }
                }
            }

            SectionTitle("游戏实例 (${instances.size})")
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                for (inst in instances.dashboardOrder()) {
                    InstanceCard(
                        instance = inst,
                        launching = inst.id == launchingId,
                        onLaunch = { dashboard.launch(inst.id) },
                        onOpen = { openInstance(inst.id) },
                    )
                }
            }
        }

        // Launch lifecycle banner (bottom)
        when (val ls = launchState) {
            is LaunchState.Launching -> LaunchBanner(
                text = "正在启动 ${ls.instanceName}…",
                showProgress = true,
            )
            is LaunchState.Running -> LaunchBanner(
                text = "${ls.instanceName} 运行中",
                onStop = { dashboard.stop() },
            )
            is LaunchState.Failed -> LaunchBanner(
                text = "启动失败：${ls.message}",
                onDismiss = { dashboard.dismissError() },
            )
            LaunchState.Idle -> {}
        }

        // Floating frame-rate HUD (top-end overlay)
        if (showHud) {
            FloatingHud(
                fps = fps,
                usage = usage,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(top = 8.dp, end = 16.dp),
                onClose = {
                    if (launchState !is LaunchState.Running) dashboard.toggleHud()
                },
            )
        }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleMedium,
        color = MaterialTheme.colorScheme.primary,
    )
}

@Composable
private fun LaunchBanner(
    text: String,
    showProgress: Boolean = false,
    onStop: (() -> Unit)? = null,
    onDismiss: (() -> Unit)? = null,
) {
    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.BottomCenter) {
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .padding(12.dp)
                .navigationBarsPadding(),
            shape = RoundedCornerShape(14.dp),
            color = MaterialTheme.colorScheme.primaryContainer,
            tonalElevation = 4.dp,
        ) {
            Row(
                modifier = Modifier.fillMaxWidth().padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (showProgress) {
                    CircularProgressIndicator(modifier = Modifier.size(22.dp), strokeWidth = 3.dp)
                } else {
                    Icon(Icons.Filled.Info, contentDescription = null)
                }
                Text(text, modifier = Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
                onStop?.let { TextButton(onClick = it) { Text("停止") } }
                onDismiss?.let { TextButton(onClick = it) { Text("知道了") } }
            }
        }
    }
}
