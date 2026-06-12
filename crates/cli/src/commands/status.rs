//! 查询索引状态子命令
//!
//! 显示索引量、存储占用、各语言分布等统计信息。

use std::collections::HashMap;
use std::path::Path;

use codeconnect_core::types::FileMeta;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::TantivyIndex;

/// 显示索引状态
///
/// # 参数
///
/// - `project_root` — 项目根目录
/// - `data_dir` — 索引数据目录
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = project_root;

    println!("CodeConnect 索引状态");
    println!("═══════════════════════════════════════════");
    println!("  数据目录: {}", data_dir.display());
    println!();

    let tantivy_dir = data_dir.join("tantivy");
    let sled_dir = data_dir.join("sled");

    // 检查索引是否存在
    if !tantivy_dir.exists() || !sled_dir.exists() {
        println!("  索引未构建");
        println!();
        println!("  请运行 `codeconnect index` 构建索引");
        return Ok(());
    }

    let tantivy = match TantivyIndex::open_only(&tantivy_dir) {
        Ok(t) => t,
        Err(e) => {
            println!("  无法打开 tantivy 索引: {}", e);
            return Ok(());
        }
    };

    let sled = match SledStore::open_only(&sled_dir) {
        Ok(s) => s,
        Err(e) => {
            println!("  无法打开 sled 存储: {}", e);
            return Ok(());
        }
    };

    // 索引文档数
    let doc_count = tantivy.doc_count().unwrap_or(0);
    let store_entries = sled.size();

    println!("  索引文档数:   {}", doc_count);
    println!("  存储条目数:   {}", store_entries);

    // Schema 版本
    if let Ok(Some(version)) = sled.get_schema_version() {
        println!("  Schema 版本:  {}", version);
    }

    // 各语言文件分布
    let mut lang_counts: HashMap<String, u64> = HashMap::new();
    let mut total_symbols: u64 = 0;

    let prefix = "meta:";
    for item in sled.scan_prefix(prefix.as_bytes()) {
        if let Ok((_key, value)) = item {
            if let Ok(meta) = serde_json::from_slice::<FileMeta>(&value) {
                *lang_counts.entry(meta.language).or_insert(0) += 1;
                total_symbols += meta.symbol_count;
            }
        }
    }

    if !lang_counts.is_empty() {
        println!();
        println!("  语言分布:");

        let mut sorted: Vec<_> = lang_counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));

        for (lang, count) in sorted {
            println!("    {:12}  {:>5} 个文件", lang, count);
        }

        println!();
        println!("  符号总数:     {}", total_symbols);
        println!("  已索引文件数: {}", lang_counts.values().sum::<u64>());
    }

    // 磁盘空间估算
    if let Ok(metadata) = std::fs::metadata(&sled_dir) {
        let size_mb = metadata.len() as f64 / 1_048_576.0;
        println!("  磁盘占用:     {:.2} MB", size_mb);
    }

    println!();
    println!("  状态: {}", if doc_count > 0 { "就绪" } else { "索引为空" });

    Ok(())
}
