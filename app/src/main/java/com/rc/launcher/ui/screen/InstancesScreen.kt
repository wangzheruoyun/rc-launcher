package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.component.InstanceCard
import com.rc.launcher.ui.model.dashboardOrder
import com.rc.launcher.ui.navigation.RcRoutes
import com.rc.launcher.ui.viewmodel.DashboardViewModel
import com.rc.launcher.ui.viewmodel.LaunchState

/**
 * Full game-instances list (task 12 dashboard). Shares the [DashboardViewModel]
 * with the home screen, so launching here is reflected everywhere. Tapping a
 * card body pushes the detail screen via [RcRoutes.instanceDetail] (task 13).
 */
@Composable
fun InstancesScreen(
    navController: NavHostController? = null,
    dashboard: DashboardViewModel = viewModel(),
) {
    val instances by dashboard.instances.collectAsStateWithLifecycle()
    val launchState by dashboard.launchState.collectAsStateWithLifecycle()
    val launchingId = when (val ls = launchState) {
        is LaunchState.Launching -> ls.instanceId
        is LaunchState.Running -> ls.instanceId
        else -> null
    }

    Column(
        modifier = Modifier.fillMaxSize().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("游戏实例", style = MaterialTheme.typography.headlineSmall)
        Text(
            "点击卡片进入详情；右侧播放按钮可一键快速启动。",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        FilledTonalButton(
            onClick = { navController?.navigate(RcRoutes.INSTALL) },
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(Icons.Filled.Add, contentDescription = null)
            Text("安装新实例")
        }
        LazyColumn(
            modifier = Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(10.dp),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(top = 4.dp, bottom = 16.dp),
        ) {
            items(instances.dashboardOrder(), key = { it.id }) { inst ->
                InstanceCard(
                    instance = inst,
                    launching = inst.id == launchingId,
                    onLaunch = { dashboard.launch(inst.id) },
                    onOpen = { navController?.navigate(RcRoutes.instanceDetail(inst.id)) },
                )
            }
        }
    }
}
