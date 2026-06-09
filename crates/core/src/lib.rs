//! CodeConnect 核心模块
//!
//! 提供整个项目共享的基础类型：
//! - [`types`] — Symbol, CallSite, Import, FileMeta 等核心数据结构
//! - [`symbol_id`] — 稳定符号 ID 生成与解析（blake3 指纹）
//! - [`config`] — CodeConnectConfig 与 .codeconnect.toml 解析
//! - [`error`] — 分层的错误类型 CodeConnectError
//! - [`response`] — 统一的 MCP 响应信封

pub mod config;
pub mod error;
pub mod response;
pub mod symbol_id;
pub mod types;
