//! CodeConnect Diff 感知模块
//!
//! - [`branch_diff`] — Git 分支对比：变更文件 + 行级 diff
//! - [`symbol_diff`] — 行级 diff 到符号范围映射、变更符号识别

pub mod branch_diff;
pub mod symbol_diff;
