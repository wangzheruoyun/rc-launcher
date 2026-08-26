package com.rc.launcher.ui.screen

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.model.GameDirectoryType
import com.rc.launcher.ui.model.InstallRequest
import com.rc.launcher.ui.model.InstallStep
import com.rc.launcher.ui.model.LoaderVersion
import com.rc.launcher.ui.model.ModLoader
import com.rc.launcher.ui.model.stepNumber
import com.rc.launcher.ui.model.totalSteps
import com.rc.launcher.ui.navigation.InstanceDetailRoute
import com.rc.launcher.ui.navigation.InstallRoute
import com.rc.launcher.ui.viewmodel.InstallViewModel

/** Preset cover colours offered in the configure step. */
private val ICON_COLORS = listOf(
    0xFF4CAF50, 0xFF42A5F5, 0xFFEF5350, 0xFFAB47BC,
    0xFF8D6E63, 0xFFFFB300, 0xFF26C6DA, 0xFFEC407A,
)

/** Common Java major versions offered in the configure step. */
private val JAVA_VERSIONS = listOf(8, 17, 21)

/**
 * Version-installation wizard (task 13): a linear, deterministic flow that
 * collects a [InstallRequest] and persists it as a [com.rc.launcher.ui.model.GameInstance].
 *
 * Steps: loader family -> game version -> (loader version for modded) ->
 * per-instance configuration -> review & create. Branching (skipping the loader
 * step for vanilla) is handled by the pure [InstallStep] helpers; this screen
 * only renders the current step and the back/next controls.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
fun InstallWizardScreen(
    navController: NavHostController? = null,
    vm: InstallViewModel = viewModel(),
) {
    val step by vm.step.collectAsStateWithLifecycle()
    val request by vm.request.collectAsStateWithLifecycle()
    val canProceed = vm.canProceed()
    val canGoBack = vm.canGoBack()
    val isReview = step == InstallStep.REVIEW

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("安装新实例") },
                navigationIcon = {
                    IconButton(onClick = { navController?.popBackStack() }) {
                        Icon(Icons.Filled.ArrowBack, contentDescription = "返回")
                    }
                },
                actions = {
                    Text(
                        "${step.stepNumber(request)}/${step.totalSteps(request)}",
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(end = 16.dp),
                    )
                },
            )
        },
        bottomBar = {
            Surface(tonalElevation = 2.dp, modifier = Modifier.fillMaxWidth()) {
                Column(modifier = Modifier.fillMaxWidth()) {
                    LinearProgressIndicator(
                        progress = step.stepNumber(request).toFloat() / step.totalSteps(request),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    androidx.compose.foundation.layout.Row(
                        modifier = Modifier.fillMaxWidth().padding(12.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        TextButton(
                            onClick = { vm.back() },
                            enabled = canGoBack,
                        ) { Text("上一步") }
                        Button(
                            onClick = {
                                if (isReview) {
                                    // canProceed is true here, so create() is non-null;
                                    // the guard is a defensive belt-and-braces check.
                                    val created = vm.create() ?: return@Button
                                    navController?.navigate(
                                        InstanceDetailRoute(created.id),
                                    ) { popUpTo(InstallRoute::class) { inclusive = true } }
                                } else {
                                    vm.next()
                                }
                            },
                            enabled = canProceed,
                            modifier = Modifier.weight(1f),
                        ) {
                            Text(if (isReview) "创建实例" else "下一步")
                        }
                    }
                }
            }
        },
    ) { inner ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(inner)
                .verticalScroll(rememberScrollState())
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            when (step) {
                InstallStep.LOADER -> LoaderStep(request, vm::setLoader)
                InstallStep.GAME_VERSION ->
                    GameVersionStep(request, vm.availableGameVersions(), vm::setGameVersion)
                InstallStep.LOADER_VERSION ->
                    LoaderVersionStep(vm.availableLoaderVersions(), request.loaderVersion, vm::setLoaderVersion)
                InstallStep.CONFIGURE -> ConfigureStep(request, vm)
                InstallStep.REVIEW -> ReviewStep(request)
            }
        }
    }
}

/** Step 1: choose the loader family. */
@Composable
private fun LoaderStep(request: InstallRequest, onPick: (ModLoader) -> Unit) {
    StepHeader("选择版本类型", "原版或由 Mod 加载器构建的整合包。")
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        for (loader in ModLoader.entries) {
            val selected = request.loader == loader
            Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .clickable { onPick(loader) }
                    .then(
                        if (selected) {
                            Modifier.border(2.dp, loader.color, RoundedCornerShape(12.dp))
                        } else {
                            Modifier
                        },
                    ),
                color = if (selected) loader.color.copy(alpha = 0.12f) else MaterialTheme.colorScheme.surface,
                tonalElevation = if (selected) 0.dp else 1.dp,
            ) {
                androidx.compose.foundation.layout.Row(
                    modifier = Modifier.fillMaxWidth().padding(16.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Box(
                        modifier = Modifier.size(40.dp).clip(RoundedCornerShape(10.dp))
                            .background(Color(loader.accent)),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(loader.label.take(1), color = Color.White, fontWeight = FontWeight.Bold)
                    }
                    Column(modifier = Modifier.weight(1f)) {
                        Text(loader.label, style = MaterialTheme.typography.titleMedium)
                        Text(
                            when (loader) {
                                ModLoader.VANILLA -> "官方纯净客户端"
                                ModLoader.FABRIC -> "轻量、模块化 Mod 加载器"
                                ModLoader.FORGE -> "老牌、功能丰富 Mod 加载器"
                                ModLoader.QUILT -> "Fabric 分支，向后兼容"
                                ModLoader.OPTIFINE -> "性能与画质优化"
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (selected) Icon(Icons.Filled.Check, contentDescription = null, tint = loader.color)
                }
            }
        }
    }
}

/** Step 2: choose a game version (free input + catalogue). */
@Composable
private fun GameVersionStep(
    request: InstallRequest,
    versions: List<String>,
    onPick: (String) -> Unit,
) {
    StepHeader("选择游戏版本", "可从下方列表选择，或手动输入任意版本号。")
    OutlinedTextField(
        value = request.gameVersion,
        onValueChange = onPick,
        label = { Text("游戏版本") },
        placeholder = { Text("例如 1.20.1") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    if (versions.isNotEmpty()) {
        Text("推荐版本", style = MaterialTheme.typography.labelLarge)
        LazyColumn(
            modifier = Modifier.fillMaxWidth().sizeIn(maxHeight = 280.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(versions) { v ->
                val selected = request.gameVersion == v
                Surface(
                    modifier = Modifier.fillMaxWidth()
                        .clip(RoundedCornerShape(10.dp))
                        .clickable { onPick(v) },
                    color = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
                    tonalElevation = 1.dp,
                ) {
                    androidx.compose.foundation.layout.Row(
                        modifier = Modifier.fillMaxWidth().padding(14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(v, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                        if (selected) Icon(Icons.Filled.Check, contentDescription = null)
                    }
                }
            }
        }
    }
}

/** Step 3: choose a concrete loader build (modded loaders only). */
@Composable
private fun LoaderVersionStep(
    versions: List<LoaderVersion>,
    selected: LoaderVersion?,
    onPick: (LoaderVersion) -> Unit,
) {
    StepHeader("选择加载器版本", "为该游戏版本挑选一个可用的加载器构建。")
    if (versions.isEmpty()) {
        Text(
            "暂无可用版本，请确认游戏版本号是否正确。",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (lv in versions) {
            val isSel = selected?.id == lv.id
            Surface(
                modifier = Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(10.dp))
                    .clickable { onPick(lv) },
                color = if (isSel) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
                tonalElevation = 1.dp,
            ) {
                androidx.compose.foundation.layout.Row(
                    modifier = Modifier.fillMaxWidth().padding(14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(lv.displayName, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                    if (!lv.stable) {
                        Surface(color = MaterialTheme.colorScheme.tertiaryContainer, shape = RoundedCornerShape(6.dp)) {
                            Text("实验", style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp))
                        }
                    }
                    if (isSel) Icon(Icons.Filled.Check, contentDescription = null)
                }
            }
        }
    }
}

/** Step 4: per-instance configuration (name, icon, Java, version isolation). */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ConfigureStep(request: InstallRequest, vm: InstallViewModel) {
    StepHeader("实例设置", "配置名称、Java 版本与版本隔离策略。")
    OutlinedTextField(
        value = request.name,
        onValueChange = vm::setName,
        label = { Text("实例名称") },
        placeholder = { Text("留空将自动生成") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    Text("封面颜色", style = MaterialTheme.typography.labelLarge)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        for (c in ICON_COLORS) {
            val selected = request.iconColor == c
            Box(
                modifier = Modifier.size(36.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(Color(c))
                    .clickable { vm.setIconColor(c) }
                    .then(if (selected) Modifier.border(2.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp)) else Modifier),
                contentAlignment = Alignment.Center,
            ) {
                if (selected) Icon(Icons.Filled.Check, contentDescription = null, tint = Color.White)
            }
        }
    }

    Text("Java 版本", style = MaterialTheme.typography.labelLarge)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Chip(selected = request.javaVersion == null, onClick = { vm.setJavaVersion(null) }) { Text("自动") }
        for (jv in JAVA_VERSIONS) {
            Chip(selected = request.javaVersion == jv, onClick = { vm.setJavaVersion(jv) }) { Text("Java $jv") }
        }
    }

    Text("版本隔离", style = MaterialTheme.typography.labelLarge)
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        for (type in GameDirectoryType.entries) {
            val selected = request.gameDirectoryType == type
            Surface(
                modifier = Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(10.dp))
                    .clickable { vm.setGameDirectoryType(type) },
                color = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
                tonalElevation = 1.dp,
            ) {
                Column(modifier = Modifier.fillMaxWidth().padding(14.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(type.label, style = MaterialTheme.typography.titleMedium)
                    Text(type.description, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
    }

    if (request.gameDirectoryType == GameDirectoryType.CUSTOM) {
        OutlinedTextField(
            value = request.customGameDir,
            onValueChange = vm::setCustomGameDir,
            label = { Text("自定义目录路径") },
            placeholder = { Text("/sdcard/games/my-instance") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

/** Step 5: review the request and surface any validation error. */
@Composable
private fun ReviewStep(request: InstallRequest) {
    StepHeader("确认信息", "核对后将创建实例并保存到本地。")
    val error = request.validationError()
    SummaryRow("版本类型", request.loader.label)
    SummaryRow("游戏版本", request.gameVersion.ifBlank { "—" })
    if (request.requiresLoaderVersion) {
        SummaryRow("加载器版本", request.loaderVersion?.displayName ?: "—")
    }
    SummaryRow("实例名称", request.name.ifBlank { "（自动）" })
    SummaryRow("Java 版本", request.javaVersion?.let { "Java $it" } ?: "自动")
    SummaryRow("版本隔离", request.gameDirectoryType.label)
    SummaryRow("游戏目录", previewGameDir(request))

    if (error != null) {
        Surface(
            color = MaterialTheme.colorScheme.errorContainer,
            shape = RoundedCornerShape(10.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                "无法创建：$error",
                color = MaterialTheme.colorScheme.onErrorContainer,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(12.dp),
            )
        }
    }
}

/** Resolve the preview game directory for the current [request]. */
private fun previewGameDir(request: InstallRequest): String {
    val base = "games/RC"
    return when (request.gameDirectoryType) {
        GameDirectoryType.DEFAULT -> base
        GameDirectoryType.ISOLATED -> "$base/instances/${request.defaultId()}"
        GameDirectoryType.CUSTOM ->
            request.customGameDir.takeIf { it.isNotBlank() } ?: "$base/instances/${request.defaultId()}"
    }
}

@Composable
private fun StepHeader(title: String, subtitle: String) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(title, style = MaterialTheme.typography.headlineSmall)
        Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun SummaryRow(label: String, value: String) {
    androidx.compose.foundation.layout.Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun Chip(selected: Boolean, onClick: () -> Unit, content: @Composable () -> Unit) {
    Surface(
        modifier = Modifier.clip(RoundedCornerShape(20.dp)).clickable { onClick() },
        color = if (selected) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surface,
        tonalElevation = 1.dp,
    ) {
        Box(modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) { content() }
    }
}
