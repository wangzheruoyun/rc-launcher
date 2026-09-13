package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
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
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.viewmodel.DownloadEntry
import com.rc.launcher.ui.viewmodel.DownloadsViewModel

/**
 * Downloads screen (task 30).
 *
 * Shows the live progress of every active download job reported through the
 * Rust-core event bus. Each row displays:
 * * label / scope,
 * * downloaded vs total bytes and fraction,
 * * current download speed (`speed_bps`, formatted human-readable),
 * * lifecycle status (`running` / `paused` / `completed` / `failed`),
 * * a linear progress bar and a **Pause / Cancel** button.
 *
 * The actual transfer is performed by the Rust core's resumable downloader
 * (task 2) — this screen is purely the UI shell that subscribes to progress
 * events and issues pause / cancel commands back through `cancelAsync`.
 */
@Composable
fun DownloadsScreen(
    modifier: Modifier = Modifier,
    viewModel: DownloadsViewModel = viewModel(),
) {
    val entries by viewModel.entries.collectAsState()

    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(24.dp),
    ) {
        Text(
            text = "下载中心",
            style = MaterialTheme.typography.headlineSmall,
            fontWeight = FontWeight.Bold,
        )
        Text(
            text = "版本清单、依赖库、资源包与 Mod 的下载将在此管理（使用任务 2 的断点续传下载器）。",
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(top = 8.dp),
        )
        Spacer(modifier = Modifier.height(12.dp))

        if (entries.isEmpty()) {
            Text(
                text = "暂无活动的下载任务",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 16.dp),
            )
        } else {
            LazyColumn(
                verticalArrangement = Arrangement.spacedBy(8.dp),
                contentPadding = PaddingValues(vertical = 4.dp),
            ) {
                items(entries, key = { it.scope }) { entry ->
                    DownloadEntryCard(entry = entry, onPause = {
                        viewModel.pauseDownload(it.scope)
                    })
                }
            }
        }
    }
}

/**
 * A single download progress row with speed, status, and a pause button (task 30).
 */
@Composable
private fun DownloadEntryCard(
    entry: DownloadEntry,
    onPause: (DownloadEntry) -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        elevation = CardDefaults.cardElevation(defaultElevation = 2.dp),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = entry.label,
                        style = MaterialTheme.typography.bodyLarge,
                        fontWeight = FontWeight.Medium,
                    )
                    Text(
                        text = entry.scope,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                // Status badge
                val statusColor = when (entry.status) {
                    "running" -> Color(0xFF4CAF50)
                    "paused" -> Color(0xFFFF9800)
                    "completed" -> Color(0xFF2196F3)
                    "failed" -> Color(0xFFF44336)
                    else -> MaterialTheme.colorScheme.onSurfaceVariant
                }
                Text(
                    text = entry.status,
                    color = statusColor,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold,
                )
            }

            Spacer(modifier = Modifier.height(8.dp))

            // Progress bar
            LinearProgressIndicator(
                progress = { entry.fraction },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(6.dp),
                color = when (entry.status) {
                    "running" -> MaterialTheme.colorScheme.primary
                    "paused" -> androidx.compose.ui.graphics.Color(0xFFFF9800)
                    "completed" -> androidx.compose.ui.graphics.Color(0xFF4CAF50)
                    "failed" -> androidx.compose.ui.graphics.Color(0xFFF44336)
                    else -> MaterialTheme.colorScheme.primary
                },
            )

            Spacer(modifier = Modifier.height(8.dp))

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // Speed + bytes
                Column {
                    Text(
                        text = formatSpeed(entry.speedBps),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        text = formatBytes(entry.downloaded) + " / " + (entry.total?.let { formatBytes(it) } ?: "?"),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }

                // Pause / Cancel button
                if (!entry.isFinished) {
                    TextButton(onClick = { onPause(entry) }) {
                        Text("暂停")
                    }
                } else if (entry.status == "failed") {
                    TextButton(onClick = { onPause(entry) }) {
                        Text("重试")
                    }
                }
            }

            entry.errorMessage?.let { msg ->
                Text(
                    text = msg,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(top = 4.dp),
                )
            }
        }
    }
}

/** Format bytes to a human-readable string (e.g. "1.5 MB"). */
private fun formatBytes(bytes: Long): String {
    val mb = bytes.toFloat() / (1024 * 1024)
    val kb = bytes.toFloat() / 1024
    return when {
        mb >= 1f -> String.format("%.1f MB", mb)
        kb >= 1f -> String.format("%.0f KB", kb)
        else -> String.format("%d B", bytes)
    }
}

/** Format bytes-per-second to a human-readable string. */
private fun formatSpeed(bps: Long): String {
    val mbps = bps.toFloat() / (1024 * 1024)
    val kbps = bps.toFloat() / 1024
    return when {
        bps <= 0 -> " — "
        mbps >= 1f -> String.format("%.1f MB/s", mbps)
        kbps >= 1f -> String.format("%.0f KB/s", kbps)
        else -> String.format("%d B/s", bps)
    }
}
