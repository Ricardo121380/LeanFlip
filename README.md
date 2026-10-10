# ccswitch-lite

只做一件事：**一键切换每个 AI 编码 agent 的供应商配置**（Claude Code、Codex……），
没有本地代理、路由、聚合、用量统计。

基于 [farion1231/cc-switch](https://github.com/farion1231/cc-switch)（MIT，© 2025 Jason Young）
的写入引擎重写，见 [NOTICE.md](NOTICE.md)。

## 状态

Phase 1：Rust 核心库 `ccsl-core`（无 Tauri 依赖），支持 Claude Code 和 Codex（API Key 供应商 +
ChatGPT 原生登录的混合登录保护）。桌面应用（Tauri 2 + React）在后续阶段。

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
└── import.rs       首次运行导入 live 配置
```

数据目录：`~/.cc-switch-lite/`（`providers.json`、`live-state.json`、`codex-login-stash.json`、
`backups/live-first-write/`），和 cc-switch 的 `~/.cc-switch` 互不干扰。

## 开发

```sh
cargo build
cargo test        # 测试全部使用临时 HOME，不会碰真实的 ~/.claude、~/.codex
cargo clippy --all-targets -- -D warnings
```

## 许可证

MIT，见 [LICENSE](LICENSE)。原作版权 © 2025 Jason Young（cc-switch）。
