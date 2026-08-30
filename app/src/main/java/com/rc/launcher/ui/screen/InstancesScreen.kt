package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.ProvideRcWindowInfo
import com.rc.launcher.ui.component.InstanceCard
import com.rc.launcher.ui.model.dashboardOrder
import com.rc.launcher.ui.navigation.InstallRoute
import com.rc.launcher.ui.navigation.InstanceDetailRoute
import com.rc.launcher.ui.rcWindowInfo
import com.rc.launcher.ui.viewmodel.DashboardViewModel
import com.rc.launcher.ui.viewmodel.LaunchState

/**
 * Full game-instances list (task 12 dashboard). Shares the [DashboardViewModel]
 * with the home screen, so launching here is reflected everywhere. Tapping a
 * card body pushes the detail screen via [InstanceDetailRoute] (task 13).
 *
 * **Adaptive (task 9).** The cards live in a [LazyVerticalGrid] whose column
 * count comes from the measured window ([rcWindowInfo]): one column on a compact
 * portrait phone, two once the phone is rotated (or on a small tablet) and three
 * on an expanded window. Rotating therefore re-flows the list instead of
 * stretching a single column across the whole width, and the header shrinks its
 * padding to the size class.
 */
@Composable
fun InstancesScreen(
    navController: NavHostController? = null,
    dashboard: DashboardViewModel = viewModel(),
) {
    val instances by dashboard.instances.collectAsStateWithLifecycle()
    val launchState by dashboard.launchState.collectAsStateWithLifecycle()
    val launchStateVal = launchState
    val launchingId = when (launchStateVal) {
        is LaunchState.Launching -> launchStateVal.instanceId
        is LaunchState.Running -> launchStateVal.instanceId
        else -> null
    }
    val window = rcWindowInfo()
    val pad = window.contentPaddingDp.dp

    Column(
        modifier = Modifier.fillMaxSize().padding(pad),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(
            // Keep the copy readable on a wide tablet instead of stretching it.
            modifier = Modifier.widthIn(max = window.maxContentWidthDp.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("游戏实例", style = MaterialTheme.typography.headlineSmall)
            // A short landscape window spends its little height on cards, not prose.
            if (!window.isShort) {
                Text(
                    "点击卡片进入详情；右侧播放按钮可一键快速启动。",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            FilledTonalButton(
                onClick = { navController?.navigate(InstallRoute) },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Filled.Add, contentDescription = null)
                Text("安装新实例")
            }
        }
        LazyVerticalGrid(
            columns = GridCells.Fixed(window.instanceColumns),
            modifier = Modifier.fillMaxWidth(),
            verticalArrangement = Arrangement.spacedBy(10.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
            contentPadding = PaddingValues(top = 4.dp, bottom = 16.dp),
        ) {
            items(instances.dashboardOrder(), key = { it.id }) { inst ->
                InstanceCard(
                    instance = inst,
                    launching = inst.id == launchingId,
                    onLaunch = { dashboard.launch(inst.id) },
                    onOpen = { navController?.navigate(InstanceDetailRoute(inst.id)) },
                )
            }
        }
    }
}

@Preview(name = "Instances portrait", showBackground = true, widthDp = 392, heightDp = 872)
@Composable
private fun InstancesScreenPortraitPreview() {
    ProvideRcWindowInfo { InstancesScreen() }
}

/** Task 9: the same list re-flowed for a landscape window. */
@Preview(name = "Instances landscape", showBackground = true, widthDp = 872, heightDp = 392)
@Composable
private fun InstancesScreenLandscapePreview() {
    ProvideRcWindowInfo { InstancesScreen() }
}
