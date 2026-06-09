//! 离线分析子命令
//!
//! 执行代码质量分析：圈复杂度、死代码检测、架构规则验证等。

use std::path::Path;

use codeconnect_core::types::Symbol;
use codeconnect_graph::call_graph::CallGraph;
use codeconnect_graph::metrics::MetricCalculator;
use codeconnect_graph::type_hierarchy::TypeHierarchy;
use codeconnect_index::sled_store::SledStore;

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

    let sled_dir = data_dir.join("sled");
    let sled = SledStore::open(&sled_dir)
        .map_err(|e| format!("无法打开 sled 存储: {}", e))?;

    // 收集所有符号
    let mut all_symbols: Vec<Symbol> = Vec::new();
    let prefix = "symbols:";
    for item in sled.scan_prefix(prefix.as_bytes()) {
        if let Ok((_key, value)) = item {
            if let Ok(sym) = serde_json::from_slice::<Symbol>(&value) {
                all_symbols.push(sym);
            }
        }
    }

    if all_symbols.is_empty() {
        println!("索引中没有符号数据，请先运行 `codeconnect index`");
        return Ok(());
    }

    println!("符号总数: {}", all_symbols.len());
    println!();

    // 构建调用图
    let call_graph = CallGraph::build_from_sled(&sled)
        .map_err(|e| format!("构建调用图失败: {}", e))?;

    let type_hierarchy = TypeHierarchy::new();

    match analyze_type {
        "complexity" | "metrics" | "all" => {
            // 计算圈复杂度排名
            println!("═══════════════════════════════════════════");
            println!("  圈复杂度分析（Top 10）");
            println!("═══════════════════════════════════════════");

            let metrics = MetricCalculator::compute_all(&all_symbols, &call_graph, &type_hierarchy);

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
            println!("═══════════════════════════════════════════");
            println!("  死代码检测");
            println!("═══════════════════════════════════════════");

            let all_names: Vec<String> = all_symbols.iter().map(|s| s.name.clone()).collect();
            let entries = vec!["main".to_string()];

            let dead = MetricCalculator::detect_dead_code(&all_names, &call_graph, &entries);

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
                    println!(
                        "  {} — 置信度: {} ({:.0}%)",
                        d.name,
                        confidence_str,
                        d.confidence * 100.0
                    );
                    println!("    {}", d.reason);
                }
                println!();
                println!("  共 {} 个可疑死代码条目", dead.len());
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
