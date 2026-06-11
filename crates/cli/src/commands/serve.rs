//! 启动 MCP 服务器子命令
//!
//! 以 stdio 模式启动 CodeConnect MCP 服务器，
//! 读取已构建的索引并提供代码分析工具供 AI 助手调用。

use std::path::Path;
use std::sync::Arc;

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

    // 检查索引目录是否存在（不自动创建——索引应由 `codeconnect index` 命令构建）
    let required_dirs: [(&str, &Path); 3] = [
        ("tantivy", &tantivy_dir),
        ("sled", &sled_dir),
        ("调用边索引", &tantivy_edges_dir),
    ];
    let mut missing = Vec::new();
    for (name, dir) in &required_dirs {
        if !dir.exists() {
            missing.push(*name);
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "索引数据不完整，缺失: {}\n请先运行 `codeconnect index -p \"{}\"` 构建索引。",
            missing.join(", "),
            project_root.display()
        ).into());
    }

    // 打开索引（为 query_engine 创建）
    let tantivy = TantivyIndex::open_or_create(&tantivy_dir)
        .map_err(|e| format!("无法打开 tantivy 索引: {}", e))?;
    let call_edge_index = CallEdgeIndex::open_or_create(&tantivy_edges_dir)
        .map_err(|e| format!("无法打开调用边索引: {}", e))?;
    let sled = SledStore::open(&sled_dir)
        .map_err(|e| format!("无法打开 sled 存储: {}", e))?;

    let doc_count = tantivy.doc_count().unwrap_or(0);
    let edge_count = call_edge_index.doc_count().unwrap_or(0);
    tracing::info!("已加载索引: {} 个符号文档, {} 条调用边", doc_count, edge_count);

    if doc_count == 0 {
        tracing::warn!("索引为空！请先运行 `codeconnect index` 构建索引。");
    }

    // 构建查询引擎（拥有 tantivy 和 sled）
    let query_engine = Arc::new(codeconnect_index::query_engine::QueryEngine::new(tantivy, sled));

    // 注意：query_engine 已消费 tantivy 和 sled 的所有权，
    // ToolRegistry 通过 query_engine 访问它们。
    // 调用边索引独立传入（query_engine 不持有 CallEdgeIndex）。
    // 同时传入 project_root 和 data_dir，供 handle_reindex 调用 CLI 索引命令。
    let registry = ToolRegistry::new()
        .with_query_engine(query_engine)
        .with_call_edge_index(Arc::new(call_edge_index))
        .with_project_root(project_root.to_path_buf())
        .with_data_dir(data_dir.to_path_buf());

    // 创建并启动服务器
    let server = CodeConnectServer::new(registry);

    tracing::info!("MCP 服务器已就绪，等待客户端连接...");

    server.start_stdio().await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

    Ok(())
}
