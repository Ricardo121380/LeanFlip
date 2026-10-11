# ccswitch-lite

只做一件事：**一键切换每个 AI 编码 agent 的供应商配置**（Claude Code、Codex……），
没有本地代理、路由、聚合、用量统计。

基于 [farion1231/cc-switch](https://github.com/farion1231/cc-switch)（MIT，© 2025 Jason Young）
的写入引擎重写，见 [NOTICE.md](NOTICE.md)。

## 状态

- `crates/core`：Rust 核心库（无 Tauri 依赖）。切换式工具：Claude Code、Codex（含 ChatGPT 混合登录
  保护和多个托管 ChatGPT 账号）、Gemini CLI、Grok Build、Claude Desktop（直连）；并存式工具：
  OpenCode、OpenClaw、Hermes、Pi、MCode。
- `crates/app`：Tauri 2 桌面外壳（薄命令层 + 托盘 + 文件监听 + 托管账号后台刷新）。
- `crates/service`：App 和 CLI 共用的业务层、路径设置、脱敏与 OAuth 网络实现，不依赖 Tauri。
- `crates/cli`：独立命令行程序 `ccsl`，可在没有桌面 App、Node.js 或 WebView 的环境构建和运行。
- `ui/`：React + TypeScript + Vite + TanStack Query + shadcn/ui 风格组件，中英文界面。

首版为 **v0.1.0**，安装包与校验文件见 [GitHub Releases](https://github.com/Ricardo121380/ccswitch-lite/releases/latest)。
提供 macOS Apple Silicon 的 App（DMG）和 CLI，以及 Linux x86_64 的 App（DEB）和 CLI。
下载与安装说明见 [v0.1.0 发布说明](docs/releases/v0.1.0.md)。截图见 `docs/screenshots/`。

## 桌面操作

侧栏将单一配置切换与多供应商并存分组显示。供应商页支持按名称、ID、地址搜索，
当前选择单独标明，配置文件路径可展开和复制。写入时会禁用冲突操作；读取失败可原地重试。
设置页显示未保存状态，支持放弃修改；后台刷新不会覆盖正在编辑的设置或供应商草稿。
最小窗口为 860×560，侧栏与内容区可独立滚动，导航和控件支持键盘聚焦。
编辑器的保存区始终可见，关闭或更换预设前会保护未保存的输入；保存失败可直接修改重试。
设置页离开与历史前进/后退同样会确认未保存修改。导入页按本机目录、导出文件分开预览，
更换目录必须重新扫描；导入操作期间锁定来源和页面切换，避免预览与实际导入不一致。

## CLI

```sh
cargo build -p ccsl-cli --release --locked
./target/release/ccsl                  # 交互终端：方向键菜单
./target/release/ccsl menu codex       # 直接选择 Codex 供应商
./target/release/ccsl status
./target/release/ccsl list codex
./target/release/ccsl use codex provider-id
./target/release/ccsl enable opencode provider-id
./target/release/ccsl import file backup.sql --preview
./target/release/ccsl status --json
./target/release/ccsl --lang en --help
```

CLI 与 App 共用 `~/.cc-switch-lite/` 及目录覆盖设置。查询和导入预览不会恢复中断写入；
写入命令在跨进程锁下执行，`ccsl recover [tool]` 可显式恢复。`--home` 可指定独立测试根目录。
卡片编辑支持文件或标准输入，列表及 JSON 输出不包含密钥。完整命令、账号登录及退出码见
[CLI 使用说明](crates/cli/README.md)。
终端界面默认中文，`--lang en` 切换英文；列表按中英文显示宽度对齐，较窄的 `COLUMNS`
会启用字段列表。`--json` 的字段、英文错误消息和退出码保持稳定。
交互终端中直接运行 `ccsl` 即可用方向键、文字筛选和回车完成操作；`ccsl menu <tool>`
可直接进入指定工具，Esc 返回，Ctrl+C 退出。重定向或管道中的无命令调用仍显示快速入门；
命令参数错误显示对应帮助，人读输出支持管道提前关闭。

## 结构

```
crates/core/src
├── env.rs          路径解析（home、~/.claude、~/.codex、~/.cc-switch-lite），显式传递
├── error.rs        AppError
├── fsutil.rs       原子写（临时文件 + fsync + rename，0600）
├── live/           写入引擎：只替换关键字段，其余字节不碰（移植）
│   ├── engine.rs   计划 / 暂存 / 首写备份 / 重读比对 / 写锁
│   ├── floor.rs    关键字段与独有字段定义
│   ├── residue.rs  旧版残留清理
│   ├── patch/      JSON / TOML / .env / 整文件补丁
│   └── project/    供应商行 → 关键字段投影（claude、codex）
├── state.rs        live-state.json：pending 写前意图、written
├── operation.rs    一次操作：pending → 逐个发布 → 落定；崩溃后前滚 / 丢弃
├── codex/          auth.json 判定、混合登录（登录暂存）规划
├── provider.rs     供应商模型（兼容 cc-switch 行格式）
├── store.rs        providers.json 供应商库
├── switch/         切换入口（claude、codex）
├── import.rs       首次运行导入 live 配置
├── ccswitch_import.rs  只读导入本机 ~/.cc-switch（cc-switch.db / 旧版 config.json）
├── ccswitch_export.rs  导入 cc-switch 的导出文件（SQL 导出 / .db 快照 / 旧版 JSON），只进卡片库
└── promo.rs        认出推广链接（返利 / 追踪 / 短链），导入时丢掉
```

数据目录：`~/.cc-switch-lite/`（`providers.json`、`live-state.json`、`codex-login-stash.json`、
`backups/live-first-write/`），和 cc-switch 的 `~/.cc-switch` 互不干扰。

## 开发

```sh
cargo test        # 测试全部使用临时 HOME，不会碰真实的 ~/.claude、~/.codex
cargo clippy --all-targets -- -D warnings
pnpm install
pnpm typecheck && pnpm test     # 前端
pnpm tauri dev                  # 开发运行（会读写当前用户的真实配置！）
pnpm tauri build --bundles deb  # 本地打包，不发布
pnpm tauri build               # macOS：生成 .app 和 .dmg（平台配置自动合并）
pnpm presets ../ccswitch        # 从 cc-switch 源码重新生成裁剪后的预设（链接只留模型厂商官方平台，
                                # 白名单 ui/src/lib/official-vendor-domains.json；中转站链接整条删除，
                                # 删了哪些字段记在 ui/src/lib/removed-preset-links.json）
```

用演示数据安全地跑一遍（临时 HOME，不碰真实配置）：

```sh
DEMO=$(mktemp -d)
cargo run -p ccsl-app --example seed_demo -- $DEMO/home
HOME=$DEMO/home CCSL_HOME=$DEMO/home XDG_CONFIG_HOME=$DEMO/xdg target/release/ccswitch-lite
```

桌面应用自己的设置在 `~/.cc-switch-lite/app-settings.json`（语言、目录覆盖、开机自启、后台刷新）。

`.github/workflows/ci.yml` 在主分支 push / PR 时检查前端类型和测试、Rust 测试与 Clippy，
并在 Linux / macOS 分别验证 debug `.deb` / `.app` 打包；不会发布 Release。
CLI 使用独立任务测试和构建，不安装桌面依赖；两种平台的 release 二进制以保留执行权限的
`.tar.gz` 包保存在 CI artifacts。
`.github/workflows/release.yml` 在推送版本标签时构建正式优化的 App 和 CLI，校验版本与架构，
生成 `SHA256SUMS` 并创建带安装包的草稿 Release；确认检查和附件后再发布。
macOS 配置在 `crates/app/tauri.macos.conf.json`；Linux 仍使用 `.deb` / `.AppImage`
默认配置。公开分发所需的 Apple 签名和公证需另行配置。

## 许可证

MIT，见 [LICENSE](LICENSE)。原作版权 © 2025 Jason Young（cc-switch）。
