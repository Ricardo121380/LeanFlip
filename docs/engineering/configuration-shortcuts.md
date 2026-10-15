# 配置快捷设置的兼容范围

本功能编辑原有 JSON / JSONC / TOML / YAML 文件，不安装或启动 Agent。下表是代码核对基线，不代表自动探测到的本机版本。CLI 参数、项目／模型／会话覆盖、组织策略和服务端能力仍会影响实际生效。

| Agent | 核对基线与源码 | 实现范围 |
| --- | --- | --- |
| Codex | [rust-v0.162.1](https://github.com/openai/codex/blob/092d3acd6bec3e3a14bdc7e7a2810ab628ab759d/codex-rs/models-manager/src/model_info.rs) | 上下文、压缩阈值、工具结果预算；模型容量和阈值由上游再次限制。 |
| Gemini CLI | [v0.64.0](https://github.com/google-gemini/gemini-cli/blob/4a0749aceac5643e6f85a36d7420314d85e13b52/packages/cli/src/config/settingsSchema.ts) | 普通／自动编辑／计划模式、压缩比例、会话轮数；不提供配置文件不支持的 YOLO。 |
| Grok Build | [2bdd1d6a](https://github.com/xai-org/grok-build/blob/2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8/crates/codegen/xai-grok-pager/docs/user-guide/26-config-reference.md) | 通用压缩百分比、沙箱、更新；具体模型的上下文、输出上限和压缩百分比。 |
| OpenCode 1.x | [v1.19.0](https://github.com/anomalyco/opencode/blob/733b2c627e5aec39cb33cc22373bdce33053ef2c/packages/core/src/v1/config/config.ts) | notify 更新检查、压缩、预留空间、权限；true 保留为自定义，避免跨版本改变更新行为。 |
| OpenCode 2.x | [37df62f4](https://github.com/anomalyco/opencode/blob/37df62f4a1b6d89187a1169187a6cef10557c0ca/packages/schema/src/config.ts) | 按文件结构识别 update、compaction.buffer 和 compaction.keep.tokens；不转换已有权限规则。 |
| OpenClaw | [v2026.10.3](https://github.com/openclaw/openclaw/blob/59cb44796b0f1f0daa0e37af8b1495abb45d9bc1/src/config/zod-schema.agent-defaults-base.ts) | 内置运行后端的压缩、记忆保存、思考档位、并发和超时；具体模型的上下文。 |
| Hermes | [v0.22.1](https://github.com/NousResearch/hermes-agent/blob/297552a9d646119871391d202509822cd86330df/agent/agent_init.py) | 压缩开关、比例、固定 token 上限、长期记忆和用户画像；保留 null 表示无固定上限。 |
| Pi | [v1.2.0](https://github.com/earendil-works/pi/blob/06a466935536f8c9bcacb2d5efc0fc021be1bdff/packages/coding-agent/docs/settings.md) | 自动压缩、输出预留量、近期内容保留量、重试、思考档位。 |
| MCode | [v0.6.5](https://github.com/MiniMax-AI/minimax-code/blob/309e28554ec091bb17b6c4f67111c8c01d688b94/packages/config/src/config.ts) | 权限模式、工具结果归档预算；自定义模型的上下文和输出上限。 |

Claude Code 依据[官方环境变量](https://code.claude.com/docs/en/env-vars)、[权限](https://code.claude.com/docs/en/permissions)、[缓存](https://code.claude.com/docs/en/prompt-caching)说明：压缩窗口是 100000–1000000；输出上限由模型决定，128000 只是未知模型的上限；缓存 TTL 要求 2.1.242+，环境变量可覆盖，Claude apps gateway 不支持一小时缓存。未找到充分依据的 Claude Desktop 快捷开关不添加。

## 保留与校验

- 移除 Codex `disable_response_storage` 的 UI 控件；已有原始字段保持不动，避免擅自清理用户配置。
- OpenCode 更新检查统一写入 `notify`，不用语义跨版本变化的 `true`。检测文件格式，不推断安装版本；新旧 provider 树都不暴露给通用编辑器。
- 未识别的枚举、嵌套权限规则和未改动的旧数值保留。重置只删除对应字段，不把默认值写死。新增／修改的数值在前端和文件写入边界校验。
- Gemini / Hermes 比例使用 0–1 数值；Grok 使用 0–100 整数。Pi 的输出预留量与近期保留量是两个不同预算。
- 模型 ID 当作字面键处理，支持点号和斜杠；数组模型只改目标元素。其他模型、未知元数据和原生文件注释保留。
- 通用配置仍校验文件 revision；外部改写后必须重新载入。供应商连接、密钥及 MCode 默认模型引用不可通过通用配置修改。
- 原始 TOML 的内联表等结构若无法安全局部编辑，保留原草稿并提示使用完整配置，不生成无效 TOML。

## 验证边界

前端测试覆盖控件与 JSON/TOML 双向同步、非法数值、未知值、缓存覆盖、模型键及 OpenCode 格式。Rust 服务测试使用临时 HOME 实际写入并重新解析原生文件，验证保留、重置、供应商切换与冲突。浏览器预览使用内存后端，不使用真实密钥。没有以“成功写文件”声称所有 Agent、接入渠道和模型都完成在线端到端验证。
