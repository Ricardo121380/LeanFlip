# 独立实现与来源核查记录

本轮从 `6fc809d` 开始，在 `refactor/independent-core` 分支实现。基线明确包含 CC Switch 移植代码。本轮以 LeanFlip 的产品行为、客户端配置格式和自行编写的实现为依据替换这些模块；没有把上游行为或旧测试当作必然正确的规范。

重写过程中读取了旧代码以核对接口与兼容行为，因此不宣称这是隔离式 clean-room 开发。本文记录实际替换范围、保留的互操作标识和验证证据，不作为法律意见。

## 替换范围

| 范围 | 当前实现 |
| --- | --- |
| `fsutil.rs`、`live/engine.rs` | 使用 `tempfile` 准备同目录私有文件；不可变快照、按数据目录隔离的线程门闩与系统文件锁、首写备份锁 |
| `state.rs`、`operation.rs` | 独立的 Journal 状态机；阶段文件、发布、取消、恢复和落定；损坏状态单独归档，未知版本拒绝覆盖 |
| `live/patch/*` | 自行编写 JSON、dotenv 编辑器及 JSONC 字节范围扫描／编辑器；TOML 使用 `toml_edit` 公共文档 API；移除不再使用的 `json-five` 依赖，没有升级其他依赖 |
| `live/floor.rs`、`live/project/*` | 按客户端字段和本项目所有权规则提取连接配置；字段名属于客户端接口事实；删除原先按固定数值删除配置的 `live/residue.rs` |
| `switch/{claude,codex,gemini,grok,claude_desktop}.rs` | 重新组织配置计划与事务适配；Codex 凭据准备和磁盘发布分离；Desktop 使用 LeanFlip 自有 UUID 与名称 |
| `grok_config.rs`、`error.rs` | 新的校验与错误实现；格式错误不回显源配置内容 |
| `codex/auth.rs`、`codex/login.rs` | 凭据载体判断、用户及 workspace 身份、登录暂存、纯函数切换决策 |
| `codex/oauth/{mod,live}.rs` | 磁盘为准的账号事务；缓存绑定完整账号版本；设备码兑换互斥、取消处理、刷新身份核对与客户端同步 |
| `coexist/{opencode,openclaw,hermes,pi,mcode}.rs` | 使用独立的 JSONC 条目编辑器、字段校验和 YAML 顶层段编辑器；MCode 保留无关注释并维护目录锁心跳 |
| `service/oauth_http.rs` | 独立 HTTP 适配器；禁用重定向、限制响应大小、校验必要字段、错误不回显响应体 |
| 移植测试与 fixture | 删除 `codex_red_lines.rs`、`claude_key_fields.rs` 及 `claudeKeyFields.json`；新增 `connection_contracts.rs`、`oauth_integrity.rs` 和新模块内的独立测试 |
| `ui/src/presets/*.json`、生成器 | 删除上游目录扫描和预设提取流程；自行维护 26 个官方登录及通用连接模板；删除旧预设链接清单 |

基线中独立编写的 App / CLI / TUI 界面、服务层业务、备份加密、供应商库、路径容器、CC Switch 导入／导出读取器和测试辅助服务继续使用。对比检查中仍相同的结构主要是公开 API、序列化字段、错误键、协议常量及简单接口样板；没有将“改名后 grep 不到 CC Switch”当成独立实现证据。

## 有意修正的行为

- **配置所有权：** 不再根据几个固定的上下文数值猜测来源；仅清理上一供应商记录且仍相同的可选字段。
- **磁盘安全：** 新版状态文件不会被旧解析器静默重置；恢复发布前重新读取客户端文件；取消失败时保留可恢复的阶段文件。
- **凭据安全：** JSON／TOML／HTTP 错误不附带配置或服务端原文；第三方路由不能回退发送官方凭据；命令鉴权与其他 token 来源冲突时拒绝。
- **Codex 账号：** 每次修改先锁定再读取最新账号库；按完整账号版本判断缓存是否有效；已删除账号不会被先前准备的切换重新写入；刷新响应改变用户或 workspace 时拒绝；重复设备码轮询不会重复兑换。
- **只读查询：** 原生登录列表不恢复操作、不创建目录或锁文件。
- **Gemini：** 空白密钥不算有效凭据；Vertex 使用 `vertex-ai`；名称为 Google 的 API 卡不会仅凭名称误选订阅登录。
- **dotenv：** 解析引号后的实际值，写入时不使用会改变反斜杠的 JSON 转义；不允许注入第二行变量。遇到多行／未闭合值时拒绝编辑，要求先在客户端处理。
- **Desktop：** LeanFlip 的 profile 不再冒用 CC Switch 的固定 UUID。导入读取实际激活的 profile，拒绝包含路径分隔符的标识。
- **Hermes / MCode：** YAML 段编辑保留其他文本及注释；Hermes 接受初始 `model: ""`；MCode 不再仅凭经过十秒就删除别人持有的锁。

## 实现依据

保留的字段名、配置路径和协议字符串用于互操作。实现参考以下公开规范；未复制这些文档中的完整示例程序或实现代码：

- [Codex 配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Codex 认证](https://learn.chatgpt.com/docs/auth)
- [Claude Code 环境变量](https://code.claude.com/docs/en/env-vars)
- [Claude Code 配置](https://code.claude.com/docs/en/settings)
- [Gemini CLI 配置](https://geminicli.com/docs/reference/configuration/)
- [OpenCode 供应商](https://opencode.ai/docs/providers/)
- [OpenClaw 自定义供应商](https://docs.openclaw.ai/gateway/config-tools/custom-providers)
- [Hermes 配置](https://hermes-agent.nousresearch.com/docs/user-guide/configuration/)
- [Pi 自定义模型格式](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/models.md)
- [dotenv 值与引号的解析规则](https://github.com/motdotla/dotenv/blob/master/lib/main.js)

Grok、Desktop 和 MCode 的已有配置结构继续兼容，并通过本项目临时目录中的输入／输出契约测试检验；本轮没有声称对所有真实客户端版本完成端到端测试。Codex JWT 解析仅用于本地身份匹配，不是 JWT 签名校验。

## 保留的互操作边界

- `~/.cc-switch-lite`、现有供应商 JSON 字段、账号库格式和旧备份格式继续可读。
- `cc-switch.db`、旧 JSON / SQL 导出及 `.cc-switch` 路径用于只读导入；没有对源应用的数据执行写入。
- `custom` 路由 ID、旧代理占位标记和已知旧目录指针仅用于识别现有数据，不引入 CC Switch 运行时或源码依赖。
- 接口与数据结构保持兼容，App、CLI 和 TUI 继续使用同一服务层。既有用户卡不依赖新的内置模板。
- 新版只提供 `leanflip` 命令。历史安装器所对应的旧二进制名称不据此改写。

## 许可证与分发

当前源码根目录使用 `Copyright (c) 2026 LeanFlip contributors` 的 MIT 许可证，不再声明当前实现移植自 CC Switch，根目录 `NOTICE.md` 已移除。

历史 v0.1.0 二进制确实包含上游实现。其原许可证和通知从 `v0.1.0` Git 标签原样归档为 `docs/releases/v0.1.0-LICENSE` 与 `v0.1.0-NOTICE.md`。历史 Git 提交、标签和远端发布未改动。仍下载旧二进制的 npm 安装器保留旧许可证通知；生成新版本分发包时脚本改用当前 LICENSE 并移除旧 NOTICE。生成测试覆盖“旧版 → 新版 → 旧版”，防止分发材料与实际二进制错配。

第三方依赖仍受各自许可证约束。[MIT 的保留通知条件](https://opensource.org/license/mit)仍适用于包含原代码的历史分发。这次工程核查不对版权争议作法律保证。

## 验证

全部使用临时 HOME、虚构密钥与本地假 OAuth 服务，不登录、刷新或修改真实账号。没有为验证而操作 Ghostty，也没有更改系统锁屏设置。

- 重写前核心基线通过：`target/independent-baseline.log`。
- 删除移植测试前，新实现通过已有行为回归：`target/independent-preaudit-tests.log`（暂跳过一项已单独修正的错误本地化断言；最终工作区未跳过）。
- 工作区测试：`target/independent-workspace-tests.log`，244 项通过；覆盖三端调用、迁移、加密备份、多进程写入、登录、故障恢复。
- 最后 dotenv 和刷新阶段文件清理修正后，156 项核心回归通过；另外新增 Desktop 外部 profile 导入及切走前新登录出现的检查，分别通过连接契约和 OAuth 回归。结果记录于 `target/independent-final-core-tests.log`、`target/independent-contract-tests.log`（6 项）和 `target/independent-final-oauth-tests.log`（32 项）。
- 前端：类型检查与 83 项测试通过；`target/independent-typecheck.log`、`target/independent-ui-tests.log`。
- npm：11 项测试通过；`target/independent-npm-tests.log`。
- Clippy：工作区全部目标、`-D warnings` 通过；`target/independent-clippy.log`；最后账号守卫修正后，核心所有目标再次通过，见 `target/independent-final-core-clippy.log`。
- 发布构建的 CLI 菜单与 TUI PTY 检查通过，含确认／取消、密钥隐藏、窄窗口、导入、加密备份恢复、SIGINT 和终端模式恢复；`target/independent-menu-check.log`、`target/independent-tui-check.log`。密码脚本显式等待关闭回显后才输入，避免终端库切换模式时清空过早发送的字符。
- macOS 开发 App 打包通过，产物 `target/debug/bundle/macos/LeanFlip.app`；发布 CLI 构建通过，产物 `target/release/leanflip`。仅验证打包，没有运行真实客户端登录流程或发布远端版本。
- 日志保存在本机忽略的 `target/` 目录，未提交测试凭据或构建产物。

锁与重新读回可以发现相当一部分外部并发编辑；不遵守本项目锁的客户端仍可能在最后读回与操作系统发布之间写入。没有宣称消除了所有跨应用竞争窗口。
