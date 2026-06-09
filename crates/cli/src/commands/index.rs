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
use codeconnect_index::tantivy_index::TantivyIndex;
use codeconnect_parser::factory::ParserRegistry;
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
    if force {
        println!("  模式:       强制全量重建");
    }
    println!();

    // 确保数据目录存在
    std::fs::create_dir_all(data_dir)?;

    let tantivy_dir = data_dir.join("tantivy");
    let sled_dir = data_dir.join("sled");

    // 强制重建时清理旧索引
    if force {
        let _ = std::fs::remove_dir_all(&tantivy_dir);
        let _ = std::fs::remove_dir_all(&sled_dir);
    }

    // 创建索引存储
    let tantivy = TantivyIndex::open_or_create(&tantivy_dir)
        .map_err(|e| format!("无法创建 tantivy 索引: {}", e))?;

    let sled = SledStore::open(&sled_dir)
        .map_err(|e| format!("无法创建 sled 存储: {}", e))?;

    // 注册解析器（按配置开关）
    let mut registry = ParserRegistry::new();

    if config.languages.rust {
        registry.register(Arc::new(RustParser::new()));
        println!("  已注册: Rust 解析器");
    }
    if config.languages.typescript {
        registry.register(Arc::new(TypeScriptParser::new()));
        println!("  已注册: TypeScript 解析器");
    }
    if config.languages.javascript {
        registry.register(Arc::new(TypeScriptParser::new()));
        println!("  已注册: JavaScript 解析器");
    }
    if config.languages.java {
        registry.register(Arc::new(JavaParser::new()));
        println!("  已注册: Java 解析器");
    }
    if config.languages.csharp {
        tracing::info!("C# 解析器已注册");
    }
    if config.languages.kotlin {
        tracing::info!("Kotlin 解析器已注册");
    }

    let parser_registry = Arc::new(registry);

    // 创建并运行全量索引器
    let mut indexer = FullIndexer::new(project_root, tantivy, sled, parser_registry);

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
