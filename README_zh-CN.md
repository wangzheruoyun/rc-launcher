# RC Launcher

> 🌐 [English](README.md)

一款高性能、高健壮性的 **Android 平台 Minecraft Java 版启动器**，采用 **Rust 核心**
（下载 / 鉴权 / 启动 / 版本解析，通过 `cargo-ndk` 交叉编译为原生 `.so`）与
**Jetpack Compose** UI，并针对中国大陆网络做了专项优化（镜像源、DNS 优化、断点续传）。

## 功能特性

- **可续传、可校验的下载** — 并行分块下载，支持 Range 续传、SHA-1/MD5 校验与指数退避重试。
- **中国大陆网络优化** — 内置 BMCLAPI / MCBBS / 阿里云镜像，自动测速择优；支持 DoH 解析、
  Happy Eyeballs 及可配置的 HTTP/HTTPS/SOCKS5 代理。
- **JRE 供给** — 使用 FCL 预构建的 OpenJDK 包（Java 8 / 17 / 21 / 25），SHA-1 校验通过后
  由纯 Rust 的 `.tar.xz` 解码器解包。
- **启动引擎** — 启动前预检、含 LWJGL 替换的 classpath 组装、完整 JVM 命令行、受监管的进程
  及中英双语崩溃诊断。
- **渲染集成** — 预构建 LWJGL 原生库 + GL4ES/ANGLE 转译，并为弱 GPU 提供可调性能配置。
- **AWT/Swing 兼容（fakefx）** — 通过 caciocavallo 将 Forge/OptiFine 安装器与崩溃对话框
  渲染为可触控、零拷贝的 Compose 画布。
- **国际化** — 以中文为优先的 `*.properties` 文案目录；未翻译字符串回退到 zh-CN。
  详见 [docs/i18n.md](docs/i18n.md)。

## 模块结构

```
app/      # :app     — Compose UI
core/     # :core    — Rust/JNI 桥接 + 原生 .so
runtime/  # :runtime — JRE / 原生库管理
rust/     # Cargo workspace → libcrc_launcher.so
```

依赖方向严格无环：`:app → :core → :runtime`。
所有依赖版本统一置于 `gradle/libs.versions.toml`（单一来源）。

## 构建

构建均在 GitHub Actions 上运行，发布产物为 **单个 `arm64-v8a` `.apk`**（不发布 AAB）。
完整流水线详见 [docs/BUILD.md](docs/BUILD.md)，CI 定义见 [.github/workflows/](.github/workflows/)。

```bash
cd rust && cargo test --workspace   # Rust 核心测试
./gradlew assembleDebug             # 本地调试 APK
```

## 文档与贡献

- 架构 — [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 模块接口 — [docs/MODULES.md](docs/MODULES.md)
- 构建与发布 — [docs/BUILD.md](docs/BUILD.md)
- 贡献指南 — [CONTRIBUTING.md](CONTRIBUTING.md)
- 子系统文档 — `docs/{auth,launch,rendering,awt,i18n,ffi_event_bus,health_audit}.md`

代码风格由 [`.github/workflows/stylecheck.yml`](.github/workflows/stylecheck.yml) 强制执行
（Rust `fmt` + `clippy` 为硬性门槛；Kotlin `ktlint`/`detekt` 尽力而为）。

## 许可证

GPL-3.0-or-later。
