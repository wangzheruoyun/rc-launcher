# 架构分析报告（Step 2）—— com.rc.launcher

> 目标：基于以下 4 个 GitHub 组织/仓库的目录结构快照，提炼可用于「Android 平台 Minecraft Java 版启动器（Rust 核心 + Jetpack Compose UI，针对中国大陆网络优化）」的架构设计。
> 快照存放于 `~/com.rc.launcher/snapshots/`（共 39 份，2 份因 GitHub HTTP 451 访问受限：`FCL-Team/FCLRendererPlugin`、`ZalithLauncher/ZalithRendererPlugin`）。

## 1. FCL-Team / FoldCraftLauncher（Android Minecraft Java 启动器）
- **技术栈**：Kotlin/Java + Gradle 多模块 + 自带 JRE（Android-OpenJDK-Build）+ LWJGL/GL4ES/ANGLE/caciocavallo。
- **模块划分**（根 `settings.gradle.kts`）：
  - `FCL`：UI 与上层逻辑（`com/tungsten/fcl`、`fcllibrary` 主题引擎）。
  - `FCLCore`：核心逻辑（`com/tungsten/fclcore`），子包即子系统边界：
    - `auth/`（账号鉴权）、`download/`（下载管理）、`event/`（事件总线）、`fakefx/`（JavaFX/AWT-Swing 在 Android 的兼容渲染）、`game/`（版本管理）、`launch/`（启动引擎）、`mod/`（Mod/资源包）、`task/`（异步任务框架）、`util/`。
  - `FCLauncher`：启动引导；`Terracotta`：版本/启动引擎实现；`ZipFileSystem`：自定义 ZIP 虚拟文件系统；`LWJGL`：LWJGL 3.3.3/3.4.1 原生库打包。
- **CI**：`.github/workflows/build.yml`、`release.yml`、`cleanup-artifacts.yml`。
- **可借鉴**：清晰的多模块边界；`download/`、`auth/`、`launch/`、`mod/`、`game/` 直接对应启动器核心子系统；`fakefx` 解决 Minecraft 内嵌 UI 在移动端的渲染；Terracotta 的进程派生与日志捕获模式。

## 2. ZalithLauncher / ZalithLauncher + ZalithLauncher2（Android 启动器衍生）
- **技术栈**：Kotlin + Gradle 单模块 + `jre_lwjgl3glfw`；ZalithLauncher2 为较新重写，CI 更成熟（`build.yml`、`push_ci.yml`、`release_ci.yml`）。
- **可借鉴**：单一主模块 + `jre_lwjgl3glfw` 的轻量结构；成熟的 push/release CI 流水线；NativeLib 插件（`NativeLibPlugin`）与渲染插件（`RendererPlugin`/`RendererPlugin-v2`）的插件化加载思路。

## 3. pmh1314520 / MCTier（Rust 后端 + 前端，虚拟局域网联机工具）
- **技术栈**：Rust（Tauri2 桌面端 `src-tauri/`：`Cargo.toml`、`tauri_commands.rs`、`tauri_events.rs`）+ React/TypeScript 前端 + Android（Kotlin，`MCTier-Android/app/`，通过 JNI 调用 `libeasytier_ffi.so` 的 Rust 核心）。
- **可借鉴（最关键）**：这正是「**Rust 核心 + 前端 UI**」的目标形态——Rust 编译为原生库（`.so`/FFI），Android 侧以 Kotlin/Compose 调用；Tauri 命令/事件模型（`tauri_commands.rs`/`tauri_events.rs`）可类比我们的 FFI/JNI 桥接层与事件总线。

## 4. cuberite / cuberite（高性能 C++ Minecraft 服务端）
- **技术栈**：C++ + CMake（`CMake/` 模块化管理）+ Lua 插件 API + 跨平台 CI（`Build.yml`、`StyleCheck.yml` + AppVeyor）。
- **可借鉴**：高度模块化、插件化扩展（Lua API）、跨平台构建与严格风格检查；其「健壮性 + 高性能 + 清晰模块边界」是核心逻辑层的典范。

## 5. 综合可借鉴架构设计
| 关注点 | 借鉴来源 | 本启动器落地 |
|---|---|---|
| 多模块工程 | FCL Gradle 模块、cuberite CMake | `:app`(Compose) / `:core`(Rust-JNI桥) / `:runtime`(JRE/库)；Cargo workspace 经 cargo-ndk 出 .so |
| 核心子系统 | FCLCore 子包 | Rust crate 按 `auth/download/launch/game/mod` 划分 |
| 后端+前端形态 | MCTier Rust FFI + Compose/Kotlin | Rust 核心 .so + cbindgen/JNI 桥 + Compose UI |
| 网络优化 | 国内镜像/DoH 思路 | 镜像源测速择优 + DNS 优化 + 代理 |
| 渲染 | FCL/Zalith LWJGL+GL4ES/ANGLE、fakefx | LWJGL 原生 + GL4ES/ANGLE + AWT 兼容层 |
| 插件化 | FCL RendererPlugin、Zalith NativeLibPlugin | 渲染器/本地库插件接口与安全加载 |
| CI/CD | FCL/Zalith build+release、cuberite StyleCheck | GitHub Actions 交叉编译 + 构建签名 + 发布 + 审计 |
