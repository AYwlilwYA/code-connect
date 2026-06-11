//! 变更影响分析服务
//!
//! 基于调用图的 BFS 传播分析，评估符号变更的影响范围。
//! 对每个变更符号向上追溯调用者，按距离分类为直接和传递影响。

use std::collections::HashSet;

use codeconnect_core::error::CodeConnectError;
use codeconnect_graph::call_graph::CallGraph;

/// 影响等级枚举
///
/// 根据调用者与变更符号的距离分类影响程度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpactLevel {
    /// 直接调用者（距离 = 1）
    Direct,
    /// 传递调用者（距离 >= 2）
    Transitive,
}

/// 单个受影响的条目
///
/// 描述某个符号被变更波及的具体信息。
#[derive(Debug, Clone)]
pub struct ImpactEntry {
    /// 受影响的符号 ID
    pub symbol_id: String,
    /// 受影响的符号名称
    pub name: String,
    /// 距离变更符号的调用链深度（1 = 直接调用者）
    pub distance: usize,
    /// 影响等级
    pub level: ImpactLevel,
    /// 触发此影响的变更符号 ID
    pub caused_by: String,
}

/// 影响分析报告
///
/// 汇总所有变更符号的影响面分析结果。
#[derive(Debug, Clone)]
pub struct ImpactReport {
    /// 被分析的变更符号 ID 列表
    pub changed_symbols: Vec<String>,
    /// 直接调用者（距离 = 1）
    pub direct_impacts: Vec<ImpactEntry>,
    /// 传递调用者（距离 >= 2）
    pub transitive_impacts: Vec<ImpactEntry>,
    /// 所有受影响的唯一符号 ID 集合
    pub all_affected_ids: HashSet<String>,
}

impl ImpactReport {
    /// 创建空的影响报告
    fn new(changed_symbols: Vec<String>) -> Self {
        Self {
            changed_symbols,
            direct_impacts: Vec::new(),
            transitive_impacts: Vec::new(),
            all_affected_ids: HashSet::new(),
        }
    }

    /// 受影响符号总数（去重后）
    pub fn total_affected(&self) -> usize {
        self.all_affected_ids.len()
    }
}

/// 变更影响分析器
///
/// 从 sled 存储或已有调用图构建，对一组变更符号执行 BFS 调用者追溯，
/// 按距离分类为直接和传递影响，生成结构化的影响分析报告。
pub struct ImpactAnalyzer {
    /// 调用图实例
    graph: CallGraph,
    /// BFS 最大深度（默认 5）
    max_depth: usize,
}

impl ImpactAnalyzer {
    /// 从 sled 存储构建影响分析器（已废弃）
    ///
    /// 扫描 sled 中的调用边，构建完整调用图。
    ///
    /// # 参数
    /// - `sled` — 已打开的 sled 数据库实例
    /// - `max_depth` — BFS 最大搜索深度
    ///
    /// # 已废弃
    /// 调用边已迁入 tantivy 调用边索引。请使用 [`from_graph`] 配合
    /// `CallGraph::build_from_tantivy_edges` 来构建。
    #[deprecated(note = "调用边已迁入 tantivy，请使用 from_graph 配合 CallGraph::build_from_tantivy_edges")]
    pub fn new(
        sled: &codeconnect_index::sled_store::SledStore,
        max_depth: usize,
    ) -> Result<Self, CodeConnectError> {
        let graph = CallGraph::build_from_sled(sled)?;
        Ok(Self { graph, max_depth })
    }

    /// 从已有调用图构建影响分析器
    ///
    /// # 参数
    /// - `graph` — 已构建好的调用图
    /// - `max_depth` — BFS 最大搜索深度
    pub fn from_graph(graph: CallGraph, max_depth: usize) -> Self {
        Self { graph, max_depth }
    }

    /// 分析变更影响面
    ///
    /// 对每个变更符号向上追溯调用者（谁调用了它），
    /// 将结果按距离分类：
    /// - **距离 = 1** → 直接调用者（`ImpactLevel::Direct`）
    /// - **距离 >= 2** → 传递调用者（`ImpactLevel::Transitive`）
    ///
    /// 同一个符号可能被多个变更符号触发，报告中只计入首次发现的记录。
    ///
    /// # 参数
    /// - `changed_symbols` — 变更符号的 ID 列表
    ///
    /// # 返回
    /// 结构化的影响分析报告。
    pub fn analyze(&self, changed_symbols: &[String]) -> ImpactReport {
        let mut report = ImpactReport::new(changed_symbols.to_vec());

        for symbol_id in changed_symbols {
            // 向上追溯调用者
            let callers = self.graph.trace_callers(symbol_id, self.max_depth);

            for caller in callers {
                // 去重：同一符号只计入首次发现
                if report.all_affected_ids.contains(&caller.symbol_id) {
                    continue;
                }
                report.all_affected_ids.insert(caller.symbol_id.clone());

                let entry = ImpactEntry {
                    symbol_id: caller.symbol_id.clone(),
                    name: caller.name.clone(),
                    distance: caller.depth,
                    level: if caller.depth == 1 {
                        ImpactLevel::Direct
                    } else {
                        ImpactLevel::Transitive
                    },
                    caused_by: symbol_id.clone(),
                };

                if caller.depth == 1 {
                    report.direct_impacts.push(entry);
                } else {
                    report.transitive_impacts.push(entry);
                }
            }
        }

        report
    }

    /// 设置 BFS 搜索的最大深度
    pub fn set_max_depth(&mut self, depth: usize) {
        self.max_depth = depth;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构建测试调用图：A → B → C → D
    fn build_test_graph() -> CallGraph {
        let mut graph = CallGraph::new();
        graph.add_edge_raw("A", "B");
        graph.add_edge_raw("B", "C");
        graph.add_edge_raw("C", "D");
        graph
    }

    #[test]
    fn test_direct_impact() {
        let graph = build_test_graph();
        let analyzer = ImpactAnalyzer::from_graph(graph, 5);

        // 变更 B → 直接影响者 A（直接调用者）
        let report = analyzer.analyze(&["B".to_string()]);

        assert_eq!(report.direct_impacts.len(), 1);
        assert_eq!(report.direct_impacts[0].name, "A");
        assert_eq!(report.direct_impacts[0].distance, 1);
        assert_eq!(report.transitive_impacts.len(), 0);
    }

    #[test]
    fn test_transitive_impact() {
        let graph = build_test_graph();
        let analyzer = ImpactAnalyzer::from_graph(graph, 5);

        // 变更 D → A,B,C 都是调用者，A 距离3（传递）, B 距离2（传递）, C 距离1（直接）
        let report = analyzer.analyze(&["D".to_string()]);

        // C 是直接调用者
        assert_eq!(report.direct_impacts.len(), 1);
        assert!(report.direct_impacts.iter().any(|e| e.name == "C"));

        // A 和 B 是传递调用者
        assert_eq!(report.transitive_impacts.len(), 2);
        let trans_names: Vec<&str> = report.transitive_impacts.iter().map(|e| e.name.as_str()).collect();
        assert!(trans_names.contains(&"A"));
        assert!(trans_names.contains(&"B"));
    }

    #[test]
    fn test_multiple_changed_symbols() {
        let graph = build_test_graph();
        let analyzer = ImpactAnalyzer::from_graph(graph, 5);

        // 同时变更 B 和 D
        let report = analyzer.analyze(&["B".to_string(), "D".to_string()]);

        // B 的直接调用者：A
        // D 的直接调用者：C
        // D 的传递调用者：A, B
        // A 被 B 和 D 两个变更都触发，但去重后只出现一次
        assert!(report.direct_impacts.iter().any(|e| e.name == "A")); // B 的直接调用者
        assert!(report.direct_impacts.iter().any(|e| e.name == "C")); // D 的直接调用者
        assert_eq!(report.direct_impacts.len(), 2);

        assert!(report.transitive_impacts.iter().any(|e| e.name == "B")); // D 的传递调用者
        // A 已在 direct 中发现，不会在 transitive 中重复
        assert_eq!(report.transitive_impacts.len(), 1);

        // 总共去重后受影响数：A, C, B = 3
        assert_eq!(report.total_affected(), 3);
    }

    #[test]
    fn test_empty_changes() {
        let graph = build_test_graph();
        let analyzer = ImpactAnalyzer::from_graph(graph, 5);

        let report = analyzer.analyze(&[]);
        assert!(report.direct_impacts.is_empty());
        assert!(report.transitive_impacts.is_empty());
        assert_eq!(report.total_affected(), 0);
    }

    #[test]
    fn test_max_depth_limit() {
        let graph = build_test_graph();
        // max_depth = 1，只查找直接调用者
        let analyzer = ImpactAnalyzer::from_graph(graph, 1);

        // 变更 D，但 max_depth=1 只能找到 C（直接调用者）
        let report = analyzer.analyze(&["D".to_string()]);

        assert_eq!(report.direct_impacts.len(), 1);
        assert_eq!(report.direct_impacts[0].name, "C");
        assert_eq!(report.transitive_impacts.len(), 0);
    }
}
