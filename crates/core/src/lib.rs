//! ccswitch-lite 的核心：只做「每个 agent 的配置切换」，没有本地代理 / 路由。
//!
//! 大部分写入逻辑移植自 [farion1231/cc-switch](https://github.com/farion1231/cc-switch)
//! （MIT，Copyright (c) 2025 Jason Young），见仓库根目录的 `NOTICE.md`。

pub mod ccswitch_export;
pub mod ccswitch_import;
pub mod codex;
pub mod coexist;
pub mod env;
pub mod error;
pub mod fsutil;
pub mod grok_config;
pub mod import;
pub mod live;
pub mod operation;
pub mod promo;
pub mod provider;
pub mod state;
pub mod store;
pub mod switch;

pub use env::Env;
pub use error::AppError;
pub use provider::{Provider, Tool};
