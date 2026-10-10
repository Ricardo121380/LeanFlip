//! 纯函数投影器：供应商行 → 客户端文件里关键字段和独有字段的目标值。
//!
//! 投影器不读盘也不写盘，只给出「这个供应商要求哪些键是什么值」，再转成对应格式的
//! 补丁交给引擎，以现有文件为底改写。
//!
//! Phase 1 只移植了 Claude Code 和 Codex；Gemini CLI、Grok Build 在 Phase 2。

pub mod claude;
pub mod codex;
