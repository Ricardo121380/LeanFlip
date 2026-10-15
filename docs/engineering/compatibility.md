# 名称与历史兼容标识

当前版本为 `0.3.3`，产品名称为 LeanFlip（简切）。已发布的 v0.1.0 使用另一套名字；它的发布包保持原样，当前分支不再携带它的安装器元数据。

| 标识 | 用途 |
| --- | --- |
| `LeanFlip` | App 显示名、窗口标题和 macOS 应用菜单名 |
| `leanflip` | 命令行和 TUI；npm 包名、CLI Homebrew Cask 名 |
| `leanflip-app` | 桌面主二进制名称（`tauri.conf.json` 的 `mainBinaryName`）、npm 桌面安装命令、桌面 Homebrew Cask 名。不叫 `LeanFlip`，是因为 macOS 默认文件系统不区分大小写，会和 `target/` 下的 CLI `leanflip` 互相覆盖 |
| `dev.leanflip.desktop` | 桌面应用 identifier；Cask 的 `uninstall quit` 和 npm 安装器的替换检查都从 `tauri.conf.json` 生成 |
| `leanflip-core` / `leanflip-service` / `leanflip-app` / `leanflip-cli` | Rust 包名 |
| `LEANFLIP_HOME` | 调试用的 home 覆盖变量 |
| `~/.leanflip` | 数据目录。0.1.x 的旧数据目录在 App 启动或 CLI 首次写入时整体改名迁移，原处留下相对符号链接，仍在运行的旧版本可以继续读写；迁移前只读命令继续读取旧目录 |
| `leanflip-backup`（版本 1） | 加密备份内的格式标识，三端通用 |
| `__leanflip_redacted__` | 前端显示的隐藏凭据占位符，只在内存和前后端传输中出现，从不写盘 |
| `.cc-switch` / `cc-switch.db` | 源应用 CC Switch 的只读迁移格式，与本应用数据目录不同 |

工作区 Cargo、根 `package.json` 与 Tauri 配置的版本必须一致。`packages/npm/release.json` 和 `Casks/` 在每次发布后由 `scripts/prepare-distribution.mjs` 从正式附件生成。

core 的高层编辑与 OAuth 操作仍要求调用方按 service → OAuth/tool → state/store 的顺序持锁；App、CLI 与 TUI 由 `Service` 实施该约束。直接 core 切换在工具锁内重新读取 Claude / Grok 的上一供应商，Codex 则在工具锁内拒绝准备期间发生变化的供应商快照，避免用旧所有者清理字段。单独修改卡片库的代码也必须遵守高层锁契约。
