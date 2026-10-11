//! LeanFlip shared configuration and account-management core.
//! Implementation and compatibility notes: docs/engineering/independent-core.md.

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
