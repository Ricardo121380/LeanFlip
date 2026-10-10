# NOTICE

ccswitch-lite 是 [farion1231/cc-switch](https://github.com/farion1231/cc-switch) 的派生作品，
大量代码移植自该项目（基于 v4.0.8，commit `df40d53`）。

cc-switch 以 MIT 许可证发布：

    Copyright (c) 2025 Jason Young

完整许可证文本见本仓库的 [LICENSE](LICENSE)，ccswitch-lite 同样以 MIT 发布，保留原版权声明。

## 移植来源

| ccswitch-lite | cc-switch 原文件 | 改动 |
|---|---|---|
| `crates/core/src/live/engine.rs` | `src-tauri/src/live/engine.rs` | 去掉 `DeviceStore::for_device`（全局 home），其余未改 |
| `crates/core/src/live/floor.rs` | `src-tauri/src/live/floor.rs` | 测试 fixture 路径 |
| `crates/core/src/live/residue.rs` | `src-tauri/src/live/residue.rs` | 未改 |
| `crates/core/src/live/patch/*` | `src-tauri/src/live/patch/*` | 未改 |
| `crates/core/src/live/project/claude.rs` | `src-tauri/src/live/project/claude.rs` | 删掉代理 / Stack 投影 |
| `crates/core/src/live/project/codex.rs` | `src-tauri/src/live/project/codex.rs` | 删掉 `OfficialProxy`、`OfficialMirror`、模型目录；官方休眠表改为「原表去 Key」 |
| `crates/core/src/live/project/gemini.rs` | `src-tauri/src/live/project/gemini.rs` | 删掉代理契约 |
| `crates/core/src/live/project/grok.rs` | `src-tauri/src/live/project/grok.rs` | 删掉代理契约；cc-switch 代理占位表仍会清理 |
| `crates/core/src/grok_config.rs` | `src-tauri/src/grok_config.rs`（`validate_config_toml`、`is_official_live_config`） | 只取校验部分 |
| `crates/core/src/state.rs` | `src-tauri/src/mode/state.rs` | 只保留 `pending`、`written` |
| `crates/core/src/operation.rs` | `src-tauri/src/mode/operation.rs` | 落定状态改为调用方闭包（不依赖数据库） |
| `crates/core/src/codex/login.rs` | `src-tauri/src/services/provider/codex_login.rs` | 删掉代理目标 |
| `crates/core/src/codex/auth.rs` | `src-tauri/src/codex_config.rs`（auth 判定函数） | 未改 |
| `crates/core/src/fsutil.rs` | `src-tauri/src/config.rs`（原子写） | Windows 统一用 `rename` |
| `crates/core/src/switch/claude.rs` | `src-tauri/src/services/provider/claude_direct.rs` | 去掉数据库 |
| `crates/core/src/switch/codex.rs` | `src-tauri/src/services/provider/codex_direct.rs`（直连路径） | 去掉代理、模型目录、钥匙串、托管账号 |
| `crates/core/src/switch/gemini.rs` | `src-tauri/src/services/provider/gemini_direct.rs` | 去掉数据库 |
| `crates/core/src/switch/grok.rs` | `src-tauri/src/services/provider/grok_direct.rs` | 去掉数据库 |
| `crates/core/src/switch/claude_desktop.rs` | `src-tauri/src/claude_desktop_config.rs`（直连部分：路径、profile、`_meta.json`、模型名校验） | 去掉 proxy 模式和模型映射；官方按分类识别；四个文件一个操作 |
| `crates/core/src/codex/oauth/mod.rs` | `src-tauri/src/proxy/providers/codex_oauth_auth.rs`（`CodexOAuthManager`） | 改为同步；网络抽成 `OAuthTransport`；采纳判定、拒绝后恢复、代数锁、重复账号检查照搬；坏账号库报错不覆盖 |
| `crates/core/src/codex/oauth/live.rs` | `src-tauri/src/codex_config.rs`（托管 OAuth 标记、读回/同步 live） | 只认 v3 标记；刷新后同步 live 时拿 Codex 写锁、有未完成操作时跳过 |
| `crates/core/tests/codex_red_lines.rs` | `src-tauri/tests/golden/codex_red_lines.rs` | 改用临时 HOME 沙箱；代理相关红线未移植 |
| `crates/core/tests/claude_key_fields.rs` | `src-tauri/tests/golden/claude_key_fields.rs` | 同上；通用配置相关未移植 |
| `crates/core/tests/fixtures/claudeKeyFields.json` | `tests/config/claudeKeyFields.json` | 未改 |
