# 通用配置编辑器

此功能单独放在 `codex/common-config-editor`，基于 `codex/audit-fixes-reviewed`。

常用设置隐藏 ID；创建时仍自动生成，内部标识移至完整配置的高级折叠区。供应商配置的快捷开关与原始配置同步；通用配置独立保存并影响同一 Agent 下的所有供应商。

10 种 Agent 都支持通用配置 JSON 编辑并保存为原生格式。Claude、Codex、Gemini、OpenCode 提供预置快捷选项；其余 Agent 为已有布尔字段生成开关。没有新增、删除或合并现有三条官方卡片。

## 配置写入范围

路径均遵循应用中的目录设置，下表为默认目录内的文件。

| Agent | 通用配置文件 | 保存格式 |
|---|---|---|
| Claude Code | `.claude/settings.json`，兼容原有旧文件路径 | JSON |
| Codex | `.codex/config.toml` | TOML |
| Gemini CLI | `.gemini/settings.json` | JSON |
| Grok Build | `.grok/config.toml` | TOML |
| Claude Desktop | `Claude/claude_desktop_config.json` 和 `Claude-3p/claude_desktop_config.json` | JSON，两份文件仅同步本次变更 |
| OpenCode | `.config/opencode/opencode.jsonc` 或 `opencode.json` | JSONC |
| OpenClaw | `.openclaw/openclaw.json` | JSONC |
| Hermes | `.hermes/config.yaml` | YAML |
| Pi | `.pi/agent/settings.json` | JSON，独立于供应商的 `models.json` |
| MCode | `.minimax/config.yaml` | YAML，遵循客户端文件锁 |

供应商地址、凭据、模型和路由表继续由供应商编辑器管理。通用配置不读取或改写 Codex `auth.json`。修改已有字段时保留其他配置及未改动区域的 TOML/JSONC/YAML 注释。缺失、损坏、外部修改的文件分别处理，不在格式错误时覆盖原文件。

## 验证与边界

- 核验 JSON/开关双向同步、非法输入、取消、导航保护、外部修改冲突。
- 5 项服务集成测试覆盖 10 个 Agent 的写入、重读、字段保护、注释保留、空配置和非法类型；3 项 UI 测试覆盖编辑流程。
- 对真实 Claude/Codex 配置的临时副本执行修改、保存、重读、恢复，均通过；没有改写真实客户端文件。
- 当前本地完整分支：`pnpm typecheck`、150 项前端测试、315 项 Rust workspace 测试、Clippy、格式检查和 Tauri 调试构建全部通过。原 17 项修复的复查见同目录 `audit-fixes-review.md`。
- 这些验证覆盖 LeanFlip 配置读写路径，不等于所有第三方 Agent 已完成真实联网任务。客户端重启、项目级配置或启动参数可能影响最终生效值。

## 配置字段依据

参考行为与实现分开核验，没有复制 CCS 实现。具体字段依据包括 [Claude Agent Teams](https://code.claude.com/docs/en/agent-teams)、[Codex 配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)、[Gemini CLI 配置](https://geminicli.com/docs/reference/configuration/)、[OpenCode 配置](https://docs.opencode.ai/docs/config/)，以及仓库现有的独立配置投影契约。未设置的快捷开关明确标为“默认”，不把未配置误当成客户端已关闭；重置会移除覆盖字段。

## 构建验证记录

最终 Rust 测试、Clippy 和桌面构建使用此 worktree 自己的 `target/`，没有再与纯修复工作树共用编译产物。共用缓存曾导致 Clippy 读取另一分支的旧元数据；隔离并清理本项目编译缓存后已重跑通过。原始输出保存在 `dev-notes/verification-2026-10-14/recheck-*-isolated.log`。

- `cargo test --workspace --locked --offline`：315 passed，退出 0。
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`：退出 0。
- `pnpm tauri build --debug --ci --no-bundle -- --locked --offline`：退出 0。
- 构建产物是此 worktree 的 `target/debug/leanflip-app`，没有替换已安装的应用。两个分支均为本地提交，尚未推送或创建远端 PR。
