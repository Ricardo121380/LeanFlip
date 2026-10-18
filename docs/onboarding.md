# 首次使用向导与 Provider 导入

桌面端首次启动时展示三步向导：认识 LeanFlip、选择来源并导入 Provider、首次切换说明。支持中文、英文，以及从设置页重新打开。

## 首次启动判定

在启动 Service 时检查 LeanFlip 数据目录。已有 `providers.json`、`app-settings.json` 或 `onboarding-v1.json` 的安装不自动弹窗。跳过、关闭或完成时原子保存 `onboarding-v1.json`；保存失败保留弹窗并显示错误。此标记独立于应用设置，不改目录覆盖、登录保留设置或客户端配置。

## 导入范围

| 来源 | 读取位置 | 支持内容 |
| --- | --- | --- |
| CC Switch | 自动检测其目录覆盖，否则 `~/.cc-switch`；`cc-switch.db` 优先，兼容 `config.json` | 复用已有工具映射、兼容性检查与导入清理。向导不读取独立 OAuth 账号仓库 |
| AI Toolbox | 平台应用数据目录下 `com.ai-toolbox/ai-toolbox.db`；支持 `app_paths.json` 的 `data_dir_override` | SQLite JSON/JSONB 中 Claude Code、Claude Desktop、Codex、Gemini CLI 的独立 Provider 记录 |
| Superba | macOS `~/Library/Application Support/com.krill-code.Superba`；Windows `~/AppData/Local/krill-code/Superba/data`；Linux `~/.local/share/superba` | `profiles.json` schema 1 中 Codex API Provider；保留主地址、模型、推理强度、速度档位和全局上下文设置 |

非默认 XDG 目录、便携安装和环境变量覆盖，可在向导中填写绝对数据目录。用户每次选择一个来源，扫描后逐项勾选；导入结束可以继续选择其他来源。

AI Toolbox 的文件型工具使用客户端当前配置，不属于上述独立数据库 Provider 表；可以在相应 LeanFlip 工具页面导入当前配置。公共配置、额外设置、账号登录、原工具网关和自动切换策略不在向导迁移范围。

Superba 的官方登录、恢复点和手动 PAT 配置会解释原因并跳过。自动认证配置明确按 API Key 直连导入；内置 Krill 线路映射到同一线路的 `/codex/v1` API Key 地址，预览中显示说明。自定义完整 Base URL 原样保留。备用线路、自动选线、模型目录、路由器和额外 Codex 设置不会迁移。

Superba 钥匙串/环境变量引用以及 AI Toolbox 缺失的内嵌密钥，要求用户在所选条目中补填 Key。不会枚举系统钥匙串或环境变量。Superba 文件密钥仅允许其真实数据目录内 `secrets/profile-*.secret`，拒绝指向目录外的文件及符号链接。

## 写入和隐私边界

- 扫描来源使用只读访问；AI Toolbox 查询启用 SQLite 防御模式、只读授权、超时和数量/大小限制。
- 预览只向前端发送名称、工具、提示和是否需要补填密钥。完整配置保留在后端内存中，不返回或记录 Key。
- 确认时只接受当前预览编号及索引，导入用户预览过的那份快照；扫描失败、关闭向导或预览编号变化会使旧预览失效。
- 单次确认通过已有原子 Provider 库更新添加所选卡片。相同工具和 ID 再次导入会跳过，不覆盖现有项。
- 不设置当前供应商、不写客户端配置、不导入执行命令信任，也不迁移来源账号凭据。用户之后在工具页面主动应用配置。
- 失败保留选择以便重试。导入期间阻止关闭和来源切换。弹窗内容单独滚动，底部操作保持可见。

## 格式核对依据

2026-10-18 直接检查以下上游源码，测试使用人工构造的隔离样本，不读取本机真实账户配置：

- [AI Toolbox](https://github.com/coulsontl/ai-toolbox/tree/019372a8e64215fb7089a8ed28206da9f934c43c)：`tauri/src/app_paths.rs`、`db/schema.rs`、`coding/*/types.rs`、`coding/proxy_gateway/provider_protocol.rs`。
- [Superba](https://github.com/krill-lab/superba/tree/b69ec57da77c326651c3b4a5d1f4d7772ed0536c)：`superba-core/src/profile.rs`、`secret.rs`、`catalog.rs`、`models.rs`、`superba-switch/src/lib.rs`、`superba-config/src/secrets.rs`。

## 本地验证

前端入口：`./node_modules/.bin/vite --mode mock --host 127.0.0.1 --port 15231`，访问 `http://127.0.0.1:15231/?onboarding` 强制展示模拟首次启动。模拟导入不会操作真实配置。

已覆盖首次弹出/完成后不再弹出/设置页重新打开、逐项选择与补填 Key、空来源、扫描和保存失败、失败重试、忙碌期间禁止关闭、错误选择、重复导入、来源快照变化、JSONB、凭据隔离和目录边界。

相关前端测试 32 项通过（向导 7、应用入口 5、设置导航 9、现有导入流程 11）；Rust 新导入测试 9 项、现有 CC Switch 导入及事务测试 6 项、Service 测试 26 项通过。TypeScript 类型检查、生产前端构建、桌面端 `cargo check` 及 core/service/app 的 Clippy 严格检查通过。关闭向导后的键盘焦点回归已由组件测试覆盖。

浏览器实际点击验证了补填 Key、导入、完成、设置页重开及 Escape 关闭。检查了 1380×745 和 390×600 的 DOM 布局，窄窗口弹窗无横向溢出、正文可滚动、底部按钮位于视口内，减少动态效果生效。截图接口遇到浏览器最小化及 macOS ScreenCaptureKit 错误，未完成像素级截图检查。未在 Windows/Linux 原生窗口或安装包中运行。
