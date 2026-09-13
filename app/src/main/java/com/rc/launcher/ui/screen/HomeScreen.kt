package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
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
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.ProvideRcWindowInfo
import com.rc.launcher.ui.component.FloatingHud
import com.rc.launcher.ui.component.FloatingHudAction
import com.rc.launcher.ui.component.FloatingHudConfig
import com.rc.launcher.ui.component.GameFloatingHud
import com.rc.launcher.ui.component.InputMode
import com.rc.launcher.ui.component.InstanceCard
import com.rc.launcher.ui.component.ResourceSummary
import com.rc.launcher.ui.model.recentlyPlayed
import com.rc.launcher.ui.model.dashboardOrder
import com.rc.launcher.ui.navigation.InstanceDetailRoute
import com.rc.launcher.ui.rcWindowInfo
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
 *
 * **Adaptive (task 9).** Every dimension is derived from the measured window
 * ([rcWindowInfo]): the greeting and the resource panel sit side by side once the
 * window is wide enough, the instance grid re-flows into
 * [com.rc.launcher.ui.RcWindowInfo.instanceColumns] columns, and a short
 * landscape window drops the "最近游玩" rail so the little height that is left
 * goes to the actual cards. The instances are chunked into [Row]s (instead of a
 * nested lazy grid) because the whole screen already lives in a vertical
 * scroller.
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

    // Task 20: collect HUD state outside conditionals (Compose rule).
    val hudConfig by dashboard.hudConfig.collectAsStateWithLifecycle()
    val hudInputMode by dashboard.inputMode.collectAsStateWithLifecycle()
    val hudLogLines by dashboard.logLines.collectAsStateWithLifecycle()
    val hudCrashSnapshot by dashboard.crashSnapshot.collectAsStateWithLifecycle()
    val hudExportState by dashboard.exportState.collectAsStateWithLifecycle()

    val launchStateVal = launchState
    val launchingId = when (launchStateVal) {
        is LaunchState.Launching -> launchStateVal.instanceId
        is LaunchState.Running -> launchStateVal.instanceId
        else -> null
    }

    val openInstance: (id: String) -> Unit = { id ->
        navController?.navigate(InstanceDetailRoute(id))
    }

    // Task 9: one measurement drives the padding, the columns and which
    // sections are worth their vertical space in the current orientation.
    val window = rcWindowInfo()
    val columns = window.instanceColumns
    val twoPane = window.dashboardColumns > 1

    Box(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(window.contentPaddingDp.dp),
            verticalArrangement = Arrangement.Vertical.spacedBy(16.dp),
        ) {
            // Header (+ the resource panel beside it once the window is wide).
            val header: @Composable () -> Unit = {
                Row(
                    verticalAlignment = Alignment.Vertical.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.Vertical.spacedBy(2.dp)) {
                        Text("主页", style = MaterialTheme.typography.headlineSmall)
                        val coreStateVal = coreState
                        Text(
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
            }
            if (twoPane) {
                // Landscape / tablet: greeting and live usage share one band, which
                // keeps the cards above the fold on a short window.
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                    verticalAlignment = Alignment.Vertical.CenterVertically,
                ) {
                    Box(modifier = Modifier.weight(1f)) { header() }
                    Box(modifier = Modifier.weight(1f)) { ResourceSummary(usage) }
                }
            } else {
                header()
                ResourceSummary(usage)
            }

            // The "recently played" rail is the first thing to go when the window
            // is short (a landscape phone): the full grid below already has them.
            if (recent.isNotEmpty() && !window.isShort) {
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
                            onToggleFavorite = { dashboard.toggleFavorite(inst.id) },
                            modifier = Modifier.fillMaxWidth().widthIn(min = 280.dp),
                        )
                    }
                }
            }

            SectionTitle("游戏实例 (${instances.size})")
            Column(verticalArrangement = Arrangement.Vertical.spacedBy(10.dp)) {
                // Chunked rows rather than a nested lazy grid: this screen is
                // already inside a vertical scroller, so a lazy grid here would be
                // measured with an infinite height.
                for (row in instances.dashboardOrder().chunked(columns)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        for (inst in row) {
                            InstanceCard(
                                instance = inst,
                                launching = inst.id == launchingId,
                                onLaunch = { dashboard.launch(inst.id) },
                                onOpen = { openInstance(inst.id) },
                                onToggleFavorite = { dashboard.toggleFavorite(inst.id) },
                                modifier = Modifier.weight(1f),
                            )
                        }
                        // Keep the cards of a ragged last row at their column width
                        // instead of letting one card span the whole row.
                        repeat(columns - row.size) {
                            Spacer(modifier = Modifier.weight(1f))
                        }
                    }
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

        // Floating frame-rate / menu HUD (task 20).
        // When the game is running we show the full GameFloatingHud with menu
        // actions (log / input mode / screenshot / force-quit); otherwise we
        // fall back to the compact dashboard badge.
        if (showHud) {

            if (launchState is LaunchState.Running) {
                GameFloatingHud(
                    fps = fps,
                    usage = usage,
                    inputMode = hudInputMode,
                    onInputModeChange = { dashboard.toggleInputMode() },
                    config = hudConfig,
                    onConfigChange = { dashboard.setHudConfig(it) },
                    logLines = hudLogLines,
                    crashSnapshot = hudCrashSnapshot,
                    exportState = hudExportState,
                    onExport = { dashboard.setExportInProgress(true) },
                    onAction = { action ->
                        when (action) {
                            is FloatingHudAction.OpenLog -> { /* opens log overlay — handled by game surface */ }
                            is FloatingHudAction.SwitchInputMode -> dashboard.toggleInputMode()
                            is FloatingHudAction.Screenshot -> { /* captures frame — handled by game surface */ }
                            is FloatingHudAction.ExportLog -> dashboard.setExportInProgress(true)
                            is FloatingHudAction.TakeSnapshot -> dashboard.takeCrashSnapshot(
                                exitCode = null,
                                signal = null,
                                categoryId = null,
                                summary = "Game is running (snapshot taken)",
                                advice = "",
                                logTail = hudLogLines.lastOrNull()?.let { listOf(it.text) } ?: emptyList(),
                            )
                            is FloatingHudAction.ClearLog -> dashboard.clearLog()
                            is FloatingHudAction.ForceQuit -> dashboard.forceQuit()
                        }
                    },
                    onClose = {
                        if (launchState is LaunchState.Running) dashboard.forceQuit()
                        else dashboard.toggleHud()
                    },
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(top = 8.dp, end = 16.dp),
                )
            } else {
                FloatingHud(
                    fps = fps,
                    usage = usage,
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(top = 8.dp, end = 16.dp),
                    onClose = { dashboard.toggleHud() },
                )
            }
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
                verticalAlignment = Alignment.Vertical.CenterVertically,
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

@Preview(name = "Home portrait", showBackground = true, widthDp = 392, heightDp = 872)
@Composable
private fun HomeScreenPortraitPreview() {
    ProvideRcWindowInfo { HomeScreen() }
}

/** Task 9: the landscape dashboard (two-pane header, re-flowed grid). */
@Preview(name = "Home landscape", showBackground = true, widthDp = 872, heightDp = 392)
@Composable
private fun HomeScreenLandscapePreview() {
    ProvideRcWindowInfo { HomeScreen() }
}
