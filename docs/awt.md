# AWT / Swing 兼容层（Task 18）—— fakefx

Minecraft 的游戏画面由 LWJGL + GL4ES/ANGLE 渲染（见 [rendering.md](rendering.md)），
但**围绕**它的那一大堆界面走的是桌面 AWT/Swing：

| 场景 | 触发的 AWT 代码 |
|---|---|
| Forge / OptiFine / Fabric 安装器 | `JFrame` / `JOptionPane` / `JProgressBar` |
| Mojang 启动画面（1.6~1.12） | `java.awt.Frame` + `BufferedImage` |
| 崩溃与错误对话框 | `JOptionPane.showMessageDialog` |
| 字体度量、皮肤处理 | `Font.getStringBounds`、`BufferedImage`、`ImageIO` |

Android 上没有 X11、没有窗口管理器，`libawt_xawt.so` 也只是一个「假 X11」壳：
一旦这些代码碰到 `java.awt.Toolkit`，JVM 就会以
`java.awt.AWTError: Can't connect to X11 window server` 直接死掉。

FCL 的解法是 **caciocavallo**（`FCLCore/fakefx` + `caciocavallo` /
`caciocavallo17` 两套 jar）：重新实现 AWT 的 *peer* 层，把每个窗口画进一张离屏
ARGB 图像，而不是真实窗口。本项目把这条链路完整地做成了 Rust 核心 + Compose UI：

```text
 ┌──────────── 游戏 JVM（Android）─────────────┐        ┌───── Rust 核心 ─────┐
 │  Swing / AWT  →  caciocavallo CTC peers     │        │  AwtFrame::decode   │
 │        ↓ 画进 int[] ARGB 屏幕               │ frames │        ↓            │
 │  cacio「managed screen」 → awt bridge       ├───────▶│  AwtCanvas（双缓冲  │
 │        ▲ 注入 AWT 事件                      │◀───────┤  + damage 跟踪）    │
 └────────┼────────────────────────────────────┘ events │        ↓            │
          │                                             │  RGBA8888 直写      │
          └──── AwtEventRecord（32 字节定长）────────────┤  ByteBuffer         │
                                                        └────────┬────────────┘
                                                                 │ awtPollFrame
                                                        ┌────────▼────────────┐
                                                        │ Compose：Bitmap +    │
                                                        │ drawImage（信箱化）  │
                                                        └─────────────────────┘
```

## 1. 代码地图

| 层 | 位置 | 职责 |
|---|---|---|
| 兼容层要件 | `rust/crates/rc-launcher-core/src/launch/awt.rs` | 后端选择、cacio jar/原生库校验、JVM 参数、帧/事件 wire 格式、`AwtCanvas`、`Viewport`、输入翻译 |
| 会话运行时 | `.../launch/fakefx.rs` | `AwtSession`：把上面的零件组合成一个可被 FFI/UI 驱动的对象；帧流读取与事件写出 |
| 会话宿主 | `.../launch/awt_host.rs` | `AwtHost`：命名管道通道的创建与两个 pump 线程、链路计数与状态 |
| FFI | `.../ffi.rs`（task 18 段） | `awtOpen/awtClose/awtInfo/awtConfigure/awtAttachTransport/awtInput/awtSubmitFrame/awtPollFrame/awtDrainEvents` |
| Kotlin 领域层 | `app/src/main/java/com/rc/launcher/ui/awt/` | 视口几何、wire 编解码、输入事件、快照解析、桥接接口（Rust / Fake 两实现） |
| Compose | `ui/component/AwtCanvasSurface.kt`、`ui/screen/AwtScreen.kt`、`ui/viewmodel/AwtSurfaceViewModel.kt` | 画布、诊断面板、状态容器 |

> **关于「fakefx」的命名**：FCL 的 `fakefx` 包其实是两件事——(1) 重新实现 JavaFX
> 的属性/绑定 API（因为 FCL 源自桌面 JavaFX 启动器 HMCL），(2) AWT-on-Android 适配。
> 我们的 UI 是 Compose + `StateFlow`，**它本身就是**那层可观察属性，所以 JavaFX
> 那一半有意不移植；移植的是 AWT/Swing 这一半，以及把它显示到 Compose 的画布。

## 2. 后端选择与 JVM 参数

`AwtBackend`（`awt.rs`）按 Java 版本选择，与 FCL 完全一致：

| Java | 后端 | 目录 | Toolkit |
|---|---|---|---|
| 8 | `Cacio8` | `app_runtime/caciocavallo/` | `net.java.openjdk.cacio.ctc.CTCToolkit` |
| 9+ | `Cacio17` | `app_runtime/caciocavallo17/` | `com.github.caciocavallosilano.cacio.ctc.CTCToolkit` |
| —  | `Headless` | 无 | `-Djava.awt.headless=true`（1.13+ 原版够用，最省最稳） |

`AwtBridge::jvm_args` 产出的关键参数：

* `-Djava.awt.headless=false`、`-Dcacio.managed.screensize=<W>x<H>`（虚拟桌面尺寸）
* `-Dawt.toolkit=…`、`-Djava.awt.graphicsenv=…`、`-Dswing.defaultlaf=…MetalLookAndFeel`
* `-Dsun.java2d.opengl=false`（Java2D 必须留在软件管线；GL 翻译层属于 LWJGL）
* Java 8：`-Dcacio.font.fontmanager/fontscaler`、`-Xbootclasspath/p:ResConfHack.jar`
* Java 17+：`CACIO17_MODULE_FLAGS`（`--add-exports/--add-opens` 打开 `java.desktop` 内部）
  与 `-javaagent:cacio-agent.jar`
* 存在实时会话时：`-Drc.awt.bridge.protocol=rcaf1`、`-Drc.awt.bridge.frames=…`、
  `-Drc.awt.bridge.events=…`

缺失**可选** jar/原生库只会进 `notes`（UI 提示），缺失**必需** jar 会在 preflight
阶段直接失败——绝不让 JVM 起来后死在第一个 `Toolkit.getDefaultToolkit()`。

## 3. 传输：两条命名管道

```text
frames : JVM  --[AwtFrame]-------->  启动器   （AwtSession::submit_frame）
events : JVM  <--[AwtEventRecord]--  启动器   （AwtSession::drain_events）
```

* 路径由 `AwtTransport::in_dir(dir)` 约定：`awt-frames.rcaf` / `awt-events.rcae`。
* `awt_host::create_channels` 用 `mkfifo(0600)` 创建；**旧文件先删除**——上次崩溃留下的
  普通文件会把「实时链路」悄悄变成一个不断增长的日志文件。
* 打开动作发生在 pump 线程里：FIFO 的 open 会阻塞等待对端，而启动器广告路径时 JVM
  还没起来。读端用 `O_RDONLY|O_NONBLOCK` 打开后再清掉 `O_NONBLOCK`（读回到阻塞，
  保持帧对齐）；写端在 `ENXIO`（还没有读者）时按 `poll_interval` 重试。
* 帧 pump 线程用 `poll(2)` 等待可读，因此**空闲时也能在 ≤50ms 内看到 stop 标志**，
  `AwtHost::stop()` 永不挂死，`Drop` 也不会泄漏一个仍在写死会话的线程。
* 一个会话只允许一条传输：重复 `attach_transport` 会被拒绝（否则会在运行中的 pump 脚下
  重建 FIFO，并且每次调用泄漏一对线程）。换会话请先 `awtClose`。

### 帧格式（`"RCAF"`，小端）

```text
0  u32 magic("RCAF")  4  u16 version  6  u16 format(0=ARGB,1=RGB)
8  u32 seq           12  u16 width   14  u16 height
16 u16 damage.x      18  u16 damage.y 20 u16 damage.w  22 u16 damage.h
24 u32 payload_len(字节)              28 u32 flags(bit0=整屏)
32 …  damage.w*damage.h 个 u32 像素（行主序）
```

**只传脏矩形**：Swing 光标闪一下就只有几十个像素，而不是一整张 1280×720。

### 事件格式（32 字节定长 × N）

`id, x, y, button, key_code, key_char, modifiers, wheel` 共 8 个 `i32`。`id` 直接就是
`java.awt.event.*Event` 的常量（`MOUSE_PRESSED=501`、`KEY_PRESSED=401`…），JVM 侧
`new MouseEvent(id, …)` 即可，无需翻译表。

## 4. 画布与上屏（零拷贝）

* `AwtCanvas` 是**双缓冲 + damage 合并**的 ARGB 表面：生产者（JVM）写后缓冲并
  `present`，消费者（Compose）读前缓冲。`present` 里才切换缓冲，所以既不阻塞生产者也
  不会让消费者看到半帧。
* `AwtSession::poll_frame_into(dst)` 把**变化的行**转成 RGBA8888 写进 `dst`；
  `dst` 就是 Compose `Bitmap` 背后的 **direct `ByteBuffer`**，`awtPollFrame` 通过
  `GetDirectBufferAddress` 拿到裸指针写入 —— 像素**完全不经过 Java 数组**。
* 没有变化时返回 `{"changed":false}`，UI 连 `copyPixelsFromBuffer` 和重组都跳过。
* 信箱化（`ScaleMode`：`stretch/fit/fill_crop/center`）在 Rust 与 Kotlin 两侧用
  **同一套整数算法**（`Viewport` / `AwtViewport`），所以「画在哪」和「核心以为点到哪」
  不会漂移，也不可能有 `NaN` 混进 blit。

## 5. 输入

Compose 手势 → `AwtInputEvent`（Kotlin，`toBatchJson()`）→ `awtInput` →
`AwtInputTranslator`（Rust，持有按键/修饰键状态）→ `AwtEventRecord`：

* 一帧一批：拖动只花 **1 次** JNI 调用；按下/抬起/按键/文本立刻 flush（低频、对延迟敏感）。
* 按住按钮移动 = `MOUSE_DRAGGED`（Swing 滚动条、文本选择依赖它），否则 `MOUSE_MOVED`。
* 抬起时若位移 ≤ `click_slop` 会补一条 `MOUSE_CLICKED`（Swing 按钮响应的是 `mouseClicked`，
  而手指总会抖）。
* 点在黑边（信箱区）上**不产生**事件；但拖动中手指移出画面会 clamp，滚动条不会掉。
* 失焦会释放所有按下的键/按钮——切后台后不会留一个「按住的 Shift」。
* 硬件键盘/软键盘：`awtKeyNameForAndroidKeyCode` 把 Android keycode 翻成核心认识的
  键名（与 task 15 控制布局同一套命名），认不出的键退化成 `KEY_TYPED` 文本（IME 同理）。

## 6. 健壮性（task 19）

| 风险 | 处理 |
|---|---|
| 损坏/恶意帧头 | `AwtFrame::decode` 校验 magic/版本/格式/尺寸/damage/payload 长度，返回 `RcError` 而非 panic；**分配前**先检查声明长度 |
| 单帧非法 | `FrameRead::Rejected`：计数并继续（流仍对齐），cacio 下一次重绘即可恢复 |
| 流失步/截断 | 结束链路并给出**原因字符串**，UI 可显示「AWT 桥接已断开：…」 |
| JVM 不再读事件 | 出队有上限，优先丢**运动类**记录（`MOUSE_MOVED/DRAGGED/WHEEL`），状态类记录保留 |
| UI 不再消费帧 | `frames_dropped` 计数，永不无界缓冲 |
| 会话锁被 poison | `into_inner()` 恢复：像素可能旧一帧，UI 继续跑 |
| 缺少 `librc_launcher.so` | Kotlin 侧全部 `runCatching`，降级为可见的错误提示，UI 不崩 |
| 尺寸非法 | `AwtSessionConfig::sanitized()` 把 0 / 999999 夹到 `1..8192`，杜绝「分配数 GB」 |

## 7. 接口速查

Kotlin ⇄ Rust（`RustBridge`，全部 JSON 出入，除像素/事件的二进制通道）：

| 方法 | 作用 |
|---|---|
| `awtOpen(configJson)` | 开启/替换会话（`screen`/`surface`/`scale_mode`/`click_slop`/`java_version`/`transport`） |
| `awtClose()` | 关闭会话并 join pump 线程 |
| `awtInfo()` | 会话 + 链路快照（诊断面板、HUD） |
| `awtConfigure(json)` | 画面/桌面尺寸、缩放方式、焦点、清屏/填充 |
| `awtAttachTransport(json)` | 为已开启的会话创建并 pump 命名管道 |
| `awtInput(json)` | 一帧的输入批次，返回 `queued/pending/modifiers/focused/pointer/rejected` |
| `awtSubmitFrame(bytes)` | 手工投一帧（自检 / Kotlin 侧传输） |
| `awtPollFrame(directBuffer)` | **热路径**：脏区域直写 direct ByteBuffer（零拷贝） |
| `awtPollFrameArray(byteArray)` | 无 direct buffer 时的兜底（各多一次拷贝） |
| `awtDrainEvents()` | 取出待发的 32 字节记录（Kotlin 侧传输时使用） |

启动侧串联：`LaunchOptions.awt_transport_dir = <dir>` 时，`CommandBuilder` 会把
`rc.awt.bridge.*` 三个属性交给 JVM；不设置则 cacio 依旧离屏渲染（对话框能用，只是
不显示）——**永远不会**出现「FIFO 没有读者把游戏第一次重绘卡死」的情况。

## 8. 测试

```bash
cd rust && cargo test --workspace     # awt / fakefx / awt_host / ffi::awt_tests
cargo run --example awt_demo          # 端到端：真实命名管道 + 假 JVM + 假 UI
```

`examples/awt_demo.rs` 在**没有 Android、没有 JVM**的情况下跑完整条链路：假 JVM 线程
经真实 FIFO 发一整屏重绘 → 一个损坏帧 → 一个 8×4 脏矩形补丁，UI 线程按帧 poll 到
framebuffer 并注入一次点击 + 一次 Esc。输出会显示脏区域上屏只花 **128 B / 8192 B（1%）**、
损坏帧被计数但链路存活，以及 JVM 侧收到 `MOUSE_PRESSED/RELEASED/CLICKED` 与
`KEY_PRESSED(VK_ESCAPE)`（坐标 (32,16) 正是信箱化后的桌面中心）。

* `launch::awt`、`launch::fakefx`：wire 格式、canvas/damage、视口映射、输入翻译、限流。
* `launch::awt_host`（17 项）：真实 `UnixStream` 与**真实 FIFO** 的端到端往返、
  坏帧不致命、EOF/失步的原因字符串、`stop_and_join` 不等待 JVM、`Drop` 停止 pump、
  重复 attach 被拒绝（不泄漏线程）、通道路径不可创建时报错而非 panic。
* `ffi::awt_tests`（13 项）：JSON 控制面（开/关/配置/输入批次/坏事件）、像素往返、
  短缓冲被拒后 damage 仍在、`transport` 目录真的生成两个 FIFO，以及**跨语言契约测试**
  `the_snapshot_carries_every_field_the_compose_layer_parses`（快照/poll/input 的每个字段
  都是 Kotlin `AwtSessionInfo.kt` 会读的键——CI 跑不了 Kotlin 单测，这一项就是护栏）。
* Kotlin：`app/src/test/java/com/rc/launcher/ui/awt/*`（视口与 Rust 逐位一致、wire 编解码、
  输入 JSON、快照解析、keycode 映射）与
  `ui/viewmodel/AwtSurfaceViewModelTest`（用 `FakeAwtCanvasBridge` 走完整 UI 路径）。
