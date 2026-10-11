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
- `ui/`：React + TypeScript + Vite + TanStack Query + shadcn/ui 风格组件，中英文界面。

还没有发布版本。截图见 `docs/screenshots/`。

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

## 许可证

MIT，见 [LICENSE](LICENSE)。原作版权 © 2025 Jason Young（cc-switch）。
