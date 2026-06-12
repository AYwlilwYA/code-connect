//! 离线分析子命令
//!
//! 执行代码质量分析：圈复杂度、死代码检测、架构规则验证等。

use std::collections::HashMap;
use std::path::Path;

use codeconnect_core::types::Symbol;
use codeconnect_graph::call_graph::CallGraph;
use codeconnect_graph::metrics::MetricCalculator;
use codeconnect_graph::type_hierarchy::TypeHierarchy;
use codeconnect_index::query_engine::symbol_search_result_to_symbol;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};

/// 执行离线分析
///
/// # 参数
///
/// - `project_root` — 项目根目录
/// - `data_dir` — 索引数据目录
/// - `analyze_type` — 分析类型（metrics、deadcode、complexity、all）
pub async fn run(
    project_root: &Path,
    data_dir: &Path,
    analyze_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = project_root;

    let tantivy_dir = data_dir.join("tantivy");
    let tantivy_edges_dir = data_dir.join("tantivy_edges");

    // 检查索引目录是否存在（不自动创建——索引应由 `codeconnect index` 命令构建）
    super::check_index_dirs_exist(data_dir)?;

    let tantivy = TantivyIndex::open_or_create(&tantivy_dir)
        .map_err(|e| format!("无法打开 tantivy 索引: {}", e))?;
    let call_edge_index = CallEdgeIndex::open_or_create(&tantivy_edges_dir)
        .map_err(|e| format!("无法打开调用边索引: {}", e))?;
    let _sled_dir = data_dir.join("sled");

    // 从 tantivy 扫描所有符号（不从 sled 扫描符号数据）
    let all_ids = tantivy.scan_all_ids()
        .map_err(|e| format!("扫描符号 ID 失败: {}", e))?;

    // 收集所有 Symbol（从 tantivy 搜索结果）
    let mut all_symbols: Vec<Symbol> = Vec::new();
    // 对每个 ID 通过 tantivy 精确搜索获取完整信息
    for (stable_id, _name) in &all_ids {
        if let Ok(Some(result)) = tantivy.search_by_id(stable_id) {
            all_symbols.push(symbol_search_result_to_symbol(&result));
        }
    }

    if all_symbols.is_empty() {
        println!("索引中没有符号数据，请先运行 `codeconnect index`");
        return Ok(());
    }

    println!("符号总数: {}", all_symbols.len());
    println!();

    // 构建调用图（从 tantivy 符号索引 + tantivy 调用边索引）
    let call_graph = CallGraph::build_from_tantivy_edges(&call_edge_index, &all_ids)
        .map_err(|e| format!("构建调用图失败: {}", e))?;

    let type_hierarchy = TypeHierarchy::new();

    match analyze_type {
        "complexity" | "metrics" | "all" => {
            // 计算圈复杂度排名
            println!("═══════════════════════════════════════════");
            println!("  圈复杂度分析（Top 10）");
            println!("═══════════════════════════════════════════");

            // 构建源码缓存：收集所有符号涉及到的唯一文件路径，读取源码
            // 用于复杂度回退计算（当解析器未预计算 complexity 时）
            let mut source_cache: HashMap<String, String> = HashMap::new();
            for sym in &all_symbols {
                let fp = &sym.location.file_path;
                if !source_cache.contains_key(fp) {
                    let full_path = project_root.join(fp);
                    if let Ok(content) = std::fs::read_to_string(&full_path) {
                        source_cache.insert(fp.clone(), content);
                    }
                }
            }

            let metrics = MetricCalculator::compute_all(
                &all_symbols,
                &call_graph,
                &type_hierarchy,
                Some(&source_cache),
            );

            let mut sorted: Vec<_> = metrics.iter().collect();
            sorted.sort_by(|a, b| b.cyclomatic_complexity.cmp(&a.cyclomatic_complexity));

            for (i, m) in sorted.iter().take(10).enumerate() {
                let marker = if m.cyclomatic_complexity > 30 {
                    " 🔴"
                } else if m.cyclomatic_complexity > 15 {
                    " 🟡"
                } else {
                    ""
                };
                println!(
                    "  {}. {} — 复杂度: {}{} (入:{} / 出:{})",
                    i + 1,
                    m.name,
                    m.cyclomatic_complexity,
                    marker,
                    m.fan_in,
                    m.fan_out,
                );
            }

            // 全局统计
            let total_complexity: u64 = metrics.iter().map(|m| m.cyclomatic_complexity).sum();
            let avg_complexity = if metrics.is_empty() {
                0.0
            } else {
                total_complexity as f64 / metrics.len() as f64
            };
            let high_complexity = metrics
                .iter()
                .filter(|m| m.cyclomatic_complexity > 15)
                .count();
            let very_high = metrics
                .iter()
                .filter(|m| m.cyclomatic_complexity > 30)
                .count();

            println!();
            println!("  全局统计:");
            println!("    总符号数:     {}", metrics.len());
            println!("    平均复杂度:   {:.1}", avg_complexity);
            println!("    高复杂度(>15): {}", high_complexity);
            println!("    极高复杂度(>30): {}", very_high);
            println!();
        }

        _ => {}
    }

    // 死代码检测（第二个阶段，在 complexity 结果之后按条件运行）
    if analyze_type == "deadcode" || analyze_type == "all" {
            // 死代码检测
            use codeconnect_core::types::SymbolKind;
            println!("═══════════════════════════════════════════");
            println!("  死代码检测");
            println!("═══════════════════════════════════════════");

            // 只对可调用的符号检测死代码（Function / Method），
            // 排除 Field、Variable、Parameter、Struct 字段等不可调用符号，
            // 因为调用图中没有这些符号的边，它们会被错误标记为死代码
            let callable_symbols: Vec<&Symbol> = all_symbols
                .iter()
                .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
                .collect();

            // 使用符号稳定 ID 构建符号列表
            let all_symbol_ids: Vec<String> = callable_symbols
                .iter()
                .map(|s| s.id.clone())
                .collect();

            // 构建 id → name 映射，用于后续显示
            let id_to_name: std::collections::HashMap<String, String> = callable_symbols
                .iter()
                .map(|s| (s.id.clone(), s.name.clone()))
                .collect();

            // 入口点：
            // 1. 所有名为 "main" 的函数
            // 2. 所有 is_exported 的可调用符号（pub 函数/方法）
            let mut entries: Vec<String> = callable_symbols
                .iter()
                .filter(|s| s.name == "main" || s.is_exported)
                .map(|s| s.id.clone())
                .collect();

            // 如果没有找到任何入口点，回退：
            // 取所有 callable 符号中第一个作为入口点
            if entries.is_empty() {
                entries = callable_symbols
                    .first()
                    .map(|s| vec![s.id.clone()])
                    .unwrap_or_default();
            }

            println!("  可调用符号数: {}", callable_symbols.len());
            println!("  入口点数: {}", entries.len());
            println!();

            let dead = MetricCalculator::detect_dead_code(&all_symbol_ids, &call_graph, &entries);

            if dead.is_empty() {
                println!("  未检测到死代码");
            } else {
                for d in &dead {
                    let confidence_str = if d.confidence >= 0.8 {
                        "高"
                    } else if d.confidence >= 0.5 {
                        "中"
                    } else {
                        "低"
                    };
                    // 用 id_to_name 映射将 StableSymbolId 反查为原始名称
                    let display_name = id_to_name.get(&d.name).cloned().unwrap_or_else(|| d.name.clone());
                    println!(
                        "  {} — 置信度: {} ({:.0}%)",
                        display_name,
                        confidence_str,
                        d.confidence * 100.0
                    );
                    println!("    {}", d.reason);
                }
                println!();
                println!("  共 {} 个可疑死代码条目（仅函数/方法）", dead.len());
            }
            println!();
    }

    Ok(())
}

/// 运行架构规则检查（CI 适用）
///
/// # 参数
///
/// - `project_root` — 项目根目录
/// - `data_dir` — 索引数据目录
/// - `rules` — 要检查的规则名称列表
pub async fn run_check_rules(
    project_root: &Path,
    data_dir: &Path,
    rules: Option<Vec<String>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = project_root;
    let _ = data_dir;

    println!("架构规则验证");
    println!("═══════════════════════════════════════════");

    if let Some(rule_names) = rules {
        println!("  检查规则: {:?}", rule_names);
    } else {
        println!("  检查所有配置规则");
    }

    // TODO: 实现完整的架构规则验证引擎
    println!();
    println!("  状态: 通过（尚未集成完整规则引擎）");
    println!("  提示: 在 .codeconnect.toml 中配置 [rules] 段来定义架构约束");

    Ok(())
}
