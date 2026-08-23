package com.rc.launcher.ui.component

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.rc.launcher.ui.resource.ResourceUsage
import com.rc.launcher.ui.resource.formatBytes
import androidx.compose.foundation.layout.padding

/**
 * Compact "资源占用" panel (task 12): memory / storage / CPU with progress bars.
 * Falls back to a single "不可用" hint when the platform provides no readings.
 */
@Composable
fun ResourceSummary(usage: ResourceUsage, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(12.dp),
        tonalElevation = 1.dp,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(
                    imageVector = Icons.Filled.Memory,
                    contentDescription = null,
                    modifier = Modifier.size(18.dp),
                    tint = MaterialTheme.colorScheme.primary,
                )
                Text("资源占用", style = MaterialTheme.typography.titleSmall)
                if (usage.isUnknown) {
                    Text(
                        "（设备信息不可用）",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            UsageRow("内存", usage.memPercent, usage.usedMemBytes, usage.totalMemBytes)
            UsageRow("存储", usage.storagePercent, usage.usedStorageBytes, usage.totalStorageBytes)
            UsageRow("CPU", usage.cpuPercent.toFloat(), null, null)
        }
    }
}

@Composable
private fun UsageRow(label: String, percent: Float, used: Long?, total: Long?) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(label, style = MaterialTheme.typography.labelMedium)
            val detail = if (used != null && total != null) {
                "%.0f%% · %s / %s".format(percent, formatBytes(used), formatBytes(total))
            } else {
                "%.0f%%".format(percent)
            }
            Text(
                detail,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        LinearProgressIndicator(
            progress = { (percent / 100f).coerceIn(0f, 1f) },
            modifier = Modifier.fillMaxWidth(),
        )
    }
}
