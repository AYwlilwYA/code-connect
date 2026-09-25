//! CodeConnect MCP 服务器模块
//!
//! 基于 rmcp 的 MCP 服务器实现：
//! - [`server`] — 服务器创建与启动（stdio / SSE / Streamable HTTP）
//! - [`tools`] — 全部 MCP 工具注册与 handler 函数
//! - [`schemas`] — JSON Schema 参数定义（schemars）
//! - [`semantic`] — 向量语义检索（`semantic_search` 的真向量实现）

pub mod schemas;
pub mod semantic;
pub mod server;
pub mod tools;
