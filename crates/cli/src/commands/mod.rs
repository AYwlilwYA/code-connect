use std::path::Path;

use codeconnect_index::sled_store::SledStore;
use codeconnect_index::text_scan::{
    TEXT_DETAIL_LIMIT, TEXT_SUMMARY_THRESHOLD, TextScanReport, TextTruthContext,
    indexed_file_paths, scan_indexed_corpus,
};

pub mod analyze;
pub mod call_graph;
pub mod index;
pub mod references;
pub mod search;
pub mod serve;
pub mod setup;
pub mod status;

/// 检查索引数据目录是否完整（不自动创建——索引应由 `codeconnect index` 命令构建）
/// 返回 Ok(()) 或 Err(错误消息)
pub fn check_index_dirs_exist(data_dir: &Path) -> Result<(), String> {
    let tantivy_dir = data_dir.join("tantivy");
    let tantivy_edges_dir = data_dir.join("tantivy_edges");
    let sled_dir = data_dir.join("sled");

    let required: [(&str, &Path); 3] = [
        ("tantivy", &tantivy_dir),
        ("调用边索引", &tantivy_edges_dir),
        ("sled", &sled_dir),
    ];
    let mut missing = Vec::new();
    for (name, dir) in &required {
        if !dir.exists() {
            missing.push(*name);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "索引数据不完整，缺失: {}\n请先运行 `codeconnect index` 构建索引。",
            missing.join(", ")
        ))
    }
}

// ============================================================================
// 文本真值回显 — search / references 共用
// ============================================================================

/// 打开已构建的 sled 存储（只读；不存在时返回 None）
///
/// 同一个进程里对同一路径重复 `sled::open` 会失败，因此调用方只应打开一次，
/// 再把句柄传给 [`scan_text_truth`]。
pub fn open_sled(data_dir: &Path) -> Option<SledStore> {
    SledStore::open_only(&data_dir.join("sled")).ok()
}

/// 在已索引文件集内做文本扫描，取得「文本真值」
///
/// 与 MCP 侧走同一个入口（`scan_indexed_corpus`），范围与语义完全一致：
/// 只扫索引已知的文件，不做全盘遍历。
/// 返回 `None` 表示这层信息不可用（索引已知文件集为空），
/// 此时按「无文本信息」处理 —— 不伪造「文本里也没有」。
///
/// `sled` 由调用方传入已打开的句柄（见 [`open_sled`]），避免同一 DB 被二次打开。
pub fn scan_text_truth(
    project_root: &Path,
    sled: &SledStore,
    query: &str,
) -> Option<TextScanReport> {
    let files = indexed_file_paths(sled);
    scan_indexed_corpus(project_root, &files, query, TEXT_DETAIL_LIMIT)
}

/// 打印文本真值提示块（渐进披露，与 MCP 侧同阈值、同上限）
///
/// - 符号命中充足（≥ [`TEXT_SUMMARY_THRESHOLD`]）→ 只打印一行汇总
/// - 符号命中稀少或为 0 → 展开明细，并把「这不代表不存在」说在前面
pub fn print_text_truth(
    text: Option<&TextScanReport>,
    context: TextTruthContext,
    symbol_hits: usize,
) {
    let Some(report) = text else {
        return;
    };

    println!();

    if let Some(coverage) = report.coverage_warning() {
        println!("⚠ {}", coverage);
    }

    if let Some(notice) = report.text_truth_notice(context, symbol_hits) {
        println!("{}", notice);
    }

    // 明细被截断时给出真实总数，与 MCP 侧同口径
    if symbol_hits < TEXT_SUMMARY_THRESHOLD && report.total_hits > report.hits.len() {
        println!(
            "文本命中共 {} 处，本次仅显示前 {} 处（已截断）。文本明细条数上限固定为 {}。",
            report.total_hits,
            report.hits.len(),
            TEXT_DETAIL_LIMIT
        );
    }
}
