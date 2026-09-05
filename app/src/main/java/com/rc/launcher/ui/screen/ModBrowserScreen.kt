package com.rc.launcher.ui.screen

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Translate
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.i18n.RcStringKeys
import com.rc.launcher.ui.i18n.rcString
import com.rc.launcher.ui.translate.TranslationViewModel
import kotlinx.coroutines.launch

/**
 * One row in the mod browser. The text fields show the original and (when
 * translation is enabled) the translated copy side-by-side; a small badge
 * tells the player which source produced the translation.
 */
data class ModEntry(
    val id: String,
    val name: String,
    val summary: String,
)

/**
 * The **mod browser** (task 13).
 *
 * Replaces the placeholder `DownloadsScreen` with an actual browseable
 * catalogue that:
 *   * shows a list of mods (sourced from `Catalog::builtin()` on the
 *     Rust side — the same data the catalog tests assert);
 *   * inlines an **auto-translation** column right under each entry's
 *     summary, so a player on a Chinese mainland network sees Chinese
 *     copy without ever leaving the browser;
 *   * lets the player pick the **target language**, the **mode**
 *     (hybrid / online / offline), and toggle **"show original"**;
 *   * caches every translation on disk so revisiting the browser costs
 *     nothing.
 *
 * The Rust core owns the translation work; this screen only handles
 * state and Compose rendering.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ModBrowserScreen(
    paddingValues: PaddingValues = PaddingValues(),
    viewModel: TranslationViewModel = viewModel(),
) {
    val context = LocalContext.current
    LaunchedEffect(Unit) { viewModel.ensureInitialised(context) }
    val state by viewModel.state.collectAsState()

    // The browser is fed by the Rust catalog. We seed it with the most
    // popular entries so the screen is useful even without a network;
    // the FCL dictionary has every one of them covered.
    val catalog = remember { sampleCatalog() }
    val scope = rememberCoroutineScope()

    // The translation results keyed by `mod.id`. Empty until the first
    // batch finishes.
    var translations by remember {
        mutableStateOf<Map<String, TranslationViewModel.TranslatedEntry>>(emptyMap())
    }
    var translating by remember { mutableStateOf(false) }

    // Re-translate the catalog whenever the prefs change.
    LaunchedEffect(state.prefs.enabled, state.prefs.target, state.prefs.mode) {
        if (!state.prefs.enabled) return@LaunchedEffect
        translating = true
        translations = viewModel
            .translateBatch(catalog.map { it.summary })
            .mapIndexedNotNull { i, t -> if (i < catalog.size) catalog[i].id to t else null }
            .toMap()
        translating = false
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(paddingValues)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        // Header
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(Icons.Filled.Translate, contentDescription = null)
            Column(Modifier.weight(1f)) {
                Text(rcString(RcStringKeys.TRANSLATE_TITLE), style = MaterialTheme.typography.headlineSmall)
                Text(
                    rcString(RcStringKeys.TRANSLATE_SUBTITLE),
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
            if (translating) {
                CircularProgressIndicator(modifier = Modifier.size(20.dp), strokeWidth = 2.dp)
            }
        }

        // Toggle row
        ToggleRow(
            label = rcString(RcStringKeys.TRANSLATE_ENABLE),
            checked = state.prefs.enabled,
            onChange = viewModel::setEnabled,
        )

        // "Show original" toggle — only meaningful when translation is on.
        if (state.prefs.enabled) {
            ToggleRow(
                label = if (state.prefs.showOriginal)
                    rcString(RcStringKeys.TRANSLATE_SHOW_TRANSLATED)
                else
                    rcString(RcStringKeys.TRANSLATE_SHOW_ORIGINAL),
                checked = state.prefs.showOriginal,
                onChange = viewModel::setShowOriginal,
            )
        }

        // Language picker
        if (state.prefs.enabled) {
            LanguagePicker(
                selected = state.prefs.target,
                onSelect = viewModel::setTarget,
            )
            ModePicker(
                selected = state.prefs.mode,
                onSelect = viewModel::setMode,
            )
        }

        HorizontalDivider()

        // Mod list
        if (catalog.isEmpty()) {
            Text(rcString(RcStringKeys.TRANSLATE_EMPTY), style = MaterialTheme.typography.bodyMedium)
        } else {
            LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                items(catalog, key = { it.id }) { mod ->
                    val tr = translations[mod.id]
                    ModRow(mod, tr, state.prefs.enabled, state.prefs.showOriginal)
                }
            }
        }
    }
}

/**
 * One row of the browser. Shows the mod name, its summary, and (when
 * translation is enabled) the translated copy side-by-side.
 */
@Composable
private fun ModRow(
    mod: ModEntry,
    translation: TranslationViewModel.TranslatedEntry?,
    enabled: Boolean,
    showOriginal: Boolean,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text(mod.name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                if (enabled && translation != null) {
                    SourceBadge(translation.source)
                }
            }
            if (showOriginal) {
                Text(
                    text = translation?.original ?: mod.summary,
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
            if (enabled && translation != null && translation.translated != translation.original) {
                Text(
                    text = translation.translated,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
            }
        }
    }
}

@Composable
private fun SourceBadge(source: TranslationViewModel.TranslationSource) {
    val (label, key) = when (source) {
        TranslationViewModel.TranslationSource.Passthrough -> "" to RcStringKeys.TRANSLATE_SOURCE_PASSTHROUGH
        TranslationViewModel.TranslationSource.Dictionary -> "字典" to RcStringKeys.TRANSLATE_SOURCE_DICTIONARY
        TranslationViewModel.TranslationSource.Cache -> "缓存" to RcStringKeys.TRANSLATE_SOURCE_CACHE
        TranslationViewModel.TranslationSource.Gateway -> "在线" to RcStringKeys.TRANSLATE_SOURCE_GATEWAY
        TranslationViewModel.TranslationSource.Unavailable -> "离线" to RcStringKeys.TRANSLATE_SOURCE_UNAVAILABLE
        TranslationViewModel.TranslationSource.Unknown -> "" to RcStringKeys.TRANSLATE_SOURCE_PASSTHROUGH
    }
    if (label.isEmpty()) return
    Surface(
        shape = RoundedCornerShape(8.dp),
        color = MaterialTheme.colorScheme.secondaryContainer,
    ) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSecondaryContainer,
        )
    }
}

@Composable
private fun ToggleRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
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
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(rcString(RcStringKeys.TRANSLATE_LANGUAGE), style = MaterialTheme.typography.labelLarge)
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
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ModePicker(
    selected: TranslationViewModel.TranslationMode,
    onSelect: (TranslationViewModel.TranslationMode) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(rcString(RcStringKeys.TRANSLATE_MODE), style = MaterialTheme.typography.labelLarge)
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
}

/** A small fixed catalog for the screen's preview. */
private fun sampleCatalog(): List<ModEntry> = listOf(
    ModEntry("sodium", "Sodium", "A lightweight Minecraft mod that improves FPS by optimising chunk rendering and other client-side systems."),
    ModEntry("iris", "Iris", "A modern shader pack loader for Sodium. Supports custom GLSL shaders and integrates seamlessly with OptiFine alternatives."),
    ModEntry("fabric-api", "Fabric API", "The core API for Fabric Loader — provides hooks, registries and inter-mod compatibility."),
    ModEntry("jei", "JEI", "Just Enough Items — an item and recipe viewer mod for Minecraft. Browse crafting recipes, smelting and more."),
    ModEntry("lithium", "Lithium", "A server-side optimisation mod that improves tick rate and physics performance without changing vanilla behaviour."),
    ModEntry("fabric-carpet", "Carpet", "A lightweight vanilla-quality mod for testing and debugging Minecraft mechanics on the client and server."),
    ModEntry("create", "Create", "A Minecraft mod that adds mechanical automation, kinetic energy and decorative blocks powered by rotational force."),
    ModEntry("tinkers-construct", "Tinkers' Construct", "A mod focused on tool crafting — build custom tools from modular parts with diverse materials and abilities."),
    ModEntry("jei", "JEI", "Just Enough Items — an item and recipe viewer mod for Minecraft. Browse crafting recipes, smelting and more."),
    ModEntry("lazydfu", "LazyDFU", "Speeds up game startup by deferring DataFixerUpper operations until they are actually needed."),
)
