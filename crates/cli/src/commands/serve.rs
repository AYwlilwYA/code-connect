//! 启动 MCP 服务器子命令
//!
//! 以 stdio 模式启动 CodeConnect MCP 服务器，
//! 读取已构建的索引并提供代码分析工具供 AI 助手调用。
//!
//! 同时启动文件监控，在源文件变更时自动触发增量索引更新。

use std::path::Path;
use std::sync::Arc;

use codeconnect_core::config::CodeConnectConfig;
use codeconnect_index::incremental::IncrementalIndexer;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};
use codeconnect_mcp::server::CodeConnectServer;
use codeconnect_mcp::tools::ToolRegistry;
use codeconnect_parser::c::CParser;
use codeconnect_parser::cpp::CppParser;
use codeconnect_parser::csharp::CSharpParser;
use codeconnect_parser::factory::ParserRegistry;
use codeconnect_parser::java::JavaParser;
use codeconnect_parser::rust::RustParser;
use codeconnect_parser::typescript::TypeScriptParser;

/// 启动 MCP stdio 服务器
///
/// # 参数
///
/// - `project_root` — 项目根目录
/// - `data_dir` — 索引数据目录（包含 tantivy 和 sled 子目录）
/// - `config` — CodeConnect 配置（含语言开关）
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
    config: &CodeConnectConfig,
) -> Result<(), Box<dyn std::error::Error>> {
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

    // 构建解析器注册表（根据 config 中的语言开关注册解析器）
    let mut registry = ParserRegistry::new();

    tracing::debug!(
        "语言开关: rust={}, ts={}, js={}, java={}, csharp={}, c={}, cpp={}, kotlin={}",
        config.languages.rust, config.languages.typescript, config.languages.javascript,
        config.languages.java, config.languages.csharp, config.languages.c, config.languages.cpp,
        config.languages.kotlin
    );

    if config.languages.rust {
        registry.register(Arc::new(RustParser::new()));
        tracing::info!("已注册: Rust 解析器");
    }
    if config.languages.typescript {
        registry.register(Arc::new(TypeScriptParser::new()));
        tracing::info!("已注册: TypeScript 解析器");
    }
    if config.languages.javascript {
        registry.register(Arc::new(TypeScriptParser::new()));
        tracing::info!("已注册: JavaScript 解析器");
    }
    if config.languages.java {
        registry.register(Arc::new(JavaParser::new()));
        tracing::info!("已注册: Java 解析器");
    }
    if config.languages.csharp {
        registry.register(Arc::new(CSharpParser::new()));
        tracing::info!("已注册: C# 解析器");
    }
    if config.languages.c {
        registry.register(Arc::new(CParser::new()));
        tracing::info!("已注册: C 解析器");
    }
    if config.languages.cpp {
        registry.register(Arc::new(CppParser::new()));
        tracing::info!("已注册: C++ 解析器");
    }
    if config.languages.kotlin {
        tracing::info!("已注册: Kotlin 解析器");
    }

    let parser_registry = Arc::new(registry);

    // 构建 ToolRegistry — 即使索引为空也能启动，MCP 工具返回友好错误提示
    let registry = ToolRegistry::new()
        .with_query_engine_opt(tantivy, sled)
        .with_call_edge_index_opt(call_edge_index)
        .with_project_root(project_root.to_path_buf())
        .with_data_dir(data_dir.to_path_buf())
        .with_config(config.clone())
        .with_parser_registry(Arc::clone(&parser_registry));

    // 启动文件监控 — 在后台监控源文件变更，触发增量索引
    // 仅在索引已加载且数据具备时才启动监控
    if doc_count > 0 {
        // 从 registry 中克隆索引的 Arc（复用已打开的实例，无需重新打开）
        let tantivy_monitor = registry.tantivy.clone();
        let sled_monitor = registry.sled.clone();
        let call_edge_monitor = registry.call_edge_index.clone();

        if let (Some(tantivy_monitor), Some(sled_monitor), Some(call_edge_monitor)) =
            (tantivy_monitor, sled_monitor, call_edge_monitor)
        {
            let project_root_owned = project_root.to_path_buf();
            let excludes = config.workspace.excludes.clone();

            // 创建增量索引器（复用 serve 已打开的索引实例，避免 sled 锁冲突）
            let incremental_indexer = IncrementalIndexer::new(
                &project_root_owned,
                sled_monitor,
                tantivy_monitor,
                call_edge_monitor,
                Arc::clone(&parser_registry),
            );

            tokio::spawn(async move {
                tracing::info!("文件监控已启动，将自动检测源文件变更并增量更新索引");
                if let Err(e) = incremental_indexer.start_watching(excludes).await {
                    tracing::error!("文件监控异常停止: {}", e);
                }
            });
        } else {
            tracing::warn!("无法启动文件监控: 索引存储未完整加载");
        }
    } else {
        tracing::info!("索引为空，跳过文件监控启动（索引构建后再重启即可）");
    }

    // 创建并启动服务器
    let server = CodeConnectServer::new(registry);

    tracing::info!("MCP 服务器已就绪，等待客户端连接...");

    server.start_stdio().await
        .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;

    Ok(())
}
