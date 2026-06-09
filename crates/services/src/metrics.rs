//! 代码质量指标服务
//!
//! 封装 [`MetricCalculator`]，提供单个符号的圈复杂度计算和
//! 批量代码质量指标统计。此模块是 graph::metrics 的薄封装。

use std::collections::HashMap;

use codeconnect_core::types::Symbol;
use codeconnect_graph::call_graph::CallGraph;
use codeconnect_graph::metrics::{CodeMetrics, MetricCalculator};
use codeconnect_graph::type_hierarchy::TypeHierarchy;

/// 代码质量指标服务
///
/// 封装 graph::metrics 的指标计算功能，提供面向服务层的统一接口。
pub struct MetricsService;

impl MetricsService {
    /// 计算单个符号的圈复杂度
    ///
    /// 优先使用 symbol.complexity 预计算值（来自解析器）。
    /// 如果预计算值不存在，则对 source 文本做词法级分支关键字统计。
    ///
    /// # 参数
    /// - `symbol` — 目标符号
    /// - `source` — 符号所在文件的源码内容（用于回退计算）
    ///
    /// # 返回
    /// 圈复杂度（基准值为 1，即无分支的函数）。
    pub fn cyclomatic_complexity(symbol: &Symbol, source: &str) -> u64 {
        MetricCalculator::compute_complexity(symbol, source)
    }

    /// 批量计算所有符号的代码质量指标
    ///
    /// 对每个符号计算：
    /// - 圈复杂度（来自预计算值或回退文本扫描）
    /// - fan_in（入度，被调用次数）
    /// - fan_out（出度，调用次数）
    /// - 继承深度（类型层次中的祖先数量）
    ///
    /// # 参数
    /// - `symbols` — 符号列表
    /// - `call_graph` — 调用图（用于计算出入度）
    /// - `type_hierarchy` — 类型层次图（用于计算继承深度）
    /// - `source_cache` — 可选的文件路径→源码内容映射，用于复杂度回退计算
    ///
    /// # 返回
    /// 每个符号的完整代码质量指标列表。
    pub fn compute_all(
        symbols: &[Symbol],
        call_graph: &CallGraph,
        type_hierarchy: &TypeHierarchy,
        source_cache: Option<&HashMap<String, String>>,
    ) -> Vec<CodeMetrics> {
        MetricCalculator::compute_all(symbols, call_graph, type_hierarchy, source_cache)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeconnect_core::types::{SourceLocation, SymbolKind};

    /// 创建测试用 Symbol
    /// 注意：id 与 name 保持一致，以便在调用图中按 id 查找节点
    fn make_symbol(name: &str, complexity: Option<u64>) -> Symbol {
        Symbol {
            id: name.to_string(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            location: SourceLocation {
                file_path: "test.rs".to_string(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            signature: None,
            doc_comment: None,
            parent_id: None,
            modifiers: vec![],
            is_exported: false,
            complexity,
        }
    }

    #[test]
    fn test_cyclomatic_complexity_uses_precomputed() {
        let symbol = make_symbol("my_fn", Some(42));
        // 即使 source 很简单，也应使用预计算值
        let complexity = MetricsService::cyclomatic_complexity(&symbol, "fn my_fn() { return 1; }");
        assert_eq!(complexity, 42);
    }

    #[test]
    fn test_cyclomatic_complexity_fallback() {
        let symbol = make_symbol("simple_fn", None);
        let source = r#"
            fn simple_fn(x: i32) -> i32 {
                if x > 0 {
                    return x;
                }
                x * 2
            }
        "#;
        let complexity = MetricsService::cyclomatic_complexity(&symbol, source);
        // 1（入口） + 1（if） = 2
        assert_eq!(complexity, 2);
    }

    #[test]
    fn test_cyclomatic_complexity_minimal() {
        let symbol = make_symbol("minimal", None);
        let source = "fn minimal() {}";
        let complexity = MetricsService::cyclomatic_complexity(&symbol, source);
        assert_eq!(complexity, 1);
    }

    #[test]
    fn test_compute_all_empty() {
        let symbols: Vec<Symbol> = vec![];
        let call_graph = CallGraph::new();
        let type_hierarchy = TypeHierarchy::new();

        let metrics = MetricsService::compute_all(&symbols, &call_graph, &type_hierarchy, None);
        assert!(metrics.is_empty());
    }

    #[test]
    fn test_compute_all_with_data() {
        let symbols = vec![
            make_symbol("main", Some(3)),
            make_symbol("helper", None),
        ];

        let mut call_graph = CallGraph::new();
        call_graph.add_edge_raw("main", "helper");

        let type_hierarchy = TypeHierarchy::new();

        let metrics = MetricsService::compute_all(&symbols, &call_graph, &type_hierarchy, None);
        assert_eq!(metrics.len(), 2);

        let main = metrics.iter().find(|m| m.name == "main").unwrap();
        assert_eq!(main.cyclomatic_complexity, 3);
        assert_eq!(main.fan_out, 1); // 调用了 helper
        assert_eq!(main.fan_in, 0);

        let helper = metrics.iter().find(|m| m.name == "helper").unwrap();
        assert_eq!(helper.cyclomatic_complexity, 1); // 默认值（无源码缓存）
        assert_eq!(helper.fan_in, 1); // 被 main 调用
        assert_eq!(helper.fan_out, 0);
    }
}
