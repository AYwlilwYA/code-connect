//! 调用链分析服务
//!
//! 封装 [`CallGraph`]，提供调用者/被调用者的双向 BFS 遍历分析，
//! 以及基于调用图的全量引用查找。
//!
//! 调用图从 tantivy 调用边索引构建，`CallAnalyzer` 持有预构建的图实例，
//! 避免每次查询都重新扫描存储。

use codeconnect_core::error::CodeConnectError;
use codeconnect_graph::call_graph::{CallChainNode, CallGraph};

/// 调用链分析器
///
/// 基于调用图的符号间调用关系分析服务。
/// 内部持有从 sled 存储预构建的 [`CallGraph`] 实例，
/// 提供调用者追溯、被调用者追溯和引用查找三类分析操作。
pub struct CallAnalyzer {
    /// 预构建的调用图
    graph: CallGraph,
}

impl CallAnalyzer {
    /// 从 sled 存储构建调用分析器（已废弃）
    ///
    /// 扫描 sled 中的 edges 命名空间，构建完整的调用图。
    /// 此操作会遍历所有调用边和符号，对于大型项目可能需要一定时间。
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
        let graph = CallGraph::build_from_sled(sled)?;
        Ok(Self { graph })
    }

    /// 直接使用已有的调用图创建分析器
    ///
    /// 用于从外部预构建的图实例化，避免重复构建。
    ///
    /// # 参数
    /// - `graph` — 已构建好的调用图
    pub fn from_graph(graph: CallGraph) -> Self {
        Self { graph }
    }

    // =========================================================================
    // 调用者追溯
    // =========================================================================

    /// 向上追溯调用者链条
    ///
    /// 从目标符号出发，沿"谁调用了它"方向进行 BFS，
    /// 返回所有直接和间接调用者。
    ///
    /// # 参数
    /// - `symbol_id` — 目标符号 ID
    /// - `max_depth` — 最大搜索深度（1 = 直接调用者，2 = 间接调用者，以此类推）
    ///
    /// # 返回
    /// 调用链节点列表，按 BFS 发现顺序排列，不包含起始符号自身。
    /// 如果符号不在图中，返回空列表。
    pub fn trace_callers(
        &self,
        symbol_id: &str,
        max_depth: usize,
    ) -> Vec<CallChainNode> {
        self.graph.trace_callers(symbol_id, max_depth)
    }

    // =========================================================================
    // 被调用者追溯
    // =========================================================================

    /// 向下追溯被调用者链条
    ///
    /// 从目标符号出发，沿"它调用了谁"方向进行 BFS，
    /// 返回所有直接和间接被调用者。
    ///
    /// # 参数
    /// - `symbol_id` — 目标符号 ID
    /// - `max_depth` — 最大搜索深度（1 = 直接被调用者，2 = 间接被调用者，以此类推）
    ///
    /// # 返回
    /// 调用链节点列表，按 BFS 发现顺序排列，不包含起始符号自身。
    /// 如果符号不在图中，返回空列表。
    pub fn trace_callees(
        &self,
        symbol_id: &str,
        max_depth: usize,
    ) -> Vec<CallChainNode> {
        self.graph.trace_callees(symbol_id, max_depth)
    }

    // =========================================================================
    // 引用查找
    // =========================================================================

    /// 查找所有引用目标符号的调用者（简称）
    ///
    /// 即获取所有入边来源的符号 ID。
    /// 等价于 `trace_callers` 的结果但仅返回 ID 列表。
    ///
    /// # 参数
    /// - `symbol_id` — 目标符号 ID
    pub fn find_references(&self, symbol_id: &str) -> Vec<String> {
        self.graph.get_callers(symbol_id)
    }

    // =========================================================================
    // 辅助方法
    // =========================================================================

    /// 获取调用图的节点数
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// 获取调用图的边数
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// 获取指定符号的出入度
    ///
    /// 返回 `(fan_in, fan_out)`：
    /// - `fan_in` — 被多少个符号调用
    /// - `fan_out` — 调用了多少个符号
    pub fn degree(&self, symbol_id: &str) -> (u64, u64) {
        self.graph.degree(symbol_id)
    }

    /// 获取底层调用图的不可变引用
    ///
    /// 供需要直接访问图数据的场景使用。
    pub fn inner_graph(&self) -> &CallGraph {
        &self.graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeconnect_graph::call_graph::CallGraph;

    /// 测试从已有图构建分析器
    #[test]
    fn test_from_graph() {
        let graph = CallGraph::new();
        let analyzer = CallAnalyzer::from_graph(graph);
        assert_eq!(analyzer.node_count(), 0);
        assert_eq!(analyzer.edge_count(), 0);
    }

    /// 测试不存在的符号
    #[test]
    fn test_nonexistent_symbol() {
        let graph = CallGraph::new();
        let analyzer = CallAnalyzer::from_graph(graph);
        let callers = analyzer.trace_callers("ghost", 10);
        assert!(callers.is_empty());
        let callees = analyzer.trace_callees("ghost", 10);
        assert!(callees.is_empty());
        let refs = analyzer.find_references("ghost");
        assert!(refs.is_empty());
    }

    /// 测试完整的调用者追溯
    #[test]
    fn test_trace_callers_chain() {
        let mut graph = CallGraph::new();
        // 构建调用链: main → helper → util
        graph.add_edge_raw("main", "helper");
        graph.add_edge_raw("helper", "util");

        let analyzer = CallAnalyzer::from_graph(graph);

        let callers = analyzer.trace_callers("util", 10);
        let names: Vec<String> = callers.iter().map(|c| c.name.clone()).collect();
        assert!(names.contains(&"helper".to_string()));
        assert!(names.contains(&"main".to_string()));

        // helper 应为深度 1（直接调用者）
        let helper = callers.iter().find(|c| c.name == "helper").unwrap();
        assert_eq!(helper.depth, 1);
    }

    /// 测试被调用者追溯
    #[test]
    fn test_trace_callees_chain() {
        let mut graph = CallGraph::new();
        graph.add_edge_raw("main", "helper");
        graph.add_edge_raw("helper", "util");

        let analyzer = CallAnalyzer::from_graph(graph);

        let callees = analyzer.trace_callees("main", 10);
        let names: Vec<String> = callees.iter().map(|c| c.name.clone()).collect();
        assert!(names.contains(&"helper".to_string()));
        assert!(names.contains(&"util".to_string()));

        // helper 应为深度 1
        let helper = callees.iter().find(|c| c.name == "helper").unwrap();
        assert_eq!(helper.depth, 1);
    }

    /// 测试查找引用（入边来源）
    #[test]
    fn test_find_references() {
        let mut graph = CallGraph::new();
        graph.add_edge_raw("a", "c");
        graph.add_edge_raw("b", "c");

        let analyzer = CallAnalyzer::from_graph(graph);

        let refs = analyzer.find_references("c");
        assert_eq!(refs.len(), 2);
        assert!(refs.contains(&"a".to_string()));
        assert!(refs.contains(&"b".to_string()));
    }
}
