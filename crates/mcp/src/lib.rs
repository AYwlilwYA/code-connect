//! CodeConnect MCP 服务器模块
//!
//! 基于 rmcp 的 MCP 服务器实现：
//! - [`server`] — 服务器创建与启动（stdio / SSE / Streamable HTTP）
//! - [`tools`] — 全部 MCP 工具注册与 handler 函数
//! - [`schemas`] — JSON Schema 参数定义（schemars）

pub mod schemas;
pub mod server;
pub mod tools;
