# 名称与历史兼容标识

当前源码版本为 `0.2.0-dev`，产品名称为 LeanFlip（简切）。历史 v0.1.0 的二进制、安装器元数据和发布材料继续对应旧版，不随开发版本号一起改写。

| 标识 | 当前用途与保留原因 |
| --- | --- |
| `LeanFlip` / `leanflip` | 当前 App 显示名与源码 CLI 命令；App、CLI、TUI 使用同一服务层 |
| `ccsl` / `ccsl-app` | 历史 v0.1.0 CLI 及其安装器命令；README 安装章节与旧发布包对应 |
| `ccsl-core` / `ccsl-service` / `ccsl-app` / `ccsl-cli` | Rust 包名及公共模块标识，保持已有构建命令和内部引用兼容 |
| `ccswitch-lite` | Tauri 桌面主二进制名称；保留已有打包及启动路径 |
| `dev.ccswitch.lite` | 桌面应用 identifier；保留既有应用数据与系统注册身份 |
| `~/.cc-switch-lite` | 本应用数据目录；继续读取已有卡片、账号、状态及备份，不迁移用户数据 |
| `switchleaf-backup`（版本 1） | 加密备份内的格式标识；现有 SwitchLeaf / LeanFlip 备份继续互通 |
| `Ricardo121380/ccswitch-lite`、npm 与 Homebrew 旧标识 | 仓库和历史分发入口；开发期不修改远端发布或包管理器状态 |
| `.cc-switch` / `cc-switch.db` | 源应用 CC Switch 的只读迁移格式，与本应用数据目录不同 |

工作区 Cargo、根 `package.json` 与 Tauri 配置的版本必须一致。`packages/npm` 和 Cask 当前仍指向历史二进制，版本及校验和由正式分发脚本在新版本发布时更新。

core 的高层编辑与 OAuth 操作仍要求调用方按 service → OAuth/tool → state/store 的顺序持锁；App、CLI 与 TUI 由 `Service` 实施该约束。直接 core 切换在工具锁内重新读取 Claude / Grok 的上一供应商，Codex 则在工具锁内拒绝准备期间发生变化的供应商快照，避免用旧所有者清理字段。单独修改卡片库的代码也必须遵守高层锁契约。
