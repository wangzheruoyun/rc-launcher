package com.rc.launcher.ui.screen

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.awt.AwtControlWire
import com.rc.launcher.ui.awt.AwtCursorKind
import com.rc.launcher.ui.awt.AwtMouseButton
import com.rc.launcher.ui.awt.AwtScaleMode
import com.rc.launcher.ui.component.AwtCanvasSurface
import com.rc.launcher.ui.i18n.rcFps
import com.rc.launcher.ui.viewmodel.AwtSurfaceViewModel

/**
 * AWT / Swing compatibility screen (task 18, "fakefx").
 *
 * Minecraft's *own* window is drawn by LWJGL + GL4ES/ANGLE (task 17); everything
 * built on the desktop toolkit — the Forge / OptiFine installers, the Mojang
 * splash, `JOptionPane` crash dialogs, font metrics — goes through AWT, which
 * Android does not have. caciocavallo renders those windows into an off-screen
 * ARGB desktop inside the game JVM; this screen is where that desktop becomes
 * visible and touchable inside Compose.
 *
 * It doubles as the diagnostics panel for the bridge: the link state, the frame /
 * event counters, the measured fps and the named-pipe channel paths, plus a
 * self-test that pushes a locally generated pattern through the whole pipeline
 * (wire format → validation → canvas → direct buffer → bitmap) so the layer can
 * be verified on a device without launching a game.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun AwtScreen(
    viewModel: AwtSurfaceViewModel = viewModel(),
    onBack: () -> Unit = {},
) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    var rightClickMode by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = onBack) { Text("返回") }
            Spacer(Modifier.width(8.dp))
            Text("AWT / Swing 兼容层", style = MaterialTheme.typography.titleLarge)
        }
        Text(
            "游戏内嵌的 AWT/Swing 界面（安装器、崩溃对话框、字体度量）由 caciocavallo " +
                "渲染到离屏 ARGB 桌面，再经 Rust 核心的帧通道投送到下方画布；触摸与按键会被" +
                "翻译成 java.awt.event 事件回传给 JVM。",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        // ---- The canvas ------------------------------------------------------
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(16f / 9f),
        ) {
            AwtCanvasSurface(
                viewModel = viewModel,
                modifier = Modifier.fillMaxSize(),
                touchButton = if (rightClickMode) AwtMouseButton.RIGHT else AwtMouseButton.LEFT,
            )
        }

        // ---- Session controls ------------------------------------------------
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (state.open) {
                Button(onClick = { viewModel.close() }) { Text("关闭会话") }
            } else {
                Button(onClick = { viewModel.open() }) { Text("开启会话") }
            }
            OutlinedButton(
                onClick = { viewModel.submitTestPattern() },
                enabled = state.open,
            ) { Text("自检帧") }
            OutlinedButton(
                onClick = { viewModel.repaint() },
                enabled = state.open,
            ) { Text("清屏重绘") }
            OutlinedButton(
                onClick = { viewModel.releaseAll() },
                enabled = state.open,
            ) { Text("释放按键") }
            OutlinedButton(
                onClick = {
                    // Exercise the whole control plane the way a real cacio peer
                    // would: hover a text field, name the window, ask for input,
                    // copy something, then ask to paste. The canvas answers the
                    // paste from the Android clipboard on the next frame.
                    val cx = state.info.screenWidth / 2
                    val cy = state.info.screenHeight / 2
                    listOf(
                        AwtControlWire.encodeCursor(AwtCursorKind.TEXT),
                        AwtControlWire.encode(
                            com.rc.launcher.ui.awt.AwtControlKind.WINDOW_OPENED,
                            a = 1,
                            text = "自检对话框",
                        ),
                        AwtControlWire.encodeImeShow(cx, cy, 18),
                        AwtControlWire.encodeClipboardSet("RC Launcher AWT 自检"),
                        AwtControlWire.encodeClipboardRequest(1),
                    ).forEach { viewModel.submitControl(it) }
                },
                enabled = state.open,
            ) { Text("控制面自检") }
            OutlinedButton(
                onClick = { viewModel.resetControl() },
                enabled = state.open,
            ) { Text("复位控制面") }
            OutlinedButton(onClick = { viewModel.refresh() }) { Text("刷新状态") }
        }

        Row(verticalAlignment = Alignment.CenterVertically) {
            Switch(checked = rightClickMode, onCheckedChange = { rightClickMode = it })
            Spacer(Modifier.width(12.dp))
            Text("右键模式（触摸映射为 BUTTON3）", style = MaterialTheme.typography.bodyMedium)
        }

        // ---- Fitting policy --------------------------------------------------
        Text("缩放方式", style = MaterialTheme.typography.titleMedium)
        SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
            val modes = AwtScaleMode.values()
            modes.forEachIndexed { index, mode ->
                SegmentedButton(
                    selected = state.info.scaleMode == mode,
                    onClick = { viewModel.setScaleMode(mode) },
                    shape = SegmentedButtonDefaults.itemShape(index = index, count = modes.size),
                    enabled = state.open,
                ) {
                    Text(mode.label, style = MaterialTheme.typography.labelMedium)
                }
            }
        }

        // ---- Virtual desktop size -------------------------------------------
        Text("虚拟桌面分辨率", style = MaterialTheme.typography.titleMedium)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            for ((width, height) in DESKTOP_PRESETS) {
                FilterChip(
                    selected = state.info.screenWidth == width && state.info.screenHeight == height,
                    onClick = {
                        if (state.open) viewModel.resizeDesktop(width, height)
                        else viewModel.open(width, height)
                    },
                    label = { Text("${width}x$height") },
                )
            }
        }

        HorizontalDivider()

        // ---- Diagnostics -----------------------------------------------------
        Text("桥接状态", style = MaterialTheme.typography.titleMedium)
        Card(modifier = Modifier.fillMaxWidth()) {
            Column(
                modifier = Modifier.padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                InfoRow("后端", if (state.open) state.info.backend else "未开启")
                InfoRow("连接", state.info.link.label + (state.info.link.reason?.let { " · $it" } ?: ""))
                InfoRow(
                    "桌面 / 画面",
                    "${state.info.screenWidth}x${state.info.screenHeight} → " +
                        "${state.surfaceWidth}x${state.surfaceHeight}",
                )
                InfoRow(
                    "绘制区域",
                    with(state.placement) { "${width}x$height @ ($x, $y)" },
                )
                InfoRow(
                    "帧",
                    "接收 ${state.info.framesAccepted} · 丢弃 ${state.info.framesRejected} · " +
                        "上屏 ${state.uploads} · 跳过 ${state.skipped}",
                )
                // Through the catalogue (task 20): the decimal separator and the
                // "FPS" wording are translatable instead of a hardcoded %.1f.
                InfoRow("帧率", rcFps(state.info.fps.toDouble()))
                InfoRow(
                    "事件",
                    "待发 ${state.info.pendingEvents} · 已发 ${state.info.link.eventsWritten} · " +
                        "丢失 ${state.info.link.eventsLost}",
                )
                InfoRow("焦点", if (state.info.focused) "已获得" else "已失去")
                InfoRow(
                    "指针形状",
                    "${state.cursor.label}（${state.cursor.id} / AWT ${state.cursor.awtType}）",
                )
                InfoRow("窗口标题", state.title ?: "—")
                InfoRow(
                    "软键盘",
                    if (state.wantsKeyboard) {
                        val caret = state.control.caret
                        "文本组件请求输入" + (caret?.let { "（(${it.x}, ${it.y})）" } ?: "")
                    } else {
                        "无请求"
                    },
                )
                InfoRow(
                    "剪贴板",
                    "待应答 ${state.control.clipboardRequests} · " +
                        "JVM 复制 ${state.control.clipboardOut ?: "—"}",
                )
                InfoRow(
                    "控制消息",
                    "累计 ${state.controlMessages} · 窗口 ${state.control.windows.size} · " +
                        "提示音 ${state.control.beeps}",
                )
                state.control.bye?.let { InfoRow("桥接结束", it) }
                state.info.framesChannel?.let { InfoRow("帧通道", it) }
                state.info.eventsChannel?.let { InfoRow("事件通道", it) }
                state.lastInput.rejected.take(3).forEach { InfoRow("被拒事件", it) }
                state.message?.let {
                    Spacer(Modifier.height(4.dp))
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                    TextButton(onClick = { viewModel.clearMessage() }) { Text("清除提示") }
                }
            }
        }
    }
}

@Composable
private fun InfoRow(label: String, value: String) {
    Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(
            label,
            style = MaterialTheme.typography.labelLarge,
            modifier = Modifier.fillMaxWidth(0.32f),
        )
        Text(value, style = MaterialTheme.typography.bodySmall)
    }
}

/** Desktop resolutions offered by the screen (`-Dcacio.managed.screensize`). */
private val DESKTOP_PRESETS = listOf(
    854 to 480,
    1280 to 720,
    1920 to 1080,
)
