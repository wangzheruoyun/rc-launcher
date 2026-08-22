package com.rc.launcher.ui.component

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.rc.launcher.ui.resource.ResourceUsage

/**
 * Floating frame-rate / performance HUD (task 12 "悬浮帧率 HUD"), modelled after
 * MCTier's `GameHudOverlay`. Rendered as a z-stacked overlay above the dashboard
 * so it never pushes content around; [onClose] hides it.
 */
@Composable
fun FloatingHud(
    fps: Int,
    usage: ResourceUsage,
    modifier: Modifier = Modifier,
    onClose: (() -> Unit)? = null,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.92f),
        shadowElevation = 6.dp,
        tonalElevation = 2.dp,
    ) {
        Column(
            modifier = Modifier.width(180.dp).padding(10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("性能 HUD", style = MaterialTheme.typography.labelMedium)
                Spacer(Modifier.weight(1f))
                onClose?.let {
                    IconButton(onClick = it, modifier = Modifier.size(20.dp)) {
                        Icon(
                            imageVector = Icons.Filled.Close,
                            contentDescription = "关闭 HUD",
                            modifier = Modifier.size(14.dp),
                        )
                    }
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                HudMetric("FPS", fps.toString())
                HudMetric("CPU", "%.0f%%".format(usage.cpuPercent))
                HudMetric("内存", "%.0f%%".format(usage.memPercent))
            }
            Text(
                "存储占用 %.0f%%".format(usage.storagePercent),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun HudMetric(label: String, value: String) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(
            text = value,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.primary,
        )
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
