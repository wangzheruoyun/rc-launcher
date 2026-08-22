package com.rc.launcher.ui.screen

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.align
import androidx.compose.foundation.layout.IntOffset
import androidx.compose.foundation.layout.IntSize
import androidx.compose.material3.Button
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.rc.launcher.ui.model.ControlElement
import com.rc.launcher.ui.model.JoystickKind
import com.rc.launcher.ui.model.MappedKey
import com.rc.launcher.ui.model.VirtualButton
import com.rc.launcher.ui.model.VirtualJoystick
import com.rc.launcher.ui.viewmodel.ControlLayoutViewModel
import kotlin.math.minOf
import kotlin.math.roundToInt

/**
 * Controller / input-mapping editor (task 15).
 *
 * Lets the user pick a layout (built-in touch / WASD+mouse / gamepad, or any
 * saved custom one), visually lay out virtual buttons and touch joysticks on a
 * scaled "touch surface", edit each element (label, bound keys, size, axis) and
 * save the result as a named custom layout. The whole thing is the Compose
 * counterpart of FCL-Controllers / Zalith control mapping: the same screen that
 * both previews and authors the on-screen controls used by the launch engine.
 *
 * State lives entirely in [ControlLayoutViewModel]; every mutation is
 * copy-on-write + sanitized, so the renderer (task 9) can trust the persisted
 * [com.rc.launcher.ui.model.LauncherSettings.controllerLayoutId].
 */
@Composable
fun ControllerScreen(
    viewModel: ControlLayoutViewModel = viewModel(),
    onBack: () -> Unit = {},
) {
    val layout by viewModel.layout.collectAsStateWithLifecycle()
    val savedLayouts by viewModel.savedLayouts.collectAsStateWithLifecycle()
    val selectedId by viewModel.selectedElementId.collectAsStateWithLifecycle()
    val dirty by viewModel.dirty.collectAsStateWithLifecycle()

    val allLayouts = viewModel.builtInLayouts + savedLayouts

    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // ---- Header --------------------------------------------------------
        Row(verticalAlignment = Alignment.CenterVertically) {
            Button(onClick = onBack) { Text("返回") }
            Spacer(Modifier.width(12.dp))
            Text(
                layout.name + if (dirty) " *" else "",
                style = MaterialTheme.typography.titleLarge,
            )
        }

        // ---- Layout picker -------------------------------------------------
        Surface(
            tonalElevation = 1.dp,
            shape = RoundedCornerShape(16.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("选择布局", style = MaterialTheme.typography.titleMedium)
                LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(allLayouts, key = { it.id }) { meta ->
                        FilterChip(
                            selected = meta.id == layout.id,
                            onClick = { viewModel.loadLayout(meta.id) },
                            label = { Text(meta.name) },
                        )
                    }
                }
            }
        }

        // ---- Touch surface -------------------------------------------------
        Surface(
            tonalElevation = 1.dp,
            shape = RoundedCornerShape(16.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("触控区域（拖动可移动，点击下方按钮新增）", style = MaterialTheme.typography.titleMedium)
                val surfaceSize = remember { mutableStateOf(IntSize.Zero) }
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(360.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(Color.Black.copy(alpha = 0.22f))
                        .align(Alignment.CenterHorizontally)
                        .onSizeChanged { surfaceSize.value = it },
                ) {
                    val surface = surfaceSize.value
                    val minDim = if (surface.width > 0 && surface.height > 0) {
                        minOf(surface.width, surface.height).toFloat()
                    } else 1f
                    for (el in layout.elements) {
                        val cx = (el.x * surface.width).roundToInt()
                        val cy = (el.y * surface.height).roundToInt()
                        val isSel = el.id == selectedId
                        when (el) {
                            is VirtualJoystick -> {
                                val r = (el.radius * minDim / 2).roundToInt().coerceAtLeast(1)
                                Box(
                                    contentAlignment = Alignment.Center,
                                    modifier = Modifier
                                        .align(Alignment.TopStart)
                                        .offset { IntOffset(cx - r, cy - r) }
                                        .size((r * 2).dp)
                                        .clip(CircleShape)
                                        .background(
                                            if (isSel) MaterialTheme.colorScheme.primary.copy(alpha = 0.45f)
                                            else Color(0x33FFFFFF),
                                        )
                                        .clickable { viewModel.selectElement(el.id) }
                                        .pointerInput(el.id) {
                                            var lastX = 0f
                                            var lastY = 0f
                                            detectDragGestures(
                                                onDragStart = {
                                                    viewModel.selectElement(el.id)
                                                    lastX = viewModel.layout.value.elements
                                                        .firstOrNull { it.id == el.id }?.x ?: el.x
                                                    lastY = viewModel.layout.value.elements
                                                        .firstOrNull { it.id == el.id }?.y ?: el.y
                                                },
                                                onDrag = { change, dragAmount ->
                                                    change.consume()
                                                    val nx = (lastX + dragAmount.x / surface.width).coerceIn(0f, 1f)
                                                    val ny = (lastY + dragAmount.y / surface.height).coerceIn(0f, 1f)
                                                    lastX = nx
                                                    lastY = ny
                                                    viewModel.moveElement(el.id, nx, ny)
                                                },
                                            )
                                        },
                                ) {
                                    Text(
                                        if (el.kind == JoystickKind.MOVE) "移动" else "视角",
                                        color = Color.White,
                                        style = MaterialTheme.typography.labelMedium,
                                    )
                                }
                            }
                            is VirtualButton -> {
                                val d = (el.size * minDim).roundToInt().coerceAtLeast(1)
                                Box(
                                    contentAlignment = Alignment.Center,
                                    modifier = Modifier
                                        .align(Alignment.TopStart)
                                        .offset { IntOffset(cx - d / 2, cy - d / 2) }
                                        .size(d.dp)
                                        .clip(CircleShape)
                                        .background(
                                            if (isSel) MaterialTheme.colorScheme.primary
                                            else Color(el.colorArgb),
                                        )
                                        .clickable { viewModel.selectElement(el.id) }
                                        .pointerInput(el.id) {
                                            var lastX = 0f
                                            var lastY = 0f
                                            detectDragGestures(
                                                onDragStart = {
                                                    viewModel.selectElement(el.id)
                                                    lastX = viewModel.layout.value.elements
                                                        .firstOrNull { it.id == el.id }?.x ?: el.x
                                                    lastY = viewModel.layout.value.elements
                                                        .firstOrNull { it.id == el.id }?.y ?: el.y
                                                },
                                                onDrag = { change, dragAmount ->
                                                    change.consume()
                                                    val nx = (lastX + dragAmount.x / surface.width).coerceIn(0f, 1f)
                                                    val ny = (lastY + dragAmount.y / surface.height).coerceIn(0f, 1f)
                                                    lastX = nx
                                                    lastY = ny
                                                    viewModel.moveElement(el.id, nx, ny)
                                                },
                                            )
                                        },
                                ) {
                                    Text(
                                        el.displayLabel,
                                        color = Color.White,
                                        style = MaterialTheme.typography.labelSmall,
                                    )
                                }
                            }
                        }
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(onClick = { viewModel.addButton(0.5f, 0.5f) }) { Text("＋ 按键") }
                    OutlinedButton(onClick = { viewModel.addJoystick(0.5f, 0.5f) }) { Text("＋ 摇杆") }
                }
            }
        }

        // ---- Save / persist -------------------------------------------------
        Surface(
            tonalElevation = 1.dp,
            shape = RoundedCornerShape(16.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text("保存布局", style = MaterialTheme.typography.titleMedium)
                OutlinedTextField(
                    value = layout.name,
                    onValueChange = viewModel::setName,
                    label = { Text("布局名称") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(onClick = { viewModel.saveCurrent() }) { Text("保存") }
                    OutlinedButton(onClick = { viewModel.saveCurrent(name = layout.name + " 副本", asCopy = true) }) { Text("另存为") }
                    OutlinedButton(
                        onClick = viewModel::resetCurrent,
                        enabled = dirty,
                    ) { Text("重置") }
                    OutlinedButton(
                        onClick = viewModel::deleteCurrent,
                        enabled = layout.editable,
                    ) { Text("删除") }
                }
                Text(
                    if (layout.editable) "当前为自定义布局，可直接覆盖保存。" else "当前为内置布局，保存将另存为新布局。",
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }

        // ---- Element editor -------------------------------------------------
        selectedElementEditor(viewModel, layout.elements.firstOrNull { it.id == selectedId })
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun selectedElementEditor(
    viewModel: ControlLayoutViewModel,
    selected: ControlElement?,
) {
    if (selected == null) {
        Surface(
            tonalElevation = 1.dp,
            shape = RoundedCornerShape(16.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Text(
                "未选择元素。点击触控区域中的按键或摇杆进行编辑。",
                modifier = Modifier.padding(16.dp),
                style = MaterialTheme.typography.bodyMedium,
            )
        }
        return
    }

    Surface(
        tonalElevation = 1.dp,
        shape = RoundedCornerShape(16.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("编辑元素", style = MaterialTheme.typography.titleMedium)
            when (selected) {
                is VirtualButton -> buttonEditor(viewModel, selected)
                is VirtualJoystick -> joystickEditor(viewModel, selected)
            }
            HorizontalDivider()
            Button(
                onClick = { viewModel.removeElement(selected.id) },
                modifier = Modifier.fillMaxWidth(),
            ) { Text("删除该元素") }
        }
    }
}

@Composable
private fun buttonEditor(viewModel: ControlLayoutViewModel, btn: VirtualButton) {
    OutlinedTextField(
        value = btn.label,
        onValueChange = { viewModel.updateButton(btn.id, it, btn.keys, btn.size) },
        label = { Text("显示文字") },
        singleLine = true,
        modifier = Modifier.fillMaxWidth(),
    )
    Text("大小：${(btn.size * 100).roundToInt()}%")
    Slider(
        value = btn.size,
        onValueChange = { viewModel.updateButton(btn.id, btn.label, btn.keys, it) },
        valueRange = VirtualButton.MIN_SIZE..VirtualButton.MAX_SIZE,
    )
    Text("绑定按键（可多选）", style = MaterialTheme.typography.titleSmall)
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        maxItemsInEachRow = 6,
    ) {
        for (key in MappedKey.ALL) {
            val on = btn.keys.contains(key)
            FilterChip(
                selected = on,
                onClick = {
                    val next = if (on) btn.keys - key else btn.keys + key
                    viewModel.updateButton(btn.id, btn.label, next, btn.size)
                },
                label = { Text(key.label) },
            )
        }
    }
}

@Composable
private fun joystickEditor(viewModel: ControlLayoutViewModel, js: VirtualJoystick) {
    Text("半径：${(js.radius * 100).roundToInt()}%")
    Slider(
        value = js.radius,
        onValueChange = { viewModel.updateJoystick(js.id, it, js.kind) },
        valueRange = VirtualJoystick.MIN_RADIUS..VirtualJoystick.MAX_RADIUS,
    )
    Text("驱动轴", style = MaterialTheme.typography.titleSmall)
    val kinds = listOf(JoystickKind.MOVE, JoystickKind.LOOK)
    SingleChoiceSegmentedButtonRow {
        kinds.forEachIndexed { index, kind ->
            SegmentedButton(
                selected = js.kind == kind,
                onClick = { viewModel.updateJoystick(js.id, js.radius, kind) },
                shape = SegmentedButtonDefaults.itemShape(index, kinds.size),
            ) { Text(kind.label) }
        }
    }
}
