package com.rc.launcher.ui.screen

import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.model.ConflictResolution
import com.rc.launcher.ui.model.ControlLayoutPackage
import com.rc.launcher.ui.model.ControlLayoutSource
import com.rc.launcher.ui.model.ImportConflict
import com.rc.launcher.ui.viewmodel.ControlLayoutLibraryViewModel

/**
 * "Control layout library" screen (task 15).
 *
 * Lets the user:
 *   * Browse the built-in / community catalogue of [ControlLayoutSource]s.
 *   * Fetch any source through the Rust-core download manager (chunked resume +
 *     mirror fallback + checksum verification, task 2) or the pure-JVM
 *     fallback (preview / test).
 *   * Inspect the decoded package, surface any [ImportConflict] with existing
 *     built-in or user-saved layouts, and apply it with a single click (or
 *     rename / skip).
 *   * Paste a custom URL or, in future builds, hook a SAF IntentReceiver to
 *     import a local `.json` / `.zip` file.
 *
 * The state lives entirely in [ControlLayoutLibraryViewModel]; the UI is
 * purely declarative.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun ControlLayoutLibraryScreen(
    onBack: () -> Unit = {},
    onApplied: (ControlLayoutPackage) -> Unit = {},
    viewModel: ControlLayoutLibraryViewModel = viewModel(),
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    var pendingConflicts by remember { mutableStateOf<Pair<ControlLayoutPackage, List<ImportConflict>>?>(null) }
    var pendingImport by remember { mutableStateOf<ControlLayoutPackage?>(null) }

    // When the user clicks "apply" on a downloaded package we defer the
    // conflict prompt until the conflicts are known.
    LaunchedEffect(pendingImport) {
        pendingImport?.let { pkg ->
            val conflicts = com.rc.launcher.ui.model.ControlLayoutImporter.detectConflicts(
                pkg,
                com.rc.launcher.ui.model.ControlLayoutRepositories.default,
            )
            if (conflicts.isEmpty()) {
                viewModel.applyPackage(pkg) { ConflictResolution.RENAME }
                onApplied(pkg)
                pendingImport = null
            } else {
                pendingConflicts = pkg to conflicts
                pendingImport = null
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // ---- Header ---------------------------------------------------------
        Row(verticalAlignment = Alignment.CenterVertically) {
            Button(onClick = onBack) { Text("\u8fd4\u56de") }
            Spacer(Modifier.width(12.dp))
            Text("\u63a7\u5236\u5e03\u5c40\u5e93", style = MaterialTheme.typography.titleLarge)
        }
        Text(
            "\u5728\u7ebf\u793e\u533a\u5e03\u5c40\u4e0e\u672c\u5730\u5bfc\u5165\uff1a\u4e0b\u8f7d\u8d70 DownloadManager \u7684\u955c\u50cf\u56de\u9000\u4e0e\u65ad\u70b9\u7eed\u4f20\uff0c\u51b2\u7a81\u68c0\u6d4b\u540e\u4e00\u952e\u5e94\u7528\u3002",
            style = MaterialTheme.typography.bodyMedium,
        )

        // ---- Error banner ---------------------------------------------------
        state.error?.let { msg ->
            Surface(
                tonalElevation = 1.dp,
                shape = RoundedCornerShape(12.dp),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(
                    "\u26d4 $msg",
                    color = Color(0xFFB00020),
                    modifier = Modifier.padding(12.dp),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }

        // ---- External URL ---------------------------------------------------
        ExternalUrlSection(viewModel = viewModel)

        // ---- Sources --------------------------------------------------------
        Surface(
            tonalElevation = 1.dp,
            shape = RoundedCornerShape(16.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    "\u793e\u533a/\u5185\u7f6e\u5e03\u5c40\u5e93\uff08${state.sources.size}\uff09",
                    style = MaterialTheme.typography.titleMedium,
                )
                if (state.sources.isEmpty()) {
                    Text("\u6682\u65e0\u53ef\u7528\u6765\u6e90", style = MaterialTheme.typography.bodySmall)
                } else {
                    for (src in state.sources) {
                        SourceCard(
                            source = src,
                            progress = state.progressFor(src.id),
                            status = state.statusFor(src.id),
                            onFetch = { viewModel.fetchAndImport(src) },
                            onCancel = { viewModel.cancelFetch(src) },
                            onRemove = { viewModel.removeSource(src.id) },
                        )
                    }
                }
            }
        }

        // ---- Downloaded / decoded packages ---------------------------------
        if (state.imported.isNotEmpty()) {
            Surface(
                tonalElevation = 1.dp,
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text("\u5df2\u89e3\u6790\u7684\u5305", style = MaterialTheme.typography.titleMedium)
                    for (pkg in state.imported) {
                        PackageCard(
                            pkg = pkg,
                            onApply = { pendingImport = pkg },
                        )
                    }
                }
            }
        }
    }

    // ---- Conflict resolution dialog ---------------------------------------
    pendingConflicts?.let { (pkg, conflicts) ->
        ConflictDialog(
            pkg = pkg,
            conflicts = conflicts,
            onResolve = { choices ->
                // The dialog returns a (possibly partial) map of choices; fall
                // back to RENAME for any conflict the user didn't touch.
                viewModel.applyPackage(pkg) { conflict ->
                    choices[conflict] ?: ConflictResolution.RENAME
                }
                onApplied(pkg)
                pendingConflicts = null
            },
            onDismiss = { pendingConflicts = null },
        )
    }
}

/**
 * A single community source card: thumbnail, title, author/version, tags and
 * the download / cancel / remove actions.
 */
@Composable
private fun SourceCard(
    source: ControlLayoutSource,
    progress: Float,
    status: String?,
    onFetch: () -> Unit,
    onCancel: () -> Unit,
    onRemove: () -> Unit,
) {
    val isDownloading = progress in 0f..0.999f
    Surface(
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.4f),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // Thumbnail placeholder.
                Box(
                    modifier = Modifier
                        .height(56.dp)
                        .width(56.dp)
                        .background(
                            color = MaterialTheme.colorScheme.primary.copy(alpha = 0.18f),
                            shape = RoundedCornerShape(8.dp),
                        ),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(source.name.take(1), style = MaterialTheme.typography.titleMedium)
                }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(source.name, style = MaterialTheme.typography.titleMedium)
                    Text(
                        source.authorVersion(),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            if (source.description.isNotBlank()) {
                Text(source.description, style = MaterialTheme.typography.bodySmall)
            }
            if (source.tags.isNotEmpty()) {
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    for (t in source.tags) {
                        Surface(
                            color = MaterialTheme.colorScheme.secondary.copy(alpha = 0.18f),
                            shape = RoundedCornerShape(8.dp),
                        ) {
                            Text(
                                "#t",
                                modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                                style = MaterialTheme.typography.labelSmall,
                            )
                        }
                    }
                }
            }
            if (isDownloading) {
                LinearProgressIndicator(progress = { progress }, modifier = Modifier.fillMaxWidth())
                Text(
                    "\u4e0b\u8f7d\u4e2d\u00b7$status",
                    style = MaterialTheme.typography.bodySmall,
                )
            } else {
                status?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = onFetch, enabled = !isDownloading) { Text("\u4e0b\u8f7d") }
                if (isDownloading) {
                    OutlinedButton(onClick = onCancel) { Text("\u53d6\u6d88") }
                } else {
                    OutlinedButton(onClick = onRemove) { Text("\u79fb\u9664") }
                }
            }
        }
    }
}

/** A decoded-package card with one-click "apply". */
@Composable
private fun PackageCard(
    pkg: ControlLayoutPackage,
    onApply: () -> Unit,
) {
    Surface(
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.4f),
        modifier = Modifier.fillMaxWidth().clickable { onApply() },
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(pkg.name, style = MaterialTheme.typography.titleMedium)
                    Text(
                        pkg.authorVersion(),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Button(onClick = onApply) { Text("\u4e00\u952e\u5e94\u7528") }
            }
            Text(
                "\u63a7\u4ef6\u6570\uff1a${pkg.layout.summary().buttonCount} \u6309\u94ae + ${pkg.layout.summary().joystickCount} \u6447\u6746",
                style = MaterialTheme.typography.bodySmall,
            )
            if (pkg.description.isNotBlank()) {
                Text(pkg.description, style = MaterialTheme.typography.bodySmall)
            }
            Text(
                "\u683c\u5f0f\uff1a${pkg.format}\u00b7\u6e90\uff1a${pkg.downloadUrl ?: "\u672c\u5730\u5bfc\u5165"}",
                style = MaterialTheme.typography.labelSmall,
            )
        }
    }
}

/** A small "add external URL" section (mirrors the FCL/Zalith "URL import" UI). */
@Composable
private fun ExternalUrlSection(viewModel: ControlLayoutLibraryViewModel) {
    var url by remember { mutableStateOf("") }
    Surface(
        tonalElevation = 1.dp,
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("\u4ece\u5916\u90e8 URL \u5bfc\u5165", style = MaterialTheme.typography.titleMedium)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = url,
                    onValueChange = { url = it },
                    label = { Text("https://\u2026") },
                    singleLine = true,
                    modifier = Modifier.weight(1f),
                )
                Button(
                    onClick = {
                        viewModel.addExternalUrl(url)
                        url = ""
                    },
                    enabled = url.isNotBlank(),
                ) { Text("\u6dfb\u52a0") }
            }
            Text(
                "\u53ef\u4ece\u5916\u90e8\u590d\u5236\u4e00\u4e2a\u5e03\u5c40\u6587\u4ef6\u7684\u76f4\u94fe\uff0c\u6216\u4ece\u672c\u5730\u6587\u4ef6\u7ba1\u7406\u5668\u9009\u62e9 .json / .zip \u5bfc\u5165\u3002",
                style = MaterialTheme.typography.bodySmall,
            )
        }
    }
}

/**
 * Dialog shown when an imported package collides with an existing layout.
 * The user picks a [ConflictResolution] per conflict (overwrite / rename /
 * skip); the resolver is invoked once with all selections when they tap
 * "Apply" or "Skip all".
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ConflictDialog(
    pkg: ControlLayoutPackage,
    conflicts: List<ImportConflict>,
    onResolve: (Map<ImportConflict, ConflictResolution>) -> Unit,
    onDismiss: () -> Unit,
) {
    var choices = remember { mutableStateOf(conflicts.associateWith { ConflictResolution.RENAME }) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("\u51b2\u7a81\u68c0\u6d4b") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    "\u201c${pkg.name}\u201d \u4e0e\u73b0\u6709\u5e03\u5c40\u5b58\u5728\u51b2\u7a81，\u9009\u62e9\u89e3\u51b3\u65b9\u5f0f：",
                    style = MaterialTheme.typography.bodyMedium,
                )
                for ((idx, c) in conflicts.withIndex()) {
                    Surface(
                        shape = RoundedCornerShape(12.dp),
                        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.4f),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text(
                                if (c.kind == ImportConflict.Kind.ID)
                                    "ID \u91cd\u590d\uff1a${c.existingId}"
                                else
                                    "\u540d\u79f0\u91cd\u590d\uff1a${c.existingName}",
                                style = MaterialTheme.typography.titleSmall,
                            )
                            Text(
                                "\u73b0\u6709\uff1a${c.existingName}\u00b7\u5373\u5c06\u5bfc\u5165\uff1a${c.incomingName}",
                                style = MaterialTheme.typography.bodySmall,
                            )
                            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                val sel = choices.value[c] ?: ConflictResolution.RENAME
                                for (r in listOf(ConflictResolution.OVERWRITE, ConflictResolution.RENAME, ConflictResolution.SKIP)) {
                                    OutlinedButton(
                                        onClick = {
                                            choices.value = choices.value.toMutableMap().apply { put(c, r) }
                                        },
                                    ) {
                                        Text(
                                            when (r) {
                                                ConflictResolution.OVERWRITE -> if (sel == r) "\u2713 \u8986\u76d6" else "\u8986\u76d6"
                                                ConflictResolution.RENAME -> if (sel == r) "\u2713 \u91cd\u540d" else "\u91cd\u540d"
                                                ConflictResolution.SKIP -> if (sel == r) "\u2713 \u8df3\u8fc7" else "\u8df3\u8fc7"
                                            },
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            Button(onClick = { onResolve(choices.value); onDismiss() }) { Text("\u5e94\u7528") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("\u53d6\u6d88") }
        },
    )
}
