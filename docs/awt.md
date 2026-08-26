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
| 兼容层要件 | `rust/crates/rc-launcher-core/src/launch/awt.rs` | 后端选择、cacio jar/原生库校验、JVM 参数、帧/事件/**控制** wire 格式、`AwtCanvas`、`Viewport`、输入翻译、`CursorKind`/`AwtControl`/控制应答分片 |
| 会话运行时 | `.../launch/fakefx.rs` | `AwtSession`：把上面的零件组合成一个可被 FFI/UI 驱动的对象；帧流读取与事件写出 |
| 会话宿主 | `.../launch/awt_host.rs` | `AwtHost`：命名管道通道的创建与两个 pump 线程、链路计数与状态 |
| FFI | `.../ffi.rs`（task 18 段） | `awtOpen/awtClose/awtInfo/awtConfigure/awtAttachTransport/awtInput/awtSubmitFrame/awtPollFrame/awtDrainEvents` |
| Kotlin 领域层 | `app/src/main/java/com/rc/launcher/ui/awt/` | 视口几何、wire 编解码、输入事件、快照解析、**控制面**（`AwtControl.kt`：光标/标题/剪贴板/IME + `RCAC` 编解码 + 应答重组）、桥接接口（Rust / Fake 两实现） |
| 跨语言门禁 | `scripts/check_awt_wire.py` | 静态比对 Rust 与 Kotlin 的 magic/版本/kind 码/光标号/事件 id/JSON 键——**无需 Rust 工具链与 JVM** |
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
frames : JVM  --[AwtFrame  ]------->  启动器   （AwtSession::submit_frame）
frames : JVM  --[AwtControl]------->  启动器   （AwtSession::submit_control）
events : JVM  <--[AwtEventRecord]--   启动器   （AwtSession::drain_events）
events : JVM  <--[控制应答记录  ]--   启动器   （AwtSession::answer_clipboard）
```

**一条通道两种记录**：像素与控制消息复用 frames 通道（见 §5），因为二者必须保持
彼此有序——「光标变成 I 形」和「显示新 hover 状态的那次重绘」是同一件事；反向的
控制应答复用 events 通道，因为 JVM 侧读端可以保持成一个 `readFully(32)` 循环。

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

## 5. 控制面：不是像素的那一半

只有像素的 AWT 桌面是不可用的：caciocavallo 的 peer 还实现了 `CTCClipboard`、
`CTCRobotPeer`、光标管理、窗口标题与输入法（见 `cacio-shared` / `cacio-tta`），
这些**都**需要真正持有屏幕的宿主配合。

| JVM 做了什么 | 启动器必须做什么 |
|---|---|
| `setCursor(TEXT_CURSOR)` | 画 I 形指针而不是箭头（这是「手指下面是文本框」的唯一线索） |
| `JFrame.setTitle("Forge 安装程序")` | 给画布标题 |
| `Clipboard.setContents("seed")` | 放进 Android 剪贴板 |
| `Clipboard.getContents()` | **应答**当前 Android 剪贴板（Swing 线程正阻塞在这里） |
| 文本组件获得焦点 | 在光标处弹出软键盘 |
| `Toolkit.beep()` | 一次触感反馈 |
| cacio 的 managed screen 实为 N×M | 把画布**静默**调成 N×M |

### 控制消息格式（`"RCAC"`，小端，JVM → 启动器）

```text
0  u32 magic("RCAC")  4  u16 version  6  u16 kind
8  u32 seq           12  i32 a       16  i32 b      20 i32 c
24 u32 payload_len                  28  u32 flags
32 …  payload_len 字节 UTF-8 文本
```

**头部与帧头同形**（version 在 4、payload_len 在 24）而 magic 不同，这正是
`AwtFrameStream::read_next` 能用一次读头解复用两种记录的原因：记录先被**整条消费**
再看 magic，所以哪怕是未知记录类型也无法让通道失步（`a_corrupt_control_message_keeps_the_frame_stream_aligned`）。

| kind | 码 | 载荷 |
|---|---|---|
| `cursor` | 1 | `a` = `java.awt.Cursor` 类型（0..13；自定义位图光标退化为箭头） |
| `title` | 2 | `text`（空串 = 清除标题） |
| `clipboard_set` | 3 | `text`（JVM 复制了什么） |
| `clipboard_request` | 4 | `seq`（用同一个 seq 应答，晚到的应答不会被错认） |
| `beep` | 5 | — |
| `screen_size` | 6 | `a`×`b`：cacio 真正在用的 managed screen |
| `ime_show` | 7 | `a`,`b` = 插入符（桌面像素），`c` = 行高 |
| `ime_hide` | 8 | — |
| `window_opened` | 9 | `a` = 窗口 id，`text` = 标题 |
| `window_closed` | 10 | `a` = 窗口 id |
| `bye` | 11 | `text` = 原因（桥接干净退出） |

### 控制应答（定长 32 字节 × N，启动器 → JVM）

反向通道**保持定长**，JVM 侧读端就还是 `readFully(32)`；文本（剪贴板应答）跨记录
分片，字段直接复用 `AwtEventRecord`：

| 字段 | 含义 |
|---|---|
| `id` | `CONTROL_EVENT_ID = 0x72630001`（远离一切 `java.awt.event` id，绝不会被 `postEvent`） |
| `x` | 应答类型（`clipboard` / `clipboard_empty` / `pong`） |
| `y` | 被应答的 `seq` |
| `button` / `key_code` | 分片序号 / 分片总数 |
| `key_char` | 本片有效字节数（0..8） |
| `modifiers`,`wheel` | 本片的 8 个文本字节 |

* 超长文本在**字符边界**截断（`MAX_REPLY_TEXT` = 8 KiB），JVM 永远收到合法 UTF-8。
* **空文本也发一片**：请求必须有应答，否则阻塞在 `getContents()` 的 Swing 线程会永久挂住。
* 出队限流（`shed_one`）**最后**才动控制记录：分片应答只有整条才有意义。

### 两种消费方式

`AwtSession` 同时提供「投影」与「消息流」，因为二者用途不同：

* **投影**（`AwtControlState`：光标/标题/IME/窗口/剪贴板）是 last-write-wins 的，
  UI 每帧直接渲染；
* **消息流**（`drain_control()`）里的副作用必须**恰好发生一次**——写一次 Android
  剪贴板、震一次、弹一次键盘。

`awtDrainControl` 一次调用把两者一起返回；控制入队有界，溢出时**先丢**纯记账消息
（beep、窗口簿记），保住光标与剪贴板交接（`the_control_inbox_is_bounded_and_sheds_bookkeeping_first`）。

### 静默采纳 managed screen

`submit_frame` 本来就能跟随「与画布尺寸不符」的整帧，但它走 `resize_screen`——
会**回抛一个 `COMPONENT_RESIZED`**。这在 UI 改尺寸时是对的，在这里是错的：JVM 才是
告诉我们的一方，回抛会让 cacio 在启动过程中反复重排并反复广播。因此
`screen_size` 走 `adopt_screen_size`：静默、且发生在**第一次重绘之前**，第一帧就落在
尺寸正确的画布上，不必付一次整画布重分配。

## 6. 输入

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

## 7. 健壮性（task 19）

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
| 损坏/未知控制消息 | 整条消费后才解析，`FrameRead::Rejected` 计数并继续；下一条记录仍能解析 |
| 控制消息参数荒谬 | 如 `screen_size 0x0`：拒绝并计入 `controls_rejected`，投影与画布不受污染 |
| 非 UTF-8 标题/剪贴板 | 解码即报错（不做 U+FFFD 静默替换）——标题最终要给人看 |
| UI 停止 poll 控制面 | 入队有界，先丢记账消息；光标与剪贴板交接优先保留 |
| 出队限流撞上分片应答 | `shed_one` 最后才丢控制记录，绝不给 JVM 半条应答 |
| Android 剪贴板为空 | 仍回 `clipboard_empty`——**应答**而不是沉默 |
| 桥接说再见（`bye`） | 清空待应答请求、收起键盘，UI 不再等永远不会来的东西 |
| 桌面缩小后插入符越界 | 每次改尺寸（UI 改或 JVM 广播）都把 `ImeCaret` 夹回画布，软键盘不会锚到信箱黑边或屏幕外 |

## 8. 接口速查

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
| `awtDrainControl()` | **控制面**：取出控制消息 + 当前投影（光标/标题/IME/剪贴板/窗口） |
| `awtControl(json)` | 启动器的应答：`clipboard` / `clipboard_empty` / `clipboard_seq` / `pong` / `reset` |
| `awtSubmitControl(bytes)` | 手工投一条 `RCAC`（自检 / Kotlin 侧传输） |

启动侧串联：`LaunchOptions.awt_transport_dir = <dir>` 时，`CommandBuilder` 会把
`rc.awt.bridge.*` 三个属性交给 JVM；不设置则 cacio 依旧离屏渲染（对话框能用，只是
不显示）——**永远不会**出现「FIFO 没有读者把游戏第一次重绘卡死」的情况。

## 9. 测试

```bash
cd rust && cargo test --workspace     # awt / fakefx / awt_host / ffi::awt_tests
cargo run --example awt_demo          # 端到端：真实命名管道 + 假 JVM + 假 UI
python3 scripts/check_awt_wire.py     # 跨语言 wire 契约门禁（无需工具链）
```

`examples/awt_demo.rs` 在**没有 Android、没有 JVM**的情况下跑完整条链路：假 JVM 线程
经真实 FIFO 发一整屏重绘 →「光标/窗口/IME/复制」四条控制消息 → 一个损坏帧 → 一个
8×4 脏矩形补丁 → 一条损坏的控制消息 → 一次 `Clipboard.getContents()`；UI 线程按帧
poll 到 framebuffer、每帧排空控制面、把「Android 剪贴板」应答回去，并注入一次点击 +
一次 Esc。输出会显示：脏区域上屏只花 **128 B / 8192 B（1%）**、损坏的**帧与控制消息**
都被计数但链路存活、投影里 `cursor=text` / 标题 / `wants keyboard=true` / JVM 复制的文本，
JVM 侧收到 `MOUSE_PRESSED/RELEASED/CLICKED` 与 `KEY_PRESSED/RELEASED(VK_ESCAPE)`
（坐标 (32,16) 正是信箱化后的桌面中心），以及**分片重组后原样到达 JVM 的剪贴板应答**。

* `launch::awt`（`control_tests`，13 项）：光标类型全量往返与自定义/未知光标退化、
  控制 kind 码往返、`RCAC` 逐字段往返、控制头与帧头同形而 magic 不同、
  解码拒绝截断/错 magic/错版本/未知 kind/荒谬长度/非 UTF-8、kind 专属 JSON 键、
  应答分片与重组（含字符边界截断）、损坏 run 被拒、控制 id 不与任何 AWT id 冲突。
* `launch::fakefx`：wire 格式、canvas/damage、视口映射、输入翻译、限流，以及控制面
  投影（光标/标题/IME 夹取与视口映射/窗口栈/beep）、静默采纳 managed screen、
  剪贴板请求应答（含空剪贴板）、请求与入队双向有界、**应答不被限流拆散**、
  帧与控制消息解复用、损坏控制消息不致失步、`read_frame` 暂存控制消息。
* `launch::awt_host`（23 项）：真实 `UnixStream` 与**真实 FIFO** 的端到端往返、
  坏帧不致命、EOF/失步的原因字符串、`stop_and_join` 不等待 JVM、`Drop` 停止 pump、
  重复 attach 被拒绝（不泄漏线程）、通道路径不可创建时报错而非 panic，以及控制消息
  经 pump 抵达会话、像素与控制共享一条通道不失步、剪贴板请求经事件 pump 应答回 JVM。
* `ffi::awt_tests`（21 项）：JSON 控制面（开/关/配置/输入批次/坏事件）、像素往返、
  短缓冲被拒后 damage 仍在、`transport` 目录真的生成两个 FIFO、控制面排空/应答/
  reset/pong/垃圾消息不致命/无会话时报错，以及两个**跨语言契约测试**
  `the_snapshot_carries_every_field_the_compose_layer_parses` 与
  `the_control_snapshot_carries_every_field_the_compose_layer_parses`（快照/poll/input/
  控制批次的每个字段都是 Kotlin 会读的键——CI 跑不了 Kotlin 单测，这两项就是护栏）。
* `scripts/check_awt_wire.py`：**静态**跨语言门禁。它既不需要 Rust 工具链也不需要 JVM，
  直接比对两侧源码里的 magic / 版本 / 头长 / kind 码 / 应答码 / 光标号与 id /
  `java.awt.event` id / `ScaleMode` id / 控制 JSON 键，并检查 `FakeAwtCanvasBridge`
  处理了每一种 kind。漂移在这里失败，而不是在设备上变成一块永远黑的画布。
* Kotlin：`app/src/test/java/com/rc/launcher/ui/awt/*`（视口与 Rust 逐位一致、wire 编解码、
  输入 JSON、快照解析、keycode 映射、`AwtControlTest`：`RCAC` 字节布局/分片应答/
  光标映射/失败软化的 JSON 解析/Fake 桥接投影）与
  `ui/viewmodel/AwtSurfaceViewModelTest`（用 `FakeAwtCanvasBridge` 走完整 UI 路径，
  含控制面投影、剪贴板应答、关闭会话即收起键盘）。
