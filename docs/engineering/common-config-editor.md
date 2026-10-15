# 通用配置编辑器

供应商编辑器的常用设置和完整配置都不显示内部 ID；新建或复制时自动生成唯一标识，编辑已有卡片时保持原标识。供应商配置的快捷开关与原始配置同步；通用配置独立保存并影响同一 Agent 下的所有供应商。

10 种 Agent 都支持通用配置 JSON 编辑并保存为原生格式。Claude、Codex、Gemini、OpenCode 提供已定义的快捷选项；其他字段保留在 JSON 中，不把任意布尔字段自动转换为控件。未设置的选项显示“默认”，通知白名单等非布尔配置显示“自定义”，避免误认为关闭。重置只移除对应覆盖字段。

保存位置按需展开。JSON 格式错误、外部文件冲突和保存失败显示在固定操作区，正文滚动不影响错误可见性；失败保留草稿。重新载入仍需先确认丢弃未保存修改。

## 配置快捷项扩展（未发布）

新增的数值与下拉控件复用现有草稿、JSON/TOML 编辑器和保存事务。修改控件只改草稿，保存后才写入配置；重置只删除该字段，不清空同级设置。未知枚举值、Codex 细分审批对象继续显示为“自定义”，原样保留。

| 范围 | 快捷项 | 写入字段 |
|---|---|---|
| Codex 供应商 | 上下文窗口、自动压缩阈值、1M/900K 预设 | `model_context_window`、`model_auto_compact_token_limit`，TOML 整数 |
| Codex 供应商 | 搜索模式、输出详细程度 | `web_search`、`model_verbosity` |
| Codex 通用 | 命令审批、沙箱、工具输出预算 | `approval_policy`、`sandbox_mode`、`tool_output_token_limit` |
| Claude Code 供应商 | 第三方模型容量、压缩工作窗口、最大输出 | `env.CLAUDE_CODE_MAX_CONTEXT_TOKENS`、`env.CLAUDE_CODE_AUTO_COMPACT_WINDOW`、`env.CLAUDE_CODE_MAX_OUTPUT_TOKENS`，环境变量保持字符串 |
| Claude Code 通用 | 权限模式（含 Bypass） | `permissions.defaultMode`，保留 `allow`、`ask`、`deny` 等规则 |
| Claude Code 通用 | 主会话与子 Agent 缓存时长 | `promptCacheTtl`、`subagentPromptCacheTtl` |

数值输入不会通过 `parseInt` 截断非法后缀。错误显示在固定底栏并阻止保存；Codex 压缩阈值不可高于明确设置的上下文窗口。模型容量及输出仍受客户端和服务端上限限制。现有 `disableBypassPermissionsMode` 不会被解除；缓存环境变量覆盖已设置时，相关快捷项禁用并显示覆盖字段。项目级配置、启动环境和组织策略仍可能优先于这些用户级文件。

参考代码按 2026-10-15 的 GitHub 版本审阅，独立实现，没有复制参考项目代码：

- [CC Switch 的 Codex 配置控件](https://github.com/farion1231/cc-switch/blob/51ac86f1c59abfefabadebd05ec5a5415b8ad72c/src/components/providers/forms/CodexConfigSections.tsx)：上下文窗口和压缩阈值的直接配置入口。
- [Magpie 的原生配置调节](https://github.com/yetone/magpie/blob/1fd73a3fceb584d9e72aba4db8e519e9bb9f0655/internal/agent/tune.go)：压缩、缓存时长及环境变量覆盖处理。
- [Magpie 的 Claude 模型容量字段](https://github.com/yetone/magpie/blob/1fd73a3fceb584d9e72aba4db8e519e9bb9f0655/internal/agent/claude.go)：第三方模型上下文、工作窗口和输出容量。
- [Codex 配置 schema](https://github.com/openai/codex/blob/main/codex-rs/core/config.schema.json)、[Claude Code 权限模式](https://code.claude.com/docs/en/permissions)：独立核对字段类型与权限模式语义。

没有纳入代理 Header/Body 重写、协议转换和通过供应商名称诱导远程压缩等选项：它们需要额外网关或服务端能力，不能仅靠本项目写入配置保证生效。其他 Agent 保留已经提供的快捷开关与 JSON 编辑，不为凑齐控件而增加缺少明确依据的字段。

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
