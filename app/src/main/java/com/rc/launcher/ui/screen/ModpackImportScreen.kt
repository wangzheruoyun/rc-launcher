package com.rc.launcher.ui.screen

import android.app.Activity
import android.content.Intent
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Cloud
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Snackbar
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.ProvideRcWindowInfo
import com.rc.launcher.ui.model.GameDirectoryType
import com.rc.launcher.ui.model.ModLoader
import com.rc.launcher.ui.navigation.InstanceDetailRoute
import com.rc.launcher.ui.navigation.InstallRoute
import com.rc.launcher.ui.rcWindowInfo
import com.rc.launcher.ui.viewmodel.ModpackImportViewModel
import androidx.navigation.NavHostController
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File

/**
 * Modpack import screen (task 17).
 *
 * The user can:
 *   * paste a remote `.mrpack` / `manifest.json` URL,
 *   * pick a local archive from the system file picker,
 *   * paste a manifest text directly,
 *
 * ...and the screen runs the Rust core's modpack import pipeline (parse
 * \u2192 download \u2192 verify \u2192 extract overrides \u2192 register instance). Progress
 * and per-file outcomes are surfaced through the event bus + the report
 * published at the end of the run.
 *
 * Adaptive (task 9): the form column caps its width on a wide tablet; the
 * `rcWindowInfo()` size class drives padding + column count so the same
 * code reads well on a phone in portrait or landscape.
 */
@Composable
fun ModpackImportScreen(
    navController: NavHostController? = null,
    vm: ModpackImportViewModel = viewModel(),
) {
    val state by vm.state.collectAsState()
    val snackbar = remember { SnackbarHostState() }
    val coroutineScope = rememberCoroutineScope()
    val window = rcWindowInfo()
    val pad = window.contentPaddingDp.dp
    val context = LocalContext.current

    val pickArchive = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.GetContent(),
    ) { uri: Uri? ->
        if (uri != null) {
            coroutineScope.launch {
                val bytes = withContext(Dispatchers.IO) {
                    context.contentResolver.openInputStream(uri)?.use { it.readBytes() }
                }
                if (bytes == null) {
                    snackbar.showSnackbar("\u65e0\u6cd5\u8bfb\u53d6\u6587\u4ef6")
                } else {
                    vm.importLocalArchive(bytes, uri.toString())
                }
            }
        }
    }

    LaunchedEffect(state.errorMessage) {
        state.errorMessage?.let {
            snackbar.showSnackbar(it)
            vm.consumeError()
        }
    }

    Column(
        modifier = Modifier.fillMaxSize().padding(pad),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(modifier = Modifier.widthIn(max = window.maxContentWidthDp.dp)) {
            Text(
                "\u6574\u5408\u5305\u5bfc\u5165",
                style = MaterialTheme.typography.headlineSmall,
            )
            if (!window.isShort) {
                Text(
                    "\u652f\u6301 Modrinth\u3001CurseForge \u4e0e MultiMC \u683c\u5f0f\uff1b\u53ef\u4ece\u8fdc\u7a0b URL\u3001\u672c\u5730\u538b\u7f29\u5305\u6216\u7c98\u8d34 manifest \u6587\u672c\u5bfc\u5165\u3002",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        ) {
            Column(
                modifier = Modifier.padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(
                    "\u5bfc\u5165\u6765\u6e90",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )

                OutlinedTextField(
                    value = state.urlInput,
                    onValueChange = vm::setUrlInput,
                    label = { Text("\u8fdc\u7a0b\u5730\u5740 (.mrpack / manifest.json)") },
                    placeholder = { Text("https://cdn.modrinth.com/.../pack.mrpack") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )

                Row(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    FilledTonalButton(
                        onClick = { vm.importFromUrl() },
                        enabled = !state.busy && state.urlInput.isNotBlank(),
                    ) {
                        Icon(Icons.Filled.Cloud, contentDescription = null)
                        Spacer(Modifier.size(8.dp))
                        Text("\u4ece URL \u5bfc\u5165")
                    }
                    Button(
                        onClick = { pickArchive.launch("*/*") },
                        enabled = !state.busy,
                    ) {
                        Icon(Icons.Filled.Folder, contentDescription = null)
                        Spacer(Modifier.size(8.dp))
                        Text("\u9009\u62e9\u672c\u5730 .zip / .mrpack")
                    }
                }

                HorizontalDivider(modifier = Modifier.padding(vertical = 4.dp))

                OutlinedTextField(
                    value = state.manifestTextInput,
                    onValueChange = vm::setManifestTextInput,
                    label = { Text("\u6216\u8005\u7c98\u8d34 manifest \u6587\u672c") },
                    placeholder = { Text("{\"formatVersion\":1,...}") },
                    modifier = Modifier.fillMaxWidth().height(120.dp),
                )

                TextButton(
                    onClick = { vm.importFromText() },
                    enabled = !state.busy && state.manifestTextInput.isNotBlank(),
                ) {
                    Icon(Icons.Filled.Add, contentDescription = null)
                    Spacer(Modifier.size(8.dp))
                    Text("\u89e3\u6790\u5e76\u5bfc\u5165\u6587\u672c")
                }
            }
        }

        if (state.parsedManifest != null) {
            ParsedManifestSummary(
                manifest = state.parsedManifest!!,
                instanceId = state.instanceId,
                onInstanceIdChange = vm::setInstanceId,
                allowOverwrite = state.allowOverwrite,
                onAllowOverwriteChange = vm::setAllowOverwrite,
                gameDirectoryType = state.gameDirectoryType,
                onGameDirectoryTypeChange = vm::setGameDirectoryType,
                onConfirm = { vm.startImport() },
                busy = state.busy,
            )
        }

        if (state.busy) {
            Card(modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(state.progressLabel.ifEmpty { "\u51c6\u5907\u4e2d..." })
                    Spacer(Modifier.size(8.dp))
                    if (state.progressFraction > 0f) {
                        LinearProgressIndicator(
                            progress = state.progressFraction.coerceIn(0f, 1f),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    } else {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    }
                }
            }
        }

        state.report?.let { report ->
            ImportReportCard(report = report, navController = navController)
        }

        Spacer(modifier = Modifier.size(8.dp))
        SnackbarHost(snackbar)
    }
}

@Composable
private fun ParsedManifestSummary(
    manifest: JSONObject,
    instanceId: String,
    onInstanceIdChange: (String) -> Unit,
    allowOverwrite: Boolean,
    onAllowOverwriteChange: (Boolean) -> Unit,
    gameDirectoryType: GameDirectoryType,
    onGameDirectoryTypeChange: (GameDirectoryType) -> Unit,
    onConfirm: () -> Unit,
    busy: Boolean,
) {
    val name = manifest.optJSONObject("spec")?.optString("name") ?: "(unknown)"
    val mcVersion = manifest.optJSONObject("spec")?.optString("mc_version") ?: "?"
    val loader = manifest.optJSONObject("spec")?.optString("loader") ?: "vanilla"
    val loaderVersion = manifest.optJSONObject("spec")?.optString("loader_version", null)
    val fileCount = manifest.optJSONObject("spec")?.optJSONArray("files")?.length() ?: 0
    val flavour = manifest.optString("flavour")

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer),
    ) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(
                name,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AssistChip(
                    onClick = {},
                    label = { Text("\u683c\u5f0f: $flavour") },
                )
                AssistChip(
                    onClick = {},
                    label = { Text("MC $mcVersion") },
                )
                AssistChip(
                    onClick = {},
                    label = {
                        Text(
                            if (loaderVersion != null) "$loader $loaderVersion"
                            else loader,
                        )
                    },
                )
                AssistChip(
                    onClick = {},
                    label = { Text("$fileCount \u6587\u4ef6") },
                )
            }

            OutlinedTextField(
                value = instanceId,
                onValueChange = onInstanceIdChange,
                label = { Text("\u5b9e\u4f8b\u76ee\u5f55\u540d\uff08\u5fc5\u987b\u4e3a\u5b89\u5168\u8def\u5f84\uff09") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )

            Row(verticalAlignment = Alignment.CenterVertically) {
                Switch(checked = allowOverwrite, onCheckedChange = onAllowOverwriteChange)
                Spacer(Modifier.size(8.dp))
                Text("\u8986\u76d6\u540c\u540d\u5df2\u6709\u5b9e\u4f8b\u76ee\u5f55")
            }

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                GameDirectoryType.entries.forEach { type ->
                    AssistChip(
                        onClick = { onGameDirectoryTypeChange(type) },
                        label = { Text(type.label) },
                        colors = if (type == gameDirectoryType)
                            AssistChipDefaults.assistChipColors(
                                containerColor = MaterialTheme.colorScheme.primaryContainer,
                            )
                        else AssistChipDefaults.assistChipColors(),
                    )
                }
            }

            Button(
                onClick = onConfirm,
                enabled = !busy && instanceId.isNotBlank(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                if (busy) {
                    CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.size(8.dp))
                }
                Text("\u5f00\u59cb\u5bfc\u5165")
            }
        }
    }
}

@Composable
private fun ImportReportCard(report: JSONObject, navController: NavHostController?) {
    val name = report.optString("name", "(unknown)")
    val summary = report.optString("summary", "")
    val downloaded = report.optInt("downloaded_count", 0)
    val already = report.optInt("already_present_count", 0)
    val unsafe = report.optInt("unsafe_path_count", 0)
    val hashMismatch = report.optInt("hash_mismatch_count", 0)
    val failed = report.optInt("failed_count", 0)
    val completed = report.optBoolean("completed", false)
    val instanceRoot = report.optString("instance_root", "")
    val files = report.optJSONArray("files") ?: org.json.JSONArray()
    val isClean = unsafe == 0 && hashMismatch == 0 && failed == 0

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = if (isClean) MaterialTheme.colorScheme.primaryContainer
            else MaterialTheme.colorScheme.errorContainer,
        ),
    ) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    if (isClean) Icons.Filled.CheckCircle else Icons.Filled.Error,
                    contentDescription = null,
                    tint = if (isClean) MaterialTheme.colorScheme.primary
                    else MaterialTheme.colorScheme.error,
                )
                Spacer(Modifier.size(8.dp))
                Text(
                    if (isClean) "\u5bfc\u5165\u6210\u529f" else "\u5bfc\u5165\u5b8c\u6210\uff0c\u6709\u95ee\u9898",
                    style = MaterialTheme.typography.titleMedium,
                )
            }
            Text("$name", fontWeight = FontWeight.SemiBold)
            if (summary.isNotEmpty()) Text(summary, style = MaterialTheme.typography.bodySmall)
            Text("\u5b9e\u4f8b\u4f4d\u7f6e: $instanceRoot", style = MaterialTheme.typography.bodySmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                AssistChip(onClick = {}, label = { Text("\u4e0b\u8f7d: $downloaded") })
                AssistChip(onClick = {}, label = { Text("\u5df2\u5b58\u5728: $already") })
                if (unsafe > 0) AssistChip(onClick = {}, label = { Text("\u4e0d\u5b89\u5168\u8def\u5f84: $unsafe") })
                if (hashMismatch > 0) AssistChip(onClick = {}, label = { Text("\u54c8\u5e0c\u4e0d\u5339\u914d: $hashMismatch") })
                if (failed > 0) AssistChip(onClick = {}, label = { Text("\u5931\u8d25: $failed") })
            }
            Text(
                "pipeline completed: $completed",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            if (files.length() > 0) {
                HorizontalDivider()
                Text("\u8be6\u60c5 (\u524d ${files.length().coerceAtMost(20)} \u6761)", fontWeight = FontWeight.SemiBold)
                LazyColumn(
                    modifier = Modifier.fillMaxWidth().height(180.dp),
                    contentPadding = PaddingValues(vertical = 4.dp),
                ) {
                    items((0 until files.length().coerceAtMost(20)).map { files.getJSONObject(it) }) { f ->
                        val path = f.optJSONObject("file")?.optString("path") ?: "?"
                        val outcome = f.optString("outcome", "")
                        Row(
                            modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Icon(
                                when (outcome) {
                                    "downloaded", "alreadypresent" -> Icons.Filled.CheckCircle
                                    "unsafepath", "hashmismatch" -> Icons.Filled.Error
                                    else -> Icons.Filled.Refresh
                                },
                                contentDescription = null,
                                tint = when (outcome) {
                                    "downloaded", "alreadypresent" -> MaterialTheme.colorScheme.primary
                                    "unsafepath", "hashmismatch" -> MaterialTheme.colorScheme.error
                                    else -> MaterialTheme.colorScheme.onSurfaceVariant
                                },
                                modifier = Modifier.size(16.dp),
                            )
                            Spacer(Modifier.size(6.dp))
                            Text(
                                path,
                                style = MaterialTheme.typography.bodySmall,
                                fontFamily = FontFamily.Monospace,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f),
                            )
                            Text(outcome, style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
            }

            FilledTonalButton(
                onClick = {
                    val id = report.optString("name", "imported")
                    navController?.navigate(InstanceDetailRoute(id))
                },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("\u67e5\u770b\u5b9e\u4f8b\u8be6\u60c5")
            }
        }
    }
}

private fun <T> List<T>.toList() = this
