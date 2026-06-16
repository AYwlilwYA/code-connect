//! 符号引用查找子命令
//!
//! 查找指定符号的所有调用者（引用位置），
//! 输出纯文本格式的引用列表。

use std::path::Path;
use std::sync::Arc;

use codeconnect_graph::call_graph::CallGraph;
use codeconnect_index::query_engine::QueryEngine;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};

/// 执行符号引用查找
///
/// # 参数
///
/// - `project_root` — 项目根目录（目前未使用，保留用于未来扩展）
/// - `data_dir` — 索引数据目录
/// - `symbol` — 符号名称或符号 ID
/// - `include_declaration` — 是否包含符号的声明位置
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
    symbol: &str,
    include_declaration: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = project_root;

    // 检查索引目录是否存在
    super::check_index_dirs_exist(data_dir)?;

    let tantivy_dir = data_dir.join("tantivy");
    let tantivy_edges_dir = data_dir.join("tantivy_edges");
    let sled_dir = data_dir.join("sled");

    // 打开各索引（只读，不自动创建）
    let tantivy = TantivyIndex::open_only(&tantivy_dir)
        .map_err(|e| format!("无法打开 tantivy 索引: {}", e))?;
    let call_edge_index = CallEdgeIndex::open_only(&tantivy_edges_dir)
        .map_err(|e| format!("无法打开调用边索引: {}", e))?;
    let sled = SledStore::open_only(&sled_dir)
        .map_err(|e| format!("无法打开 sled 存储: {}", e))?;

    let tantivy_arc = Arc::new(tantivy);
    let sled_arc = Arc::new(sled);
    let query_engine =
        Arc::new(QueryEngine::from_arc(tantivy_arc.clone(), sled_arc.clone()));
    let call_edge_index_arc = Arc::new(call_edge_index);

    // 先尝试按 ID 查找目标符号，失败则按名称搜索
    let target_sym = query_engine
        .get_symbol_by_id(symbol)
        .ok()
        .flatten()
        .or_else(|| {
            // 按名称搜索，取第一个匹配
            query_engine
                .search_by_name(symbol, None, None, 1)
                .ok()
                .and_then(|results| {
                    results.first().map(|r| {
                        codeconnect_index::query_engine::symbol_search_result_to_symbol(r)
                    })
                })
        });

    // 确定用于调用图查询的符号名称
    let query_name = target_sym
        .as_ref()
        .map(|s| s.name.clone())
        .unwrap_or_else(|| symbol.to_string());

    // 获取显示用的符号 ID
    let display_id = target_sym
        .as_ref()
        .map(|s| s.id.clone())
        .unwrap_or_else(|| symbol.to_string());

    // 扫描所有符号 ID 以构建调用图
    let all_ids = query_engine
        .scan_all_ids()
        .map_err(|e| format!("扫描符号 ID 失败: {}", e))?;

    // 构建调用图（使用 tantivy 调用边索引）
    let call_graph = CallGraph::build_from_tantivy_edges(&call_edge_index_arc, &all_ids)
        .map_err(|e| format!("构建调用图失败: {}", e))?;

    // 查找引用（向上追溯调用者，深度 10 覆盖多数场景）
    let callers = call_graph.trace_callers(&query_name, 10);

    // === 输出结果 ===
    println!("符号: {} ({})", query_name, display_id);

    // 如果要求显示声明位置
    if include_declaration {
        if let Some(ref sym) = target_sym {
            println!(
                "声明:  {}:{}:{}",
                sym.location.file_path, sym.location.line, sym.location.column
            );
        } else {
            println!("声明:  未找到");
        }
    }

    println!("引用数: {}", callers.len());
    println!();

    // 输出每个引用（按 symbol_id 从 tantivy 获取完整的文件位置信息）
    for caller in &callers {
        let file_info = query_engine
            .get_symbol_by_id(&caller.symbol_id)
            .ok()
            .flatten();

        if let Some(ref sym) = file_info {
            println!(
                "  {}:{}:{}  ({})",
                sym.location.file_path,
                sym.location.line,
                sym.location.column,
                simplify_call_type(&caller.call_type)
            );
        } else {
            println!("  {}  ({})", caller.name, simplify_call_type(&caller.call_type));
        }
    }

    Ok(())
}

/// 将中文调用类型简化为终端友好的短英文标签
fn simplify_call_type(ct: &str) -> &str {
    match ct {
        "直接调用" => "direct",
        "虚函数调用" => "virtual",
        "回调调用" => "callback",
        "宏展开调用" => "macro",
        _ => ct,
    }
}
