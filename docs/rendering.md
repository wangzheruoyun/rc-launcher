# 渲染集成（Task 17）—— LWJGL + GL4ES/ANGLE（Android OpenGL）

Minecraft 的桌面版 LWJGL 链接的是**桌面 OpenGL**。Android 上没有桌面 OpenGL，
只有系统驱动暴露的 **OpenGL ES**，或 Vulkan 后端的 **ANGLE**。本模块把 FCL /
Zalith 预编译的三部分拼装起来，让游戏在手机上获得真实图形输出：

1. **LWJGL 原生库** —— `app_runtime/lwjgl/<ver>/natives/<abi>/*.so`
   （`liblwjgl.so`、`liblwjgl_opengl.so` …）。它们替换 vanilla `version.json`
   原本会加载的桌面 natives（见任务 7 的 classpath 替换）。
2. **GL4ES / ANGLE** —— 把 OpenGL 翻译成 OpenGL ES 的翻译层。GL4ES
   （`libgl4es_114.so`）被 LWJGL 的 `liblwjgl_opengl.so` 通过
   `-Dorg.lwjgl.opengl.libname` 加载；ANGLE 提供 `libGLESv2_angle.so` +
   `libEGL_angle.so`，并通过 `ANGLE_DEFAULT_PLATFORM` 选择后端。
3. **性能调优** —— `PerfProfile`，在弱机上用吞吐换取严格的 GL 错误检查。

代码位于 `rust/crates/rc-launcher-core/src/launch/render.rs`。

## 1. LWJGL 原生库清单与校验（`LwjglNativeBundle`）

任务 7 的 `AppRuntime::verify` 只检查 `natives/<abi>/` 目录存在；任务 17 进一步
校验**具体的原生库文件**是否落地。清单按版本区分（取自
`FCL_APK_RUNTIME_ASSETS_CATALOG.md`）：

| 库 | 3.3.3 | 3.4.1 | 必需？ |
|---|---|---|---|
| `liblwjgl.so` | ✓ | ✓ | **是**（核心运行时） |
| `liblwjgl_opengl.so` | ✓ | ✓ | **是**（GL 绑定，被 GL4ES/ANGLE 满足） |
| `liblwjgl_stb.so` | ✓ | ✓ | 否 |
| `liblwjgl_nanovg.so` | ✓ | ✓ | 否 |
| `liblwjgl_tinyfd.so` | ✓ | ✓ | 否 |
| `liblwjgl_vma.so` | ✓ | ✓ | 否 |
| `libfreetype.so` | ✓ | ✓ | 否 |
| `libshaderc.so` | ✓ | ✓ | 否 |
| `liblwjgl_spng.so` | — | ✓ | 否（3.4.1 新增 PNG 编解码） |

- `LwjglNativeBundle::scan` —— 扫描目录，把每个期望的库分类为 `present` / `missing`，**永不报错**。
- `LwjglNativeBundle::discover` —— 缺失**必需**库时返回可操作的
  `RcError::MissingFile`（列出缺失库名），缺失可选库则放行。

启动引擎在 `preflight_app_runtime` 中调用 `discover`：缺失 `liblwjgl_opengl.so`
会在 spawn JVM **之前**失败并提示「重新解压 LWJGL 包」，而不是让游戏以不透明的
`UnsatisfiedLinkError` 崩溃。

## 2. OpenGL → OpenGL ES 翻译配置（`gl_translation_env`）

翻译层的基础变量（`LIBGL_ES`、`LIBGL_DRIVERS_PATH`、Mesa/Gallium 的
`MESA_LOADER_DRIVER_OVERRIDE` 等）由 `Renderer::env()` 提供（任务 9）。
本模块**补充**引擎不知道的部分：

- **ANGLE** —— 必须显式选择 Vulkan 后端，否则回退到并不存在的桌面 GL：
  `ANGLE_DEFAULT_PLATFORM=vulkan`、`ANGLE_NO_VALIDATION=1`。
- **GL4ES** —— 补充 `LIBGL_GLEXT` 指向 `nativeLibraryDir`（GL 扩展表位置）。
- **VirGL / Zink（Mesa）** —— 驱动 `.so` 已从 `nativeLibraryDir` 加载，引擎已设
  `LIBGL_DRIVERS_PATH`，此处不重复。

## 3. 性能调优（`PerfProfile`）

`PerfProfile`（Diagnostic / Balanced / LowPower / HighPerformance / Maximum）
通过 `LIBGL_*` / Mesa 开关换取吞吐。所有变量都是**追加**在渲染器基础环境之上、
且在用户 `env_overrides` 之前，因此用户仍可覆盖。

| 变量 | 作用 | LowPower | HighPerf | Maximum |
|---|---|---|---|---|
| `LIBGL_NOERROR` | 关闭 GL4ES 错误检查（最大 CPU 开销来源） | ✓ | ✓ | ✓ |
| `MESA_NO_ERROR` | 关闭 Mesa 错误检查 | ✓ | ✓ | ✓ |
| `LIBGL_NOINDIRECT` | 直连渲染，跳过间接开销 | | ✓ | ✓ |
| `LIBGL_FPS` | 屏幕 FPS 叠加 | | | ✓ |

默认 `Balanced` / `Diagnostic` 不输出任何变量（保持错误检查用于调试）。

## 4. 与启动引擎的接线

- `env.rs::build_env` —— 在 `Renderer::env()` 之后、用户覆盖之前，应用
  `gl_translation_env(renderer, native_lib_dir)` 与 `perf_profile.env()`。
- `engine.rs::preflight_app_runtime` —— 当 `verify_app_runtime` 开启且
  `app_runtime` 已配置时，调用 `LwjglNativeBundle::discover` 校验原生库完整性。
- `LaunchOptions` 新增 `perf_profile: PerfProfile` 字段（默认值 `Balanced`，
  `#[serde(default)]`，UI 可通过 JSON 传入）。

## 5. 测试

`render.rs` 内置单元测试，覆盖：

- 清单区分必需/可选库；3.4.1 新增 `spng`。
- `scan` 正确分类 present/missing。
- `discover` 接受完整包、拒绝缺失必需库、容忍缺失可选库。
- `PerfProfile` 各档位 env 内容；serde 往返（默认 `Balanced`）。
- `gl_translation_env` 为 ANGLE 选择 Vulkan、为 GL4ES 指向 native dir、对 Mesa 不重复。
- `RenderIntegration` 组合 native-lib 搜索目录与完整 env（翻译 + 性能）。

运行：`cd rust && cargo test --workspace`（全量 336 项通过，含本模块 11 项）。
