//! 启动 MCP 服务器子命令
//!
//! 以 stdio 模式启动 CodeConnect MCP 服务器，
//! 读取已构建的索引并提供代码分析工具供 AI 助手调用。

use std::path::Path;

use codeconnect_core::config::CodeConnectConfig;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};
use codeconnect_mcp::server::CodeConnectServer;
use codeconnect_mcp::tools::ToolRegistry;

/// 启动 MCP stdio 服务器
///
/// # 参数
///
/// - `project_root` — 项目根目录
/// - `data_dir` — 索引数据目录（包含 tantivy 和 sled 子目录）
/// - `config` — CodeConnect 配置
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
    config: &CodeConnectConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = config;
    tracing::info!("CodeConnect MCP 服务器正在启动...");
    tracing::info!("项目根目录: {}", project_root.display());
    tracing::info!("数据目录:   {}", data_dir.display());
    tracing::info!("模式:       stdio");

    let tantivy_dir = data_dir.join("tantivy");
    let tantivy_edges_dir = data_dir.join("tantivy_edges");
    let sled_dir = data_dir.join("sled");

    // 尝试打开已有索引（目录不存在时不自动创建，serve 不会因此崩溃）
    let tantivy = TantivyIndex::open_only(&tantivy_dir).ok();
    let call_edge_index = CallEdgeIndex::open_only(&tantivy_edges_dir).ok();
    let sled = SledStore::open_only(&sled_dir).ok();

    // 统计索引文档数
    let doc_count = tantivy.as_ref().and_then(|t| t.doc_count().ok()).unwrap_or(0);
    let edge_count = call_edge_index.as_ref().and_then(|c| c.doc_count().ok()).unwrap_or(0);
    tracing::info!("已加载索引: {} 个符号文档, {} 条调用边", doc_count, edge_count);

    if doc_count == 0 {
        tracing::warn!("索引为空！请先运行 `codeconnect index` 构建索引。");
    }

    // 构建 ToolRegistry — 即使索引为空也能启动，MCP 工具返回友好错误提示
    let registry = ToolRegistry::new()
        .with_query_engine_opt(tantivy, sled)
        .with_call_edge_index_opt(call_edge_index)
        .with_project_root(project_root.to_path_buf())
        .with_data_dir(data_dir.to_path_buf());

    // 创建并启动服务器
    let server = CodeConnectServer::new(registry);

    tracing::info!("MCP 服务器已就绪，等待客户端连接...");

    server.start_stdio().await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

    Ok(())
}
