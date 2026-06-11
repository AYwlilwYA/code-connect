//! 死代码检测服务
//!
//! 对代码库做可达性分析，从入口点出发遍历调用图，
//! 标记从任何入口点都不可达的符号为候选死代码。

use codeconnect_core::error::CodeConnectError;
use codeconnect_graph::call_graph::CallGraph;
use codeconnect_graph::metrics::{DeadCodeEntry, MetricCalculator};

/// 死代码检测服务
///
/// 封装 [`MetricCalculator::detect_dead_code`]，从 sled 存储或已有调用图构建。
/// 通过 BFS 可达性分析找出从入口点不可达的符号。
///
/// # 算法
/// 从入口点（如 main 函数、导出的 pub 函数）出发，沿调用图的出边
/// 方向做正向 BFS，标记所有可达符号。未被标记的符号即为死代码候选。
pub struct DeadCodeDetector {
    /// 调用图实例
    call_graph: CallGraph,
}

impl DeadCodeDetector {
    /// 从 sled 存储构建死代码检测器（已废弃）
    ///
    /// # 参数
    /// - `sled` — 已打开的 sled 数据库实例
    ///
    /// # 已废弃
    /// 调用边已迁入 tantivy 调用边索引。请使用 [`from_graph`] 配合
    /// `CallGraph::build_from_tantivy_edges` 来构建。
    #[deprecated(note = "调用边已迁入 tantivy，请使用 from_graph 配合 CallGraph::build_from_tantivy_edges")]
    pub fn new(
        sled: &codeconnect_index::sled_store::SledStore,
    ) -> Result<Self, CodeConnectError> {
        let call_graph = CallGraph::build_from_sled(sled)?;
        Ok(Self { call_graph })
    }

    /// 从已有调用图构建死代码检测器
    ///
    /// # 参数
    /// - `call_graph` — 已构建好的调用图
    pub fn from_graph(call_graph: CallGraph) -> Self {
        Self { call_graph }
    }

    /// 执行死代码检测
    ///
    /// 从指定的入口点列表出发，在调用图上做正向 BFS 可达性分析。
    /// 所有不可达的符号会被标记为死代码，附带置信度和原因描述。
    ///
    /// # 参数
    /// - `all_symbols` — 所有符号的名称列表
    /// - `entry_points` — 入口点符号名称列表（如 `["main", "pub_api"]`）
    ///
    /// # 返回
    /// 死代码条目列表，按置信度降序排列（最可能是死代码的在前）。
    ///
    /// # 置信度
    /// - 1.0 — 完全不可达且无入边（高度确定性）
    /// - 0.8 — 不可达但有入边（可能是反射/动态调用）
    /// - 0.7 — 不可达但名称以 `_` 开头（暗示内部函数）
    /// - 0.5 — 不可达但名称匹配测试模式（`test_*` 或 `*_test`）
    pub fn detect(
        &self,
        all_symbols: &[String],
        entry_points: &[String],
    ) -> Vec<DeadCodeEntry> {
        let mut entries = MetricCalculator::detect_dead_code(
            all_symbols,
            &self.call_graph,
            entry_points,
        );

        // 按置信度降序排列：最可能是死代码的排前面
        entries.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        entries
    }

    /// 获取调用图的节点数
    pub fn node_count(&self) -> usize {
        self.call_graph.node_count()
    }

    /// 获取调用图的边数
    pub fn edge_count(&self) -> usize {
        self.call_graph.edge_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构建测试调用图
    fn build_test_graph() -> CallGraph {
        let mut graph = CallGraph::new();
        // main → helper → util
        // orphan → dead_func（孤立子图）
        graph.add_edge_raw("main", "helper");
        graph.add_edge_raw("helper", "util");
        graph.add_edge_raw("orphan", "dead_func");
        graph
    }

    #[test]
    fn test_detect_dead_code() {
        let graph = build_test_graph();
        let detector = DeadCodeDetector::from_graph(graph);

        let all_symbols: Vec<String> = vec![
            "main".into(),
            "helper".into(),
            "util".into(),
            "orphan".into(),
            "dead_func".into(),
        ];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = detector.detect(&all_symbols, &entry_points);

        // orphan 和 dead_func 不可达
        assert_eq!(dead.len(), 2);
        let dead_names: Vec<&str> = dead.iter().map(|d| d.name.as_str()).collect();
        assert!(dead_names.contains(&"orphan"));
        assert!(dead_names.contains(&"dead_func"));
    }

    #[test]
    fn test_no_entry_points() {
        let graph = build_test_graph();
        let detector = DeadCodeDetector::from_graph(graph);

        let all_symbols: Vec<String> = vec!["main".into(), "helper".into()];
        let entry_points: Vec<String> = vec![];

        // 没有入口点 → 所有符号都不可达
        let dead = detector.detect(&all_symbols, &entry_points);
        assert_eq!(dead.len(), 2);
    }

    #[test]
    fn test_all_reachable() {
        let mut graph = CallGraph::new();
        graph.add_edge_raw("main", "helper");
        graph.add_edge_raw("helper", "util");

        let detector = DeadCodeDetector::from_graph(graph);

        let all_symbols: Vec<String> = vec!["main".into(), "helper".into(), "util".into()];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = detector.detect(&all_symbols, &entry_points);
        assert!(dead.is_empty());
    }

    #[test]
    fn test_sorted_by_confidence_desc() {
        let mut graph = CallGraph::new();
        graph.add_edge_raw("orphan", "dead_func");

        let detector = DeadCodeDetector::from_graph(graph);

        let all_symbols: Vec<String> = vec![
            "main".into(),
            "orphan".into(),
            "dead_func".into(),
            "_internal".into(),
        ];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = detector.detect(&all_symbols, &entry_points);

        // 验证按置信度降序排列
        for i in 1..dead.len() {
            assert!(
                dead[i - 1].confidence >= dead[i].confidence,
                "结果应按置信度降序排列"
            );
        }
    }
}
