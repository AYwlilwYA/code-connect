//! 调用关系图子命令
//!
//! 以纯文本形式展示指定符号的调用者和被调用者，
//! 支持控制方向和搜索深度。

use std::path::Path;
use std::sync::Arc;

use codeconnect_graph::call_graph::{CallGraph, CallChainNode};
use codeconnect_index::query_engine::QueryEngine;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};

/// 执行调用关系图查询
///
/// # 参数
///
/// - `project_root` — 项目根目录（目前未使用，保留用于未来扩展）
/// - `data_dir` — 索引数据目录
/// - `symbol` — 符号名称或符号 ID
/// - `direction` — 追踪方向：callers、callees 或 both
/// - `depth` — 最大搜索深度
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
    symbol: &str,
    direction: &str,
    depth: usize,
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

    // === 根据 direction 参数执行追踪 ===
    let show_callers = direction == "callers" || direction == "both";
    let show_callees = direction == "callees" || direction == "both";

    let callers: Vec<CallChainNode> = if show_callers {
        call_graph.trace_callers(&query_name, depth)
    } else {
        Vec::new()
    };

    let callees: Vec<CallChainNode> = if show_callees {
        call_graph.trace_callees(&query_name, depth)
    } else {
        Vec::new()
    };

    // === 输出结果 ===
    println!("=== {} ({}) ===", query_name, display_id);
    println!();

    if show_callers {
        println!("调用者 ({}):", callers.len());
        if callers.is_empty() {
            println!("  (无)");
        } else {
            for caller in &callers {
                let file_info = query_engine
                    .get_symbol_by_id(&caller.symbol_id)
                    .ok()
                    .flatten();
                print_node(caller, &file_info);
            }
        }
        println!();
    }

    if show_callees {
        println!("被调用者 ({}):", callees.len());
        if callees.is_empty() {
            println!("  (无)");
        } else {
            for callee in &callees {
                let file_info = query_engine
                    .get_symbol_by_id(&callee.symbol_id)
                    .ok()
                    .flatten();
                print_node(callee, &file_info);
            }
        }
    }

    Ok(())
}

/// 格式化输出单个调用链节点
///
/// 优先使用 tantivy 中的完整位置信息（文件路径+行号），
/// 退化到 CallChainNode 中的文件路径（可能为空）。
fn print_node(
    node: &CallChainNode,
    file_info: &Option<codeconnect_core::types::Symbol>,
) {
    let loc_str = if let Some(sym) = file_info {
        format!("{}:{}", sym.location.file_path, sym.location.line)
    } else if !node.file_path.is_empty() {
        node.file_path.clone()
    } else {
        "?".to_string()
    };

    println!("  {: <12} {: <24} depth={}", node.name, loc_str, node.depth);
}
