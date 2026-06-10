//! 启动 MCP 服务器子命令
//!
//! 以 stdio 模式启动 CodeConnect MCP 服务器，
//! 读取已构建的索引并提供代码分析工具供 AI 助手调用。

use std::path::Path;
use std::sync::Arc;

use codeconnect_core::config::CodeConnectConfig;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::TantivyIndex;
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
    eprintln!("CodeConnect MCP 服务器正在启动... (stderr, 不会影响 MCP 协议)");
    eprintln!("  项目根目录: {}", project_root.display());
    eprintln!("  数据目录:   {}", data_dir.display());
    eprintln!("  模式:       stdio");

    let tantivy_dir = data_dir.join("tantivy");
    let sled_dir = data_dir.join("sled");

    // 打开索引（为 query_engine 创建）
    let tantivy = TantivyIndex::open_or_create(&tantivy_dir)
        .map_err(|e| format!("无法打开 tantivy 索引: {}", e))?;
    let sled = SledStore::open(&sled_dir)
        .map_err(|e| format!("无法打开 sled 存储: {}", e))?;

    let doc_count = tantivy.doc_count().unwrap_or(0);
    eprintln!("已加载索引: {} 个符号文档", doc_count);

    if doc_count == 0 {
        tracing::warn!("索引为空！请先运行 `codeconnect index` 构建索引。");
    }

    // 构建查询引擎（拥有 tantivy 和 sled）
    let query_engine = Arc::new(codeconnect_index::query_engine::QueryEngine::new(tantivy, sled));

    // 注意：query_engine 已消费 tantivy 和 sled 的所有权，
    // ToolRegistry 通过 query_engine 访问它们。
    let registry = ToolRegistry::new()
        .with_query_engine(query_engine);

    // 创建并启动服务器
    let server = CodeConnectServer::new(registry);

    tracing::info!("MCP 服务器已就绪，等待客户端连接...");

    server.start_stdio().await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

    Ok(())
}
