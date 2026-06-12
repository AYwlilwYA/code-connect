//! 快速符号搜索子命令
//!
//! 通过 tantivy 全文索引按名称搜索符号，
//! 返回匹配符号的位置和类型信息。

use std::path::Path;

use codeconnect_index::tantivy_index::TantivyIndex;

/// 执行符号搜索
///
/// # 参数
///
/// - `project_root` — 项目根目录
/// - `data_dir` — 索引数据目录
/// - `query` — 搜索查询字符串
/// - `limit` — 最大结果数
/// - `language` — 语言过滤（可选）
/// - `kind` — 符号类型过滤（可选）
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
    query: &str,
    limit: usize,
    language: Option<String>,
    kind: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = project_root;

    let tantivy_dir = data_dir.join("tantivy");

    // 检查索引目录是否存在（不自动创建——索引应由 `codeconnect index` 命令构建）
    super::check_index_dirs_exist(data_dir)?;

    let tantivy = TantivyIndex::open_or_create(&tantivy_dir)
        .map_err(|e| format!("无法打开 tantivy 索引: {}", e))?;

    // 搜索符号（符号数据只存 tantivy，sled 仅用于调用边/文件指纹）
    let search_results = tantivy
        .search_by_name(query, limit)
        .map_err(|e| format!("搜索失败: {}", e))?;

    if search_results.is_empty() {
        println!("未找到匹配 '{}' 的符号", query);
        return Ok(());
    }

    println!("搜索 '{}' 结果 (最多 {} 条):", query, limit);
    println!("{0:-<80}", "");

    let mut shown = 0;
    for result in &search_results {
        // 语言过滤
        if let Some(ref lang) = language {
            let symbol_lang = result.stable_id.split("::").next().unwrap_or("");
            if symbol_lang != lang.as_str() {
                continue;
            }
        }

        // 类型过滤
        if let Some(ref kind_filter) = kind {
            if result.kind != *kind_filter {
                continue;
            }
        }

        // 搜索结果已包含完整的符号信息（从 tantivy STORED 字段）
        println!(
            "  {}  [{}]  (相关度: {:.2})",
            result.name, result.kind, result.score
        );
        println!(
            "    文件: {}:{}",
            result.file_path, result.line
        );
        if !result.signature.is_empty() {
            println!("    签名: {}", result.signature);
        }
        println!();
        shown += 1;
    }

    if shown == 0 {
        println!("未找到匹配的符号（已应用过滤条件）");
    } else {
        println!("{0:-<80}", "");
        println!("共显示 {} 条结果", shown);
    }

    Ok(())
}
