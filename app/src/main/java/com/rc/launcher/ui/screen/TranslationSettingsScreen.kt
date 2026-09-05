package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString
import com.rc.launcher.ui.translate.TranslationViewModel
import com.rc.launcher.ui.rcWindowInfo

/**
 * Translation settings (task 13).
 *
 * Reached from the settings list. Surfaces:
 *  * an enable / disable switch,
 *  * the target-language picker (auto / zh-CN / zh-Hant / en),
 *  * the translation mode picker (hybrid / online / offline),
 *  * a "show original" toggle,
 *  * the on-disk cache stats (entry count, size, limits) with a
 *    "clear cache" button.
 *
 * All work happens in the Rust core; this screen only renders state
 * and forwards intent.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TranslationSettingsScreen(
    navController: NavHostController? = null,
    viewModel: TranslationViewModel = viewModel(),
) {
    val context = LocalContext.current
    LaunchedEffect(Unit) { viewModel.ensureInitialised(context) }
    val state by viewModel.state.collectAsState()
    val window = rcWindowInfo()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(window.contentPaddingDp.dp)
            .widthIn(max = window.maxContentWidthDp.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // Header (with optional back button)
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (navController != null) {
                IconButton(onClick = { navController.popBackStack() }) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = rcString(RcStringKeys.COMMON_BACK))
                }
            }
            Column(Modifier.weight(1f)) {
                Text(
                    rcString(RcStringKeys.TRANSLATE_TITLE),
                    style = MaterialTheme.typography.headlineSmall,
                )
                Text(
                    rcString(RcStringKeys.TRANSLATE_SUBTITLE),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }

        // Enable toggle
        SettingSection(rcString(RcStringKeys.TRANSLATE_ENABLE)) {
            SwitchSetting(
                label = rcString(RcStringKeys.TRANSLATE_ENABLE),
                checked = state.prefs.enabled,
                onChange = viewModel::setEnabled,
            )
            Text(
                rcString(RcStringKeys.TRANSLATE_NETWORK_FALLBACK_SUMMARY),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        if (state.prefs.enabled) {
            // Language picker
            SettingSection(rcString(RcStringKeys.TRANSLATE_LANGUAGE)) {
                LanguagePicker(state.prefs.target, viewModel::setTarget)
            }

            // Mode picker
            SettingSection(rcString(RcStringKeys.TRANSLATE_MODE)) {
                ModePicker(state.prefs.mode, viewModel::setMode)
            }

            // Show original toggle
            SettingSection(rcString(RcStringKeys.TRANSLATE_SHOW_ORIGINAL)) {
                SwitchSetting(
                    label = if (state.prefs.showOriginal)
                        rcString(RcStringKeys.TRANSLATE_SHOW_ORIGINAL)
                    else
                        rcString(RcStringKeys.TRANSLATE_SHOW_TRANSLATED),
                    checked = state.prefs.showOriginal,
                    onChange = viewModel::setShowOriginal,
                )
            }
        }

        // Mainland-China network optimisation hint
        Surface(
            color = MaterialTheme.colorScheme.tertiaryContainer,
            contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
            shape = RoundedCornerShape(12.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                rcString(RcStringKeys.TRANSLATE_CN_HINT),
                modifier = Modifier.padding(12.dp),
                style = MaterialTheme.typography.bodySmall,
            )
        }

        // Cache stats
        SettingSection(rcString(RcStringKeys.TRANSLATE_CACHE_TITLE)) {
            val stats = state.cacheStats
            if (stats == null) {
                Text(rcString(RcStringKeys.COMMON_LOADING), style = MaterialTheme.typography.bodyMedium)
            } else {
                Text(
                    text = rcString(RcStringKeys.TRANSLATE_CACHE_COUNT)
                        .replace("{count}", stats.entryCount.toString()),
                    style = MaterialTheme.typography.bodyMedium,
                )
                Text(
                    text = formatBytes(stats.totalBytes) + " / " + formatBytes(stats.maxBytes),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                OutlinedButton(onClick = { viewModel.clearCache() }) {
                    Text(rcString(RcStringKeys.TRANSLATE_CACHE_CLEAR))
                }
            }
        }
    }
}

@Composable
private fun SettingSection(title: String, content: @Composable ColumnScope.() -> Unit) {
    Surface(
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.4f),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            content()
        }
    }
}

@Composable
private fun SwitchSetting(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = onChange)
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LanguagePicker(
    selected: TranslationViewModel.TranslationTarget,
    onSelect: (TranslationViewModel.TranslationTarget) -> Unit,
) {
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        TranslationViewModel.TranslationTarget.entries.forEach { target ->
            FilterChip(
                selected = target == selected,
                onClick = { onSelect(target) },
                label = {
                    Text(when (target) {
                        TranslationViewModel.TranslationTarget.Auto -> rcString(RcStringKeys.TRANSLATE_LANGUAGE_AUTO)
                        TranslationViewModel.TranslationTarget.ZhCn -> rcString(RcStringKeys.TRANSLATE_LANGUAGE_ZH_CN)
                        TranslationViewModel.TranslationTarget.ZhHant -> rcString(RcStringKeys.TRANSLATE_LANGUAGE_ZH_HANT)
                        TranslationViewModel.TranslationTarget.En -> rcString(RcStringKeys.TRANSLATE_LANGUAGE_EN)
                    })
                },
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ModePicker(
    selected: TranslationViewModel.TranslationMode,
    onSelect: (TranslationViewModel.TranslationMode) -> Unit,
) {
    SingleChoiceSegmentedButtonRow {
        TranslationViewModel.TranslationMode.entries.forEachIndexed { i, mode ->
            SegmentedButton(
                selected = mode == selected,
                onClick = { onSelect(mode) },
                shape = SegmentedButtonDefaults.itemShape(index = i, count = TranslationViewModel.TranslationMode.entries.size),
                label = {
                    Text(when (mode) {
                        TranslationViewModel.TranslationMode.Online -> rcString(RcStringKeys.TRANSLATE_MODE_ONLINE)
                        TranslationViewModel.TranslationMode.Offline -> rcString(RcStringKeys.TRANSLATE_MODE_OFFLINE)
                        TranslationViewModel.TranslationMode.Hybrid -> rcString(RcStringKeys.TRANSLATE_MODE_HYBRID)
                    })
                },
            )
        }
    }
}

private fun formatBytes(b: Long): String {
    if (b < 1024) return "${b} B"
    val units = arrayOf("KB", "MB", "GB")
    var v = b.toDouble() / 1024.0
    var idx = 0
    while (v >= 1024.0 && idx < units.size - 1) {
        v /= 1024.0
        idx++
    }
    return String.format("%.2f %s", v, units[idx])
}
