package com.rc.launcher.ui.screen

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.tooling.preview.Preview
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import android.content.Intent
import android.net.Uri
import androidx.compose.ui.platform.LocalContext
import com.rc.launcher.ui.theme.BackgroundConfig
import com.rc.launcher.ui.theme.BackgroundEffect
import com.rc.launcher.ui.theme.BackgroundCache
import com.rc.launcher.ui.theme.BackgroundValidator
import com.rc.launcher.ui.theme.BackgroundLimits
import com.rc.launcher.ui.theme.RcBackground
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavHostController
import com.rc.launcher.ui.ProvideRcWindowInfo
import com.rc.launcher.ui.navigation.AwtRoute
import com.rc.launcher.ui.navigation.ControllerRoute
import com.rc.launcher.ui.model.LauncherSettings
import com.rc.launcher.ui.model.OrientationMode
import com.rc.launcher.ui.model.ResolutionMode
import com.rc.launcher.ui.model.RendererOption
import com.rc.launcher.ui.model.RendererPluginConfig
import com.rc.launcher.ui.model.MirrorCatalog
import com.rc.launcher.ui.model.MirrorProbeState
import com.rc.launcher.ui.rcWindowInfo
import com.rc.launcher.ui.theme.ThemeData
import com.rc.launcher.ui.theme.ThemeNightMode
import com.rc.launcher.ui.theme.ThemeViewModel
import com.rc.launcher.ui.component.FloatingHudConfig
import com.rc.launcher.ui.component.InputMode
import com.rc.launcher.ui.viewmodel.DashboardViewModel
import com.rc.launcher.ui.viewmodel.SettingsViewModel
import com.rc.launcher.ui.viewmodel.TutorialViewModel
import com.rc.launcher.ui.navigation.AccountsRoute
import com.rc.launcher.ui.navigation.OnboardingRoute
import com.rc.launcher.ui.viewmodel.LocaleViewModel
import com.rc.launcher.ui.i18n.AppLanguage
import com.rc.launcher.ui.i18n.LocalRcStrings
import com.rc.launcher.ui.i18n.RcStringKeys
import kotlin.math.roundToInt
import kotlinx.coroutines.launch

/**
 * Settings Center (task 14).
 *
 * The single screen that exposes every global launcher preference: appearance,
 * the China-mainland network optimisations (mirror source + DoH), the Java /
 * memory knobs, the renderer + window tuning, the controller mapping and the
 * game-directory configuration. It maps 1:1 onto FCL's settings panel and the
 * renderer-plugin configuration items.
 *
 * **Adaptive (task 9).** The screen keys its padding and its maximum text width
 * off the measured window ([rcWindowInfo]) and hosts the screen-orientation
 * preference (跟随系统 / 强制横屏 / 强制竖屏) that [com.rc.launcher.MainActivity]
 * applies to the Activity.
 *
 * All state lives in [SettingsViewModel] (one [LauncherSettings] [StateFlow]);
 * the appearance sub-section reuses the [ThemeViewModel] from task 11. Every
 * mutator sanitises its input, so the UI can never push an out-of-range value
 * into the Rust core (task 19).
 */
@Composable
fun SettingsScreen(
    settingsViewModel: SettingsViewModel = viewModel(),
    themeViewModel: ThemeViewModel = viewModel(),
    localeViewModel: LocaleViewModel = viewModel(),
    tutorialViewModel: TutorialViewModel = viewModel(),
    dashboard: DashboardViewModel = viewModel(),
    navController: NavHostController? = null,
) {
    val settings by settingsViewModel.settings.collectAsStateWithLifecycle()
    val mirrorProbe by settingsViewModel.mirrorProbe.collectAsStateWithLifecycle()
    // Task 20: floating HUD config + input mode (Activity-scoped, shared with HomeScreen)
    val hudConfig by dashboard.hudConfig.collectAsStateWithLifecycle()
    val hudInputMode by dashboard.inputMode.collectAsStateWithLifecycle()
    val themes by themeViewModel.availableThemes.collectAsStateWithLifecycle()
    val currentTheme by themeViewModel.currentTheme.collectAsStateWithLifecycle()
    val nightMode by themeViewModel.nightMode.collectAsStateWithLifecycle()
    val backgroundConfig by themeViewModel.backgroundConfig.collectAsStateWithLifecycle()
    // Task 20: the string table drives every label below; collecting the selected
    // language separately keeps the picker in sync with the engine.
    val strings = LocalRcStrings.current
    val selectedLanguage by localeViewModel.selected.collectAsStateWithLifecycle()
    val effectiveLanguage by localeViewModel.effective.collectAsStateWithLifecycle()

    // Task 9: the settings rows are wide; on a landscape phone or a tablet they
    // are capped and padded instead of being stretched edge to edge.
    val window = rcWindowInfo()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(window.contentPaddingDp.dp)
            .widthIn(max = window.maxContentWidthDp.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // ---- Live validation (task 14 / task 19 robustness) --------------
        settings.validationError()?.let { err ->
            Surface(
                color = MaterialTheme.colorScheme.errorContainer,
                contentColor = MaterialTheme.colorScheme.onErrorContainer,
                shape = RoundedCornerShape(12.dp),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(
                    err,
                    style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.padding(12.dp),
                )
            }
        }

        // ---- Appearance (task 11) -----------------------------------------
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_APPEARANCE]) {
            Text(
                strings[RcStringKeys.THEME_NIGHT_TOGGLE],
                style = MaterialTheme.typography.titleMedium,
            )
            val modes = listOf(ThemeNightMode.SYSTEM, ThemeNightMode.LIGHT, ThemeNightMode.DARK)
            val modeLabel = mapOf(
                ThemeNightMode.SYSTEM to strings[RcStringKeys.THEME_NIGHT_SYSTEM],
                ThemeNightMode.LIGHT to strings[RcStringKeys.THEME_NIGHT_LIGHT],
                ThemeNightMode.DARK to strings[RcStringKeys.THEME_NIGHT_DARK],
            )
            SingleChoiceSegmentedButtonRow {
                for ((index, mode) in modes.withIndex()) {
                    SegmentedButton(
                        selected = nightMode == mode,
                        onClick = { themeViewModel.setNightMode(mode) },
                        shape = SegmentedButtonDefaults.itemShape(index = index, count = modes.size),
                    ) {
                        Text(modeLabel[mode] ?: mode.name)
                    }
                }
            }

            Text("主题配色", style = MaterialTheme.typography.titleMedium)
            LazyRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                items(themes, key = { it.id }) { theme ->
                    ThemeColorCard(
                        theme = theme,
                        selected = theme.id == currentTheme.id,
                        onClick = { themeViewModel.selectTheme(theme.id) },
                    )
                }
            }
        }

        // ---- Custom launcher background (task 11) --------------------------
        BackgroundSettingsSection(
            config = backgroundConfig,
            nightMode = nightMode,
            onChange = { themeViewModel.setBackgroundConfig(it) },
        )

        // ---- Screen & orientation (task 9) --------------------------------
        SettingsSection("屏幕与方向") {
            Text("屏幕方向", style = MaterialTheme.typography.titleMedium)
            val orientations: List<OrientationMode> = settingsViewModel.orientationModes
            SingleChoiceSegmentedButtonRow {
                for ((index, mode) in orientations.withIndex()) {
                    SegmentedButton(
                        selected = settings.orientationMode() == mode,
                        onClick = { settingsViewModel.setOrientation(mode.id) },
                        shape = SegmentedButtonDefaults.itemShape(
                            index = index,
                            count = orientations.size,
                        ),
                    ) {
                        Text(mode.label)
                    }
                }
            }
            Text(
                settings.orientationMode().description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // Live geometry: makes the adaptive breakpoints (and a rotation)
            // observable instead of magic, and doubles as a bug report aid.
            Text(
                "当前窗口 ${window.widthDp} x ${window.heightDp} dp · " +
                    "${if (window.isLandscape) "横屏" else "竖屏"} · " +
                    "宽度 ${window.widthClass.id} / 高度 ${window.heightClass.id} · " +
                    "${if (window.usesNavigationRail) "侧边导航栏" else "底部导航栏"} · " +
                    "实例 ${window.instanceColumns} 列",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                "旋转屏幕不会重建界面（Activity 声明了 configChanges），因此界面状态与" +
                    "游戏画面的坐标系都不会丢失；横竖屏切换时会释放正在按下的按键，避免" +
                    "触摸坐标错位。",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // ---- Language & region (task 20) ----------------------------------
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_LANGUAGE]) {
            DropdownSetting(
                title = strings[RcStringKeys.SETTINGS_LANGUAGE_TITLE],
                subtitle = strings[RcStringKeys.SETTINGS_LANGUAGE_SUBTITLE],
                // Endonyms are never translated, so the list stays readable even
                // if the user is currently looking at a language they misclicked.
                options = localeViewModel.options.map { language ->
                    language.tag to localeViewModel.labelFor(language, strings)
                },
                selectedId = selectedLanguage.tag,
                onSelect = { tag -> localeViewModel.setLanguageTag(tag) },
            )
            // "Follow system" is ambiguous on its own — show what it resolved to.
            Text(
                text = strings.format(
                    RcStringKeys.SETTINGS_LANGUAGE_APPLIED,
                    "language" to effectiveLanguage.nativeName,
                ),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (selectedLanguage == AppLanguage.SYSTEM) {
                Text(
                    text = strings[RcStringKeys.SETTINGS_LANGUAGE_FOLLOW_SYSTEM],
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        // ---- Translation (task 13) -----------------------------------------
        SettingsSection(strings[RcStringKeys.TRANSLATE_TITLE]) {
            Text(
                strings[RcStringKeys.TRANSLATE_SUBTITLE],
                style = MaterialTheme.typography.bodyMedium,
            )
            if (navController != null) {
                OutlinedButton(onClick = { navController.navigate(com.rc.launcher.ui.navigation.TranslationRoute) }) {
                    Text(strings[RcStringKeys.TRANSLATE_GATEWAY_TITLE])
                }
            }
        }

        // ---- Network / China optimisation ---------------------------------
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_NETWORK]) {
            DropdownSetting(
                title = "下载镜像源",
                subtitle = "下载版本 / 资源时使用的国内镜像",
                options = settingsViewModel.mirrors.map { it.id to it.name },
                selectedId = settings.mirrorId,
                onSelect = settingsViewModel::setMirror,
            )
            SwitchSetting(
                title = "自动选择最快镜像",
                subtitle = "启动前测速并择优（对应 Rust 核心 task 3）",
                checked = settings.autoSelectFastestMirror,
                onCheckedChange = settingsViewModel::setAutoSelectFastestMirror,
            )
            SwitchSetting(
                title = "启用 DoH（DNS over HTTPS）",
                subtitle = "加密解析，规避 DNS 污染",
                checked = settings.useDoh,
                onCheckedChange = settingsViewModel::setUseDoh,
            )
            if (settings.useDoh) {
                DropdownSetting(
                    title = "DoH 服务器",
                    subtitle = "默认阿里 DNS，可切换至 DNSPod / 360 / Cloudflare",
                    options = settingsViewModel.dohServers.map { it.url to it.name },
                    selectedId = settings.dohServerUrl,
                    onSelect = settingsViewModel::setDohServer,
                )
            }

            // ---- Mirror speed test (China optimisation, task 3 / task 14) ----
            val scope = rememberCoroutineScope()
            Button(
                onClick = { scope.launch { settingsViewModel.measureAndSelectFastestMirror() } },
                enabled = mirrorProbe !is MirrorProbeState.Measuring,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(
                    when (val probe = mirrorProbe) {
                        is MirrorProbeState.Measuring -> "测速中 ${probe.done}/${probe.total}…"
                        else -> "测速并选择最快镜像"
                    },
                )
            }
            when (val probe = mirrorProbe) {
                is MirrorProbeState.Idle -> {}
                is MirrorProbeState.Measuring -> {}
                is MirrorProbeState.Done -> Text(
                    if (probe.bestId != null) {
                        "已选择最快镜像：${MirrorCatalog.fromId(probe.bestId).name}"
                    } else {
                        "所有镜像均不可达，请检查网络"
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = if (probe.bestId != null) {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    } else {
                        MaterialTheme.colorScheme.error
                    },
                )
                is MirrorProbeState.Error -> Text(
                    probe.message ?: "测速失败",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }

        // ---- Java / memory -------------------------------------------------
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_JAVA]) {
            SwitchSetting(
                title = "自动分配内存",
                subtitle = "根据设备内存自动选择 -Xmx",
                checked = settings.autoAllocateMemory,
                onCheckedChange = settingsViewModel::setAutoAllocateMemory,
            )
            SliderSetting(
                title = "最大内存 (-Xmx)",
                enabled = !settings.autoAllocateMemory,
                value = settings.javaHeapMb.toFloat(),
                valueRange = LauncherSettings.MIN_HEAP_MB.toFloat()..LauncherSettings.MAX_HEAP_MB.toFloat(),
                steps = ((LauncherSettings.MAX_HEAP_MB - LauncherSettings.MIN_HEAP_MB) / 128) - 1,
                onValueChange = { settingsViewModel.setJavaHeapMb(it.roundToInt()) },
                label = "${settings.javaHeapMb} MB",
            )
            IntTextFieldSetting(
                title = "初始内存 (-Xms，0 = 不设置)",
                value = settings.javaMinHeapMb ?: 0,
                onValueChange = { settingsViewModel.setJavaMinHeapMb(if (it <= 0) null else it) },
            )
            DropdownSetting(
                title = "Java 版本",
                subtitle = "默认跟随实例；可全局指定",
                options = listOf(
                    "" to "自动",
                    "8" to "Java 8",
                    "17" to "Java 17",
                    "21" to "Java 21",
                ),
                selectedId = settings.javaVersion?.toString() ?: "",
                onSelect = { settingsViewModel.setJavaVersion(if (it.isBlank()) null else it.toIntOrNull()) },
            )
            TextFieldSetting(
                title = "JVM 启动参数",
                subtitle = "附加到 java 命令行的参数（-开头）",
                value = settings.javaArgs,
                onValueChange = settingsViewModel::setJavaArgs,
                singleLine = false,
            )
        }

        // ---- Renderer / window --------------------------------------------
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_RENDERER]) {
            DropdownSetting(
                title = "渲染器",
                subtitle = "OpenGL(ES) 转译栈（对应各 RendererPlugin）",
                options = settingsViewModel.renderers.map { it.id to it.label },
                selectedId = settings.rendererId,
                onSelect = settingsViewModel::setRenderer,
            )

            // ---- Per-renderer plugin options (task 14) -----------------------
            // Each branch mirrors the configuration items of the corresponding
            // renderer plugin / native library (see FCL_NATIVE_LIBRARIES.md).
            when (settings.renderer()) {
                RendererOption.ZINK -> DropdownSetting(
                    title = "Zink Vulkan 驱动",
                    subtitle = "选择绑定的 Vulkan 驱动（对应 libvulkan_freedreno.so / Turnip）",
                    options = RendererPluginConfig.ZINK_DRIVERS,
                    selectedId = settings.rendererOptions.zinkVulkanDriver,
                    onSelect = settingsViewModel::setZinkVulkanDriver,
                )
                RendererOption.ANGLE -> DropdownSetting(
                    title = "ANGLE 后端",
                    subtitle = "Vulkan / OpenGL / 关闭（仅测试）",
                    options = RendererPluginConfig.ANGLE_BACKENDS,
                    selectedId = settings.rendererOptions.angleBackend,
                    onSelect = settingsViewModel::setAngleBackend,
                )
                RendererOption.GL4ES, RendererOption.NG_GL4ES -> SwitchSetting(
                    title = "禁用 GL4ES sRGB 模拟",
                    subtitle = "部分驱动 / 模组下更稳定",
                    checked = settings.rendererOptions.gl4esNoSrgb,
                    onCheckedChange = settingsViewModel::setGl4esNoSrgb,
                )
                RendererOption.VIRGL -> TextFieldSetting(
                    title = "VirGL 服务器",
                    subtitle = "留空使用本地；可填 host:port 连接远程 virglrenderer",
                    value = settings.rendererOptions.virglServer,
                    onValueChange = settingsViewModel::setVirglServer,
                )
                else -> { /* no extra plugin options for this renderer */ }
            }

            val resModes = listOf(ResolutionMode.AUTO, ResolutionMode.CUSTOM)
            Text("分辨率", style = MaterialTheme.typography.titleMedium)
            SingleChoiceSegmentedButtonRow {
                for ((index, mode) in resModes.withIndex()) {
                    SegmentedButton(
                        selected = settings.resolutionMode == mode,
                        onClick = { settingsViewModel.setResolutionMode(mode) },
                        shape = SegmentedButtonDefaults.itemShape(index = index, count = resModes.size),
                    ) {
                        Text(mode.label)
                    }
                }
            }
            if (settings.resolutionMode == ResolutionMode.CUSTOM) {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    IntTextFieldSetting(
                        title = "宽",
                        modifier = Modifier.weight(1f),
                        value = settings.customWidth,
                        onValueChange = { settingsViewModel.setCustomResolution(it, settings.customHeight) },
                    )
                    IntTextFieldSetting(
                        title = "高",
                        modifier = Modifier.weight(1f),
                        value = settings.customHeight,
                        onValueChange = { settingsViewModel.setCustomResolution(settings.customWidth, it) },
                    )
                }
            }
            SliderSetting(
                title = "分辨率缩放",
                value = settings.resolutionScale,
                valueRange = LauncherSettings.MIN_SCALE..LauncherSettings.MAX_SCALE,
                steps = 14,
                onValueChange = settingsViewModel::setResolutionScale,
                label = "${(settings.resolutionScale * 100).roundToInt()}%",
            )
            SliderSetting(
                title = "帧率限制 (0 = 不限制)",
                value = settings.framerateLimit.toFloat(),
                valueRange = LauncherSettings.MIN_FRAMERATE.toFloat()..LauncherSettings.MAX_FRAMERATE.toFloat(),
                steps = LauncherSettings.MAX_FRAMERATE - 1,
                onValueChange = { settingsViewModel.setFramerateLimit(it.roundToInt()) },
                label = if (settings.framerateLimit == 0) "不限制" else "${settings.framerateLimit} FPS",
            )
            SwitchSetting(
                title = "全屏",
                subtitle = "以全屏方式运行游戏",
                checked = settings.fullscreen,
                onCheckedChange = settingsViewModel::setFullscreen,
            )
            // Task 18: the AWT/Swing compatibility layer draws Minecraft's
            // *embedded* (non-OpenGL) UI — installers, dialogs, font metrics.
            Button(
                onClick = { navController?.navigate(AwtRoute) },
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("AWT / Swing 兼容层（内嵌界面）")
            }
        }

        // ---- Controller ----------------------------------------------------
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_CONTROLLER]) {
            SwitchSetting(
                title = "启用手柄",
                subtitle = "外接手柄 / 触屏摇杆映射",
                checked = settings.controllerEnabled,
                onCheckedChange = settingsViewModel::setControllerEnabled,
            )
            DropdownSetting(
                title = "按键布局",
                subtitle = "选择或自定义映射方案",
                options = listOf(
                    "default" to "默认布局",
                    "wasd" to "WASD + 鼠标",
                    "gamepad" to "手柄布局",
                ),
                selectedId = settings.controllerLayoutId,
                onSelect = settingsViewModel::setControllerLayout,
                enabled = settings.controllerEnabled,
            )
            SliderSetting(
                title = "摇杆死区",
                enabled = settings.controllerEnabled,
                value = settings.controllerDeadzone,
                valueRange = 0f..1f,
                steps = 19,
                onValueChange = settingsViewModel::setControllerDeadzone,
                label = "${(settings.controllerDeadzone * 100).roundToInt()}%",
            )
            SwitchSetting(
                title = "震动反馈",
                checked = settings.controllerVibration,
                onCheckedChange = settingsViewModel::setControllerVibration,
                enabled = settings.controllerEnabled,
            )
            Button(
                onClick = { navController?.navigate(ControllerRoute) },
                enabled = settings.controllerEnabled,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("编辑按键映射")
            }
        }

        // ---- Directory / misc ---------------------------------------------
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_DIRECTORY]) {
            TextFieldSetting(
                title = "游戏文件根目录",
                subtitle = "留空使用应用默认目录",
                value = settings.gameFilesRoot,
                onValueChange = settingsViewModel::setGameFilesRoot,
            )
            SwitchSetting(
                title = "自动清理日志",
                subtitle = "启动后自动清理过期日志",
                checked = settings.autoCleanLogs,
                onCheckedChange = settingsViewModel::setAutoCleanLogs,
            )
            SwitchSetting(
                title = "保留崩溃报告",
                subtitle = "保存 hs_err / 崩溃日志以便排查",
                checked = settings.keepCrashReports,
                onCheckedChange = settingsViewModel::setKeepCrashReports,
            )
        }

        // ---- Game HUD settings (task 20) -----------------------------------
        SettingsSection(strings[RcStringKeys.HUD_SETTING_TITLE]) {
            Text(
                "游戏内悬浮菜单的显示与行为。这些设置实时生效，无需重启。",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            // Opacity
            Text(
                text = strings[RcStringKeys.HUD_SETTING_OPACITY] + ": " +
                    "${(hudConfig.opacity * 100).roundToInt()}%",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Slider(
                value = hudConfig.opacity,
                onValueChange = { dashboard.setHudConfig(hudConfig.copy(opacity = it)) },
                valueRange = 0.1f..1.0f,
                modifier = Modifier.testTag("settings_hud_opacity_slider"),
            )

            // Auto-hide
            SwitchSetting(
                title = strings[RcStringKeys.HUD_SETTING_AUTO_HIDE],
                subtitle = strings[RcStringKeys.HUD_SETTING_AUTO_HIDE_SUMMARY],
                checked = hudConfig.autoHide,
                onCheckedChange = { dashboard.setHudConfig(hudConfig.copy(autoHide = it)) },
            )

            // Anti-misoperation zone
            SwitchSetting(
                title = strings[RcStringKeys.HUD_SETTING_ANTI_MISOPERATION],
                subtitle = strings[RcStringKeys.HUD_SETTING_ANTI_MISOPERATION_SUMMARY],
                checked = hudConfig.antiMisoperationZone,
                onCheckedChange = { dashboard.setHudConfig(hudConfig.copy(antiMisoperationZone = it)) },
            )

            // Log touch-through (task 20)
            SwitchSetting(
                title = strings[RcStringKeys.HUD_LOG_PASSTHROUGH],
                subtitle = strings[RcStringKeys.HUD_LOG_PASSTHROUGH_SUMMARY],
                checked = hudConfig.logTouchThrough,
                onCheckedChange = { dashboard.setHudConfig(hudConfig.copy(logTouchThrough = it)) },
            )

            // Current input mode
            Text(
                text = strings[RcStringKeys.HUD_ACTION_SWITCH_INPUT] + ": " +
                    if (hudInputMode.isMouse) {
                        strings[RcStringKeys.HUD_INPUT_MODE_MOUSE]
                    } else {
                        strings[RcStringKeys.HUD_INPUT_MODE_TOUCH]
                    },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }

        // ---- Data & backup (task 14) --------------------------------------
        HorizontalDivider()
        SettingsSection("数据与备份") {
            var backupText by remember {
                mutableStateOf(settingsViewModel.exportSettings())
            }
            TextFieldSetting(
                title = "设置备份（导出）",
                subtitle = "复制下方文本以备份当前全部设置",
                value = backupText,
                onValueChange = { backupText = it },
                singleLine = false,
            )
            Button(
                onClick = { backupText = settingsViewModel.exportSettings() },
                modifier = Modifier.fillMaxWidth(),
            ) { Text("刷新导出文本") }
            var importText by remember { mutableStateOf("") }
            TextFieldSetting(
                title = "从备份恢复",
                subtitle = "粘贴此前导出的文本后点击恢复",
                value = importText,
                onValueChange = { importText = it },
                singleLine = false,
            )
            Button(
                onClick = {
                    if (settingsViewModel.importSettings(importText)) {
                        backupText = settingsViewModel.exportSettings()
                        importText = ""
                    }
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text("恢复设置") }
        }

        // ---- Help / tutorial replay (task 14) ----------------------------
        HorizontalDivider()
        SettingsSection(strings[RcStringKeys.TUTORIAL_TITLE]) {
            Text(
                strings[RcStringKeys.TUTORIAL_REPLAY_SUMMARY],
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Button(
                onClick = {
                    // Re-open the first page of the onboarding without clearing
                    // the "completed" flag — the gate in [com.rc.launcher.ui.RcApp]
                    // honours the flag, so this only navigates explicitly.
                    tutorialViewModel.rewatch()
                    navController?.navigate(OnboardingRoute)
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text(strings[RcStringKeys.TUTORIAL_REPLAY_BUTTON]) }
            // Task 23: skin import tutorial replay, linked with task 14.
            Button(
                onClick = {
                    tutorialViewModel.skinStart()
                    navController?.navigate(AccountsRoute)
                },
                modifier = Modifier.fillMaxWidth(),
            ) { Text(strings[RcStringKeys.SKIN_TUTORIAL_REWATCH]) }
        }

        // ---- About + reset ------------------------------------------------
        HorizontalDivider()
        SettingsSection(strings[RcStringKeys.SETTINGS_SECTION_ABOUT]) {
            Text(
                "RC Launcher —— Rust 核心 + Jetpack Compose 的 Minecraft Java 版启动器，" +
                    "针对中国大陆网络环境优化（镜像源 / 断点续传 / DNS 优化）。",
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        Button(
            onClick = settingsViewModel::resetToDefaults,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text("恢复默认设置")
        }
    }
}

// ============================================================================
// Reusable settings primitives
// ============================================================================

@Composable
private fun SettingsSection(title: String, content: @Composable ColumnScope.() -> Unit) {
    Surface(
        tonalElevation = 1.dp,
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(title, style = MaterialTheme.typography.titleLarge)
            content()
        }
    }
}

@Composable
private fun SwitchSetting(
    title: String,
    subtitle: String? = null,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    enabled: Boolean = true,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Column(modifier = Modifier.weight(1f).padding(end = 12.dp)) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            if (subtitle != null) {
                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
        Switch(checked = checked, onCheckedChange = onCheckedChange, enabled = enabled)
    }
}

@Composable
private fun SliderSetting(
    title: String,
    value: Float,
    valueRange: ClosedFloatingPointRange<Float>,
    steps: Int,
    onValueChange: (Float) -> Unit,
    label: String,
    enabled: Boolean = true,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Row(modifier = Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(title, style = MaterialTheme.typography.bodyLarge)
            Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.primary)
        }
        Slider(
            value = value,
            onValueChange = onValueChange,
            valueRange = valueRange,
            steps = steps,
            enabled = enabled,
        )
    }
}

@Composable
private fun DropdownSetting(
    title: String,
    subtitle: String? = null,
    options: List<Pair<String, String>>,
    selectedId: String,
    onSelect: (String) -> Unit,
    enabled: Boolean = true,
) {
    var expanded by remember { mutableStateOf(false) }
    val selectedLabel = options.firstOrNull { it.first == selectedId }?.second ?: selectedId
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        if (subtitle != null) {
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        ExposedDropdownMenuBox(
            expanded = expanded && enabled,
            onExpandedChange = { expanded = it },
            modifier = Modifier.fillMaxWidth(),
        ) {
            OutlinedTextField(
                value = selectedLabel,
                onValueChange = {},
                readOnly = true,
                enabled = enabled,
                modifier = Modifier.menuAnchor().fillMaxWidth(),
                trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = expanded) },
                colors = ExposedDropdownMenuDefaults.outlinedTextFieldColors(),
            )
            ExposedDropdownMenu(
                expanded = expanded && enabled,
                onDismissRequest = { expanded = false },
            ) {
                for ((id, label) in options) {
                    DropdownMenuItem(
                        text = { Text(label) },
                        onClick = {
                            onSelect(id)
                            expanded = false
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun TextFieldSetting(
    title: String,
    subtitle: String? = null,
    value: String,
    onValueChange: (String) -> Unit,
    singleLine: Boolean = true,
) {
    Column(modifier = Modifier.fillMaxWidth()) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        if (subtitle != null) {
            Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        OutlinedTextField(
            value = value,
            onValueChange = onValueChange,
            modifier = Modifier.fillMaxWidth(),
            singleLine = singleLine,
            keyboardOptions = KeyboardOptions.Default,
        )
    }
}

@Composable
private fun IntTextFieldSetting(
    title: String,
    value: Int,
    onValueChange: (Int) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxWidth()) {
        Text(title, style = MaterialTheme.typography.bodyLarge)
        OutlinedTextField(
            value = if (value == 0) "" else value.toString(),
            onValueChange = { onValueChange(it.toIntOrNull() ?: 0) },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        )
    }
}

// ============================================================================
// Appearance cards (kept from the task-11 implementation)
// ============================================================================

@Composable
private fun ThemeColorCard(
    theme: ThemeData,
    selected: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val scheme = theme.colorScheme(dark = false)
    Surface(
        modifier = modifier
            .width(120.dp)
            .clickable(onClick = onClick),
        shape = RoundedCornerShape(12.dp),
        tonalElevation = if (selected) 6.dp else 1.dp,
        border = if (selected) ButtonDefaults.outlinedButtonBorder else null,
        color = scheme.surface,
    ) {
        Column(
            modifier = Modifier.padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                ColorSwatch(scheme.primary)
                ColorSwatch(scheme.secondary)
                ColorSwatch(scheme.tertiary)
            }
            Text(theme.name, style = MaterialTheme.typography.labelLarge, color = scheme.onSurface)
            if (selected) {
                Text("已应用", style = MaterialTheme.typography.labelSmall, color = scheme.primary)
            }
        }
    }
}

@Composable
private fun ColorSwatch(color: Color, modifier: Modifier = Modifier) {
    Surface(
        modifier = modifier.size(20.dp).clip(RoundedCornerShape(6.dp)),
        color = color,
    ) {}
}

// ============================================================================
// Custom background (task 11)
// ============================================================================

@Composable
private fun BackgroundSettingsSection(
    config: BackgroundConfig,
    nightMode: ThemeNightMode,
    onChange: (BackgroundConfig) -> Unit,
) {
    val strings = LocalRcStrings.current
    val context = LocalContext.current
    var pickError by remember { mutableStateOf(false) }

    // Launch the system picker. `GetContent` returns a `content://` URI whose read
    // grant we persist; we then copy the bytes into our own files dir so the
    // background survives a revoked grant / uninstalled gallery app.
    val pickLauncher = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri: Uri? ->
        if (uri == null) return@rememberLauncherForActivityResult
        runCatching {
            context.contentResolver.takePersistableUriPermission(
                uri, Intent.FLAG_GRANT_READ_URI_PERMISSION,
            )
        }
        val source = BackgroundCache.cacheUri(context, uri) ?: uri.toString()
        if (BackgroundValidator.isValidUri(source)) {
            pickError = false
            onChange(config.copy(uri = source, enabled = true))
        } else {
            pickError = true
        }
    }

    SettingsSection(strings[RcStringKeys.BACKGROUND_TITLE]) {
        SwitchSetting(
            title = strings[RcStringKeys.BACKGROUND_ENABLE],
            subtitle = if (config.uri == null) strings[RcStringKeys.BACKGROUND_NONE_SELECTED] else null,
            checked = config.enabled,
            onCheckedChange = { onChange(config.copy(enabled = it)) },
        )

        Button(onClick = { pickLauncher.launch("image/*") }) {
            Text(strings[RcStringKeys.BACKGROUND_PICK])
        }
        if (pickError) {
            Text(
                strings[RcStringKeys.BACKGROUND_INVALID],
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }

        // Live preview — renders regardless of the master switch so the user can
        // see the effect before turning it on (task 11: "预览").
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(180.dp)
                .clip(RoundedCornerShape(12.dp)),
        ) {
            RcBackground(config = config.copy(enabled = config.uri != null), nightMode = nightMode)
        }

        Text(strings[RcStringKeys.BACKGROUND_EFFECT], style = MaterialTheme.typography.titleMedium)
        val effects = listOf(
            BackgroundEffect.NONE, BackgroundEffect.BLUR,
            BackgroundEffect.DARKEN, BackgroundEffect.BLUR_DARKEN,
        )
        val effectLabel = mapOf(
            BackgroundEffect.NONE to strings[RcStringKeys.BACKGROUND_EFFECT_NONE],
            BackgroundEffect.BLUR to strings[RcStringKeys.BACKGROUND_EFFECT_BLUR],
            BackgroundEffect.DARKEN to strings[RcStringKeys.BACKGROUND_EFFECT_DARKEN],
            BackgroundEffect.BLUR_DARKEN to strings[RcStringKeys.BACKGROUND_EFFECT_BLUR_DARKEN],
        )
        SingleChoiceSegmentedButtonRow {
            for ((index, effect) in effects.withIndex()) {
                SegmentedButton(
                    selected = config.effect == effect,
                    onClick = { onChange(config.copy(effect = effect)) },
                    shape = SegmentedButtonDefaults.itemShape(index = index, count = effects.size),
                ) {
                    Text(effectLabel[effect] ?: effect.name)
                }
            }
        }

        SliderSetting(
            title = strings[RcStringKeys.BACKGROUND_BLUR],
            value = config.blurRadiusDp.toFloat(),
            valueRange = BackgroundLimits.MIN_BLUR_DP.toFloat()..BackgroundLimits.MAX_BLUR_DP.toFloat(),
            steps = (BackgroundLimits.MAX_BLUR_DP - BackgroundLimits.MIN_BLUR_DP) / 4,
            onValueChange = { onChange(config.copy(blurRadiusDp = it.roundToInt())) },
            label = "${config.blurRadiusDp} dp",
            enabled = config.effect.blurs,
        )
        SliderSetting(
            title = strings[RcStringKeys.BACKGROUND_DARKEN],
            value = config.darkenAlpha,
            valueRange = BackgroundLimits.MIN_DARKEN..BackgroundLimits.MAX_DARKEN,
            steps = 19,
            onValueChange = { onChange(config.copy(darkenAlpha = it)) },
            label = "${(config.darkenAlpha * 100).roundToInt()}%",
            enabled = config.effect.darkens,
        )

        SwitchSetting(
            title = strings[RcStringKeys.BACKGROUND_FOLLOW_THEME],
            subtitle = strings[RcStringKeys.BACKGROUND_FOLLOW_THEME_SUMMARY],
            checked = config.followTheme,
            onCheckedChange = { onChange(config.copy(followTheme = it)) },
        )
    }
}

@Preview(name = "Settings portrait", showBackground = true, widthDp = 392, heightDp = 872)
@Composable
private fun SettingsScreenPreview() {
    ProvideRcWindowInfo { SettingsScreen() }
}

/** Task 9: the same centre on a landscape window (capped width, wider padding). */
@Preview(name = "Settings landscape", showBackground = true, widthDp = 872, heightDp = 392)
@Composable
private fun SettingsScreenLandscapePreview() {
    ProvideRcWindowInfo { SettingsScreen() }
}
