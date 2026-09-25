//! 构建代码索引子命令
//!
//! 遍历项目目录，解析所有源文件，
//! 提取符号、调用和导入关系，写入 tantivy 和 sled 存储。

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use codeconnect_core::config::CodeConnectConfig;
use codeconnect_index::full_indexer::FullIndexer;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};
use codeconnect_parser::factory::ParserRegistry;
use codeconnect_parser::c::CParser;
use codeconnect_parser::cpp::CppParser;
use codeconnect_parser::csharp::CSharpParser;
use codeconnect_parser::java::JavaParser;
use codeconnect_parser::rust::RustParser;
use codeconnect_parser::typescript::TypeScriptParser;

/// 执行全量代码索引
///
/// # 参数
///
/// - `project_root` — 项目根目录路径
/// - `data_dir` — 索引数据存储目录
/// - `config` — CodeConnect 配置（语言开关）
/// - `force` — 是否强制全量重建（即便有现有索引）
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
    config: &CodeConnectConfig,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now();

    println!("开始构建代码索引...");
    println!("  项目根目录: {}", project_root.display());
    println!("  数据目录:   {}", data_dir.display());
    // 索引范围限定（workspace.roots）；"." 表示整个项目根目录
    if !config.workspace.roots.is_empty() {
        let scope = config
            .workspace
            .roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!("  索引范围:   {}", scope);
    }
    if force {
        println!("  模式:       强制全量重建");
    }
    println!();

    // 校验 roots 合法性 —— 必须发生在删除旧索引之前。
    //
    // 否则用户把 roots 写错（如 ["sub"] 手抖成 ["subb"]）时，
    // `-f` 会先删光旧索引、再由索引器静默扫出 0 个文件、最后退出码 0：
    // 索引被清空却报成功，所有 MCP 工具返回空，CI 还判通过。
    if !config.workspace.roots.is_empty()
        && codeconnect_index::full_indexer::effective_walk_roots(project_root, &config.workspace.roots)
            .is_empty()
    {
        return Err(format!(
            "workspace.roots 配置的目录全部无效（{}），已中止且未改动现有索引。\
             请检查 .codeconnect.toml 中 [workspace].roots 的路径拼写",
            config
                .workspace
                .roots
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }

    // 确保数据目录存在
    std::fs::create_dir_all(data_dir)?;

    let tantivy_dir = data_dir.join("tantivy");
    let tantivy_edges_dir = data_dir.join("tantivy_edges");
    let sled_dir = data_dir.join("sled");

    // 强制重建时清理旧索引（包括旧的 sled edges 数据）
    if force {
        let _ = std::fs::remove_dir_all(&tantivy_dir);
        let _ = std::fs::remove_dir_all(&tantivy_edges_dir);
        let _ = std::fs::remove_dir_all(&sled_dir);
    }

    // 创建索引存储
    let tantivy = TantivyIndex::open_or_create(&tantivy_dir)
        .map_err(|e| format!("无法创建 tantivy 索引: {}", e))?;

    let call_edge_index = CallEdgeIndex::open_or_create(&tantivy_edges_dir)
        .map_err(|e| format!("无法创建调用边索引: {}", e))?;

    let sled = SledStore::open(&sled_dir)
        .map_err(|e| format!("无法创建 sled 存储: {}", e))?;

    // 注册解析器（按配置开关）
    tracing::debug!(
        "语言开关: rust={}, ts={}, js={}, java={}, csharp={}, c={}, cpp={}, kotlin={}",
        config.languages.rust, config.languages.typescript, config.languages.javascript,
        config.languages.java, config.languages.csharp, config.languages.c, config.languages.cpp,
        config.languages.kotlin
    );
    let mut registry = ParserRegistry::new();

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

    // 创建并运行全量索引器（将索引实例包装为 Arc 以支持共享引用）
    let tantivy_arc = Arc::new(tantivy);
    let call_edge_arc = Arc::new(call_edge_index);
    let sled_arc = Arc::new(sled);
    let indexer = FullIndexer::new(project_root, tantivy_arc, call_edge_arc, sled_arc, parser_registry)
        .with_roots(config.workspace.roots.clone());

    println!();
    println!("正在扫描并解析源文件...");

    let stats = indexer.run().map_err(|e| format!("索引失败: {}", e))?;

    let elapsed = start.elapsed();

    println!();
    println!("索引完成!");
    println!("  ──────────────────────────────────");
    println!("  扫描文件数:   {}", stats.files_scanned);
    println!("  成功解析:     {}", stats.files_parsed);
    println!("  提取符号数:   {}", stats.symbols_found);
    println!("  发现调用数:   {}", stats.calls_found);
    println!("  发现导入数:   {}", stats.imports_found);
    println!("  解析失败:     {}", stats.failed_files.len());
    println!("  耗时:         {:.2}s", elapsed.as_secs_f64());
    println!("  ──────────────────────────────────");

    // 打印覆盖度告警（spec B）—— 「没索引」必须与「不存在」可区分
    if let Some(note) = &stats.coverage.note {
        println!();
        println!("⚠ 覆盖度审计:");
        println!("  {}", note);
        for kind in stats.coverage.kinds_without_symbols.iter().take(10) {
            println!(
                "  - {}：{} 个节点 / {} 个文件未产出符号{}",
                kind.kind,
                kind.node_count,
                kind.files,
                kind.reason
                    .as_ref()
                    .map(|r| format!("（{}）", r))
                    .unwrap_or_default()
            );
        }
    }

    // 打印失败详情（如果有）
    if !stats.failed_files.is_empty() {
        println!();
        println!("解析失败的文件:");
        for failure in &stats.failed_files {
            println!("  - {}", failure);
        }
    }

    Ok(())
}
