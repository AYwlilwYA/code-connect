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

    let tantivy = TantivyIndex::open_only(&tantivy_dir)
        .map_err(|e| format!("无法打开 tantivy 索引: {}", e))?;

    // 搜索符号（符号数据只存 tantivy，sled 仅用于调用边/文件指纹）
    // 语言/类型过滤下推到查询，避免先按 limit 截断再过滤而少返回结果
    let search_results = tantivy
        .search_by_name(query, language.as_deref(), kind.as_deref(), limit)
        .map_err(|e| format!("搜索失败: {}", e))?;

    if search_results.is_empty() {
        println!("未找到匹配 '{}' 的符号", query);

        // 给出相近候选，帮助区分「拼写不对」与「确实不存在」
        if let Ok(suggestions) = tantivy.suggest_similar_names(query, 5) {
            if !suggestions.is_empty() {
                println!();
                println!("相近的符号名候选:");
                for s in &suggestions {
                    println!("  {}  [{}]  {}:{}", s.name, s.kind, s.file_path, s.line);
                }
            }
        }
        return Ok(());
    }

    println!("搜索 '{}' 结果 (最多 {} 条):", query, limit);
    println!("{0:-<80}", "");

    for result in &search_results {
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
    }

    println!("{0:-<80}", "");
    println!("共显示 {} 条结果", search_results.len());

    Ok(())
}
