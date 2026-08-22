# 启动引擎（Task 7）

Rust 核心启动引擎，对标 FCL `FCLCore/launch`（`DefaultLauncher`/`Launcher`）、
`FCLauncher`（`jre_launcher.c`）与 Terracotta 启动流程。代码位于
`rust/crates/rc-launcher-core/src/launch/`。

## 流水线

```
ResolvedVersion(Task 4) + LaunchOptions(UI) + JRE Home(Task 6)
                      │
                      ▼   LaunchEngine::prepare()
  ① 校验选项（账号 / 堆内存 / 绝对路径）
  ② 预检 JRE（存在 + majorVersion 匹配，不匹配直接报错而非让 JVM 崩溃）
  ③ 预检 app_runtime/（LWJGL、caciocavallo 是否齐全）
  ④ 组装类路径（规则过滤 + LWJGL 替换 + 重复依赖收敛）
  ⑤ 校验类路径每一项是否落盘（缺失即报「重新下载该版本」）
  ⑥ 创建 JVM 需要写入的目录（natives / logs / tmp / cache）
  ⑦ 生成 JVM 命令行与进程环境
                      ▼
              PreparedLaunch ──describe()──▶ 日志头（脱敏）
                      ▼   LaunchEngine::spawn() / launch_and_wait()
   GameProcess ──▶ GameExit { code, signal, log, crash }
```

## 组件

| 文件 | 职责 |
|---|---|
| `options.rs` | `LaunchOptions`（账号、内存、窗口、渲染器、ABI、目录、额外参数、环境覆盖）；可 serde，供 UI 以 JSON 传入；`AccountProfile` 的 `Debug` 自动脱敏 access token |
| `runtime_assets.rs` | 设备上的 `app_runtime/` 布局（LWJGL 3.3.3/3.4.1、caciocavallo/caciocavallo17、JNA），文件名取自真实 FCL APK（见 `FCL_APK_RUNTIME_ASSETS_CATALOG.md`） |
| `classpath.rs` | 规则过滤 + **LWJGL 替换**（Mojang 的桌面端 `org.lwjgl:*` 无法在 Android 加载，改用预编译 Android 包）+ 同坐标重复依赖取高版本 |
| `args.rs` | `${...}` 模板展开、`arguments.{game,jvm}` 规则过滤、无法解析的参数**连同其 flag 一起丢弃**（离线账号没有 `${clientid}`/`${auth_xuid}`） |
| `env.rs` | `LD_LIBRARY_PATH` / `java.library.path`（natives + LWJGL + nativeLibraryDir + JRE + 系统 GLES 目录）、`HOME`/`TMPDIR`、渲染器环境变量 |
| `command.rs` | 最终命令行：堆与 GC、编码、启动器标识、JNA、LWJGL/渲染器属性、模组加载器 quirks、log4j 配置、caciocavallo AWT 桥（Java 8 与 17+ 两套）、`-cp`、主类、游戏参数 |
| `process.rs` | 派生并监管进程：stdout/stderr 流式回调、有界日志环形缓冲、退出码/信号、`stop()`（SIGTERM→SIGKILL）、`kill_on_drop` |
| `crash.rs` | 崩溃分类：16 类 `CrashCategory` + 证据行 + `hs_err_pid*.log` + 首个 Java 异常 + 中英文处置建议 |
| `engine.rs` | 编排器 `LaunchEngine`（`prepare`/`spawn`/`launch`/`launch_and_wait`/`stop`），`PreflightChecks` 可关闭磁盘检查用于「命令预览」 |

## 关键设计（健壮性优先）

- **启动器掌握类路径与原生库路径**：清单里的 `-cp`、`-Djava.library.path=${natives_directory}`
  会被替换为启动器组装的版本（Android 还需要 LWJGL 预编译 natives、应用
  `nativeLibraryDir`、JRE 自身 `lib/` 与系统 `lib64`）。
- **绝不把未解析的 `${...}` 交给游戏**：否则 Minecraft 的参数解析会直接失败；被丢弃的
  参数全部记入 `PreparedLaunch::warnings`，UI 可解释原因。
- **用户参数最后追加**（HotSpot 取最后一个 `-Xmx`/`-D`），但用户的 `-cp` 会被丢弃并记录，
  避免静默破坏启动。
- **1.20+ 自动进服**用 `--quickPlayMultiplayer`，老版本才用 `--server/--port`；能力判定基于
  **未经规则过滤**的原始清单，否则会退化成 1.20+ 已移除的 `--server`。
- **秘密永不落日志**：access token 在 `Debug`、`to_shell_string()`、进程日志中都被替换为
  `<redacted>`。
- **内存有界**：日志为环形缓冲（`log_buffer_lines`），超长单行截断，避免模组刷日志把启动器
  自身 OOM。
- **输出不丢失**：进程退出后仍继续短暂抽取管道——崩溃报告恰好是最后打印的内容。
- **崩溃诊断只在真的崩溃时下结论**：退出码 0 一律视为正常退出（模组在正常运行时也会打印
  堆栈）；规则表按优先级排序，`OutOfMemoryError` 胜过随后的通用异常；原生信号
  （SIGSEGV/SIGABRT）在日志给出更好解释时让位于该解释。

## 崩溃分类

`clean_exit`、`user_terminated`、`killed_by_system`（Android LMK）、`out_of_memory`、
`unsupported_java_version`、`missing_native_library`、`graphics_failure`、`native_crash`、
`corrupted_file`、`missing_main_class`、`authentication_failure`、`disk_full`、
`permission_denied`、`mod_loader_failure`、`game_error`、`unknown`。

每类提供稳定 `id()`（供 Task 20 i18n 作 key）、`summary()`、`advice()` 与 `advice_zh()`。

## FFI（Task 7 部分，流式事件见 Task 10）

- `launchPreview(requestJson)` — `{"options":…,"version":…,"preflight":bool}` →
  预备启动 JSON（含命令行、类路径、环境、警告；不含 token）。
- `launchDiagnose(requestJson)` — `{"exit_code":…,"signal":…,"log":"…","requested_stop":…}` →
  崩溃报告 JSON。
- `launchRenderers()` — 渲染器目录（`id`/`gl_libname`/`env`），供设置页使用。

## 测试与演示

```bash
cd rust
cargo test --workspace            # 含 118 条 launch 子系统测试
cargo run --example launch_demo   # 端到端演示（伪 JRE：正常退出 / OOM 崩溃 / 停止挂死进程）
```

`launch_demo` 会搭建一棵临时的「设备目录」（伪 `app_runtime/`、伪 `bin/java` 脚本、
伪 `libraries/` 与客户端 JAR），完整跑通 组装 → 派生 → 采集 → 诊断，无需 JVM、
无需真机、无需联网。
