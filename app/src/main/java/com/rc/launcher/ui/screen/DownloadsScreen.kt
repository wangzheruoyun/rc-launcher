package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * Download center (version manifests, libraries, assets, mods). The actual
 * transfer is performed by the Rust core's resumable downloader (task 2) and
 * surfaced through the event bus (task 10). Placeholder for the task-11 shell.
 */
@Composable
fun DownloadsScreen() {
    Column(
        modifier = Modifier.fillMaxSize().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
    ) {
        Text("下载中心", style = MaterialTheme.typography.headlineSmall)
        Text(
            "版本清单、依赖库、资源包与 Mod 的下载将在此管理（使用任务 2 的断点续传下载器）。",
            style = MaterialTheme.typography.bodyMedium,
        )
    }
}
