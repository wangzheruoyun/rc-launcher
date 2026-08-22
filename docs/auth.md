# 账户与鉴权模块（Task 5）

Rust 核心账户与鉴权子系统，对应 FCL `FCLCore/auth`。代码位于
`rust/crates/rc-launcher-core/src/auth/`。

## 组件
- `model.rs`：账户模型（`Account`/`MicrosoftAccount`/`OfflineAccount`，带 `type` 标签）与离线 UUID。
- `transport.rs`：`AuthTransport` trait；生产用 `ReqwestTransport`（可基于 `net::NetworkClient`），测试用 `MockTransport`（脚本化，无网络）。
- `microsoft.rs`：Microsoft OAuth 2.0 设备码流程与令牌链：`device_code → poll → XBL → XSTS → Minecraft`，以及 `refresh_account`（用 refresh_token 刷新并重跑链）。作用域 `XboxLive.signin offline_access`。
- `offline.rs`：离线账户与确定性离线 UUID（vanilla 同款）。
- `vault.rs`：`SecretVault` 抽象（Keystore 集成点）；`InsecureVault`（测试）、`AesGcmVault`（AES-256-GCM）。Android 上密钥来自 Keystore，经 `authInit(key_hex=...)` 注入。
- `store.rs`：`TokenStorage` —— `MemoryTokenStorage` / `FileTokenStorage`（加密落盘，原子写入）。
- `manager.rs`：`AccountManager` 增删查与 `ensure_fresh` 主动刷新。
- FFI（`ffi.rs`）：`authInit/authListAccounts/authAddOfflineAccount/authBeginMicrosoft/authCompleteMicrosoft/authRemoveAccount/authRefreshAccount/authEnsureFresh`，JSON 字符串进出，包裹 `catch_unwind`。

## 测试
`cargo test --workspace` 用 `MockTransport` 覆盖整条令牌链、刷新、轮询状态、离线 UUID、加密落盘与过期/拒绝边界，无需网络。
