# 屏幕方向自适应与横竖屏切换（Task 9）

玩家反馈：**「希望支持横屏和竖屏切换」**。这件事看起来只是一个 `screenOrientation`
属性，实际牵扯三层：Activity 能不能转、界面转完之后长什么样、以及**游戏画面转的时候
触摸坐标会不会错位**。本文档说明三层各自的实现与它们之间的契约。

```
┌───────────────────────────── Rust core (display) ─────────────────────────────┐
│ OrientationPolicy  跟随系统 / 强制横屏 / 强制竖屏                              │
│   ├─ android_screen_orientation() → user / sensorLandscape / sensorPortrait    │
│   └─ orient(WindowSize)           → 强制方向下的游戏窗口分辨率                 │
│ WindowMetrics(width_dp, height_dp)                                            │
│   → orientation / width_class / height_class / navigation_rail /              │
│     instance_columns / settings_columns / dashboard_columns / padding …        │
│ rotation_flips(from, to)   判定「真正的 90° 旋转」                             │
└───────────────┬──────────────────────────────────────────┬────────────────────┘
                │ 手写镜像（金标准 fixture 校验）           │ JNI（诊断 / 校验用）
┌───────────────▼───────────────────┐      ┌───────────────▼────────────────────┐
│ ui/AdaptiveLayout.kt              │      │ RustBridge.displayOrientations()   │
│  RcWindowInfo / RcSizeClass       │      │ RustBridge.displayLayout(json)     │
│  ScreenOrientation / rotationFlips│      └────────────────────────────────────┘
└───────────────┬───────────────────┘
                │ LocalRcWindowInfo（RcApp 根部一次测量）
     MainScreen / HomeScreen / InstancesScreen / SettingsScreen
```

## 1. Activity 层：允许旋转，且**不重建**

`AndroidManifest.xml`：

| 属性 | 值 | 为什么 |
|---|---|---|
| `screenOrientation` | `user` | 允许旋转，同时尊重系统的「旋转锁定」（`unspecified` 会无视锁定）。 |
| `configChanges` | `orientation\|screenSize\|screenLayout\|smallestScreenSize\|density\|uiMode\|…` | 由 Activity 自己处理几何变化：**旋转不重建 Activity**，界面状态不丢、游戏/AWT Surface 的坐标系不重建。 |
| `resizeableActivity` | `true` | 分屏 / 自由窗口与旋转走同一套 WindowSizeClass 逻辑。 |

`MainActivity.applyOrientation()` 把用户选择映射为 `requestedOrientation`：

| 设置项（`OrientationMode`） | id | Android 常量 |
|---|---|---|
| 跟随系统 | `system` | `SCREEN_ORIENTATION_USER` |
| 强制横屏 | `landscape` | `SCREEN_ORIENTATION_SENSOR_LANDSCAPE` |
| 强制竖屏 | `portrait` | `SCREEN_ORIENTATION_SENSOR_PORTRAIT` |

`sensor*` 而不是 `landscape/portrait`：锁定轴向但保留两个方向，平板横放在支架上、手机
反向持握时画面仍然是正的。设置改变时通过 `settingsViewModel.settings.collect { … }`
立即生效，无需重启界面。

## 2. Compose 层：按窗口尺寸等级重新布局

`RcApp` 在根部用 `BoxWithConstraints` 测量一次，并以 `LocalRcWindowInfo` 下发
（`ProvideRcWindowInfo`）。选择 `BoxWithConstraints` 而非 `LocalConfiguration`：约束就是
真正用于布局的尺寸，分屏 / 折叠屏 / 自由窗口自动正确，且不依赖跨版本变动的 API。

断点与决策（与 Rust `display` 完全一致）：

| 轴 | Compact | Medium | Expanded |
|---|---|---|---|
| 宽 | `< 600dp` | `600–839dp` | `>= 840dp` |
| 高 | `< 480dp` | `480–899dp` | `>= 900dp` |

| 决策 | 规则 |
|---|---|
| 侧边导航栏（`usesNavigationRail`） | 横屏**或**宽度非 Compact。横屏下底部导航栏既吃掉本就紧张的高度，又正好压在握持的拇指下。 |
| 实例栅格列数 | Compact：竖屏 1 / 横屏 2；Medium：2；Expanded：3。 |
| 设置分栏 | Compact 1 列；Medium 横屏 2 列；Expanded 2 列（设置行很宽，窄屏强行两列会挤）。 |
| 首页概览 | 竖屏 Compact 单列；其余「问候 + 资源占用」并排。 |
| 内边距 / 最大文本宽度 | 16/20/24dp；Medium 720dp、Expanded 1080dp 上限，平板上不会拉出超长一行。 |
| 短窗口（`isShort`） | 横屏手机隐藏「最近游玩」横向导轨与说明文案，把高度让给卡片；侧边栏隐藏文字标签。 |

设置中心的「屏幕与方向」分区同时显示**实时几何信息**（`872 x 392 dp · 横屏 ·
宽度 expanded / 高度 compact · 侧边导航栏 · 实例 3 列`），让断点行为可观测，也方便玩家
反馈问题时截图。

## 3. 输入坐标：旋转不能让触摸「错位」

游戏内嵌 AWT/Swing 画面（见 [awt.md](awt.md)）的触摸事件以**Surface 像素**批量提交，
由核心按当前 viewport 映射成**桌面像素**。于是有两个真实的坐标错位来源：

1. **顺序问题**：旋转发生在「采样」与「提交」之间时，旧坐标会被新的黑边（letterbox）
   映射一次。同一个手指位置在 1200x900 上是桌面 `(320,240)`，在 900x1200 上是
   `(426,133)` —— 151px 的偏差。
   *解法*：`AwtSurfaceViewModel.onSurfaceSizeChanged()` 先 `flushInput()`，**再**下发新
   几何，保证每个采样都用采样时的 viewport 映射。
2. **手势跨旋转存活**：转屏后手指的物理位置已经不同，继续原来的拖拽/按下毫无意义，
   还可能让按键卡住。
   *解法*：`rotationFlips()` 判定为真正的 90° 旋转时，Kotlin 侧 `releaseAll()`、核心侧
   `AwtSession::set_surface_size()` 释放全部按下状态并计入 `surface_rotations`；
   `pointerInput` 以 Surface 方向为 key，Compose 侧的手势检测器也随之重启。
   软键盘弹出、分屏拖动等**同方向**尺寸变化不会误伤正在进行的拖拽。

可执行的复现 / 验证：

```bash
cd rust && cargo run --example rotation_demo
#   flush-then-rotate  -> [(320, 240)] (correct)
#   rotate-then-flush  -> [(426, 133)] (wrong: 151 px off)
#   rotation released the gesture: true …
```

## 4. 强制方向也会带上游戏窗口

`LaunchOptions.orientation`（序列化形式与设置项 id 一致：`system` / `landscape` /
`portrait`）经 `effective_window()` 作用于所有交给 JVM 的几何：`--width/--height`、
`${resolution_width}`、`-Dcacio.managed.screensize`。因此强制竖屏时游戏拿到的是
`720x1280`，而不是一个被上下黑边夹住的横向窗口。

## 5. 双实现的一致性保障

界面每次重组都要解析布局，不可能为此走一次 JNI，所以 Rust 与 Kotlin 各有一份实现。
两者靠「金标准 fixture + 校验脚本」锁死：

```bash
cargo run --example display_layout_golden -- --write   # 由 Rust 生成 fixture
python3 scripts/check_layout_parity.py                 # CI 门禁（78 项检查）
```

* `app/src/test/resources/display_layout_golden.tsv` —— 39 组窗口几何（真机尺寸、
  每个断点边界、两个方向、退化尺寸）× 13 项决策；
* `app/src/test/resources/display_orientation_golden.tsv` —— 方向策略表；
* `AdaptiveLayoutParityTest`（Kotlin）重放两张表；
* `check_layout_parity.py` 还会核对断点常量、设置 id、`ActivityInfo` 常量、Manifest 的
  `configChanges`、FFI 声明与「旋转释放手势」的实现是否都在位。

相关单元测试：Rust `display::tests`（16）、`launch::fakefx::tests`（旋转 4 项）、
`launch::options/command::tests`（强制方向 5 项）；Kotlin `AdaptiveLayoutTest`、
`AdaptiveLayoutParityTest`、`OrientationModeTest`、`AwtSurfaceViewModelTest`（旋转 5 项）、
`AwtGeometryTest`（旋转 3 项）。
