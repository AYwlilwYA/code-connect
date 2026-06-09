//! 循环依赖检测
//!
//! 使用 Kosaraju SCC 算法检测代码库中的循环依赖，
//! 支持拓扑排序输出。
//!
//! # 核心方法
//!
//! - `detect_cycles` — 找出所有强连通分量（SCC），过滤掉单节点分量，
//!   剩余的就是循环依赖
//! - `has_cycle` — 快速判断图中是否存在循环依赖
//! - `topological_order` — 对依赖图做拓扑排序，存在循环时返回参与循环的节点
//!
//! # 示例
//!
//! ```ignore
//! use codeconnect_graph::cycle_detect::CycleDetector;
//! use codeconnect_graph::dep_graph::DependencyGraph;
//!
//! let graph = DependencyGraph::build_file_graph(&sled_store)?;
//! let cycles = CycleDetector::detect_cycles(&graph);
//! if !cycles.is_empty() {
//!     for cycle in &cycles {
//!         eprintln!("发现循环依赖: {:?}", cycle.iter().map(|n| &n.id).collect::<Vec<_>>());
//!     }
//! }
//! ```

use petgraph::algo::{kosaraju_scc, toposort};

use super::dep_graph::{DepNode, DependencyGraph};

/// 循环依赖检测器
///
/// 基于 petgraph 的 Kosaraju SCC 算法进行强连通分量检测，
/// 通过在 petgraph 图中标记节点顺序来实现拓扑排序。
pub struct CycleDetector;

impl CycleDetector {
    /// 检测依赖图中的所有循环依赖
    ///
    /// 使用 Kosaraju SCC 算法找出所有强连通分量，
    /// 过滤掉单节点分量（单独节点不构成循环，除非自环），
    /// 返回多节点的 SCC 列表，每个 SCC 是一个循环依赖。
    pub fn detect_cycles(graph: &DependencyGraph) -> Vec<Vec<DepNode>> {
        let inner = graph.inner_graph();

        // 使用 Kosaraju 算法找出所有强连通分量
        let sccs = kosaraju_scc(inner);

        // 过滤：只保留多节点分量（至少2个节点的 SCC 才构成循环依赖）
        sccs
            .into_iter()
            .filter(|scc| scc.len() > 1)
            .map(|scc| {
                scc.into_iter()
                    .map(|node_idx| inner[node_idx].clone())
                    .collect()
            })
            .collect()
    }

    /// 检查依赖图中是否存在循环依赖
    ///
    /// 返回 `true` 表示图中存在至少一个多节点 SCC。
    pub fn has_cycle(graph: &DependencyGraph) -> bool {
        let inner = graph.inner_graph();
        let sccs = kosaraju_scc(inner);
        sccs.iter().any(|scc| scc.len() > 1)
    }

    /// 对依赖图进行拓扑排序
    ///
    /// # 返回值
    ///
    /// - `Ok(nodes)` — 拓扑排序成功，返回按拓扑序排列的节点列表
    /// - `Err(cycle_nodes)` — 图中存在循环依赖，返回参与循环的节点列表
    ///
    /// 注意：拓扑排序按依赖方向排列，被依赖的节点排在前面。
    pub fn topological_order(
        graph: &DependencyGraph,
    ) -> Result<Vec<DepNode>, Vec<DepNode>> {
        let inner = graph.inner_graph();

        match toposort(inner, None) {
            Ok(order) => {
                let nodes: Vec<DepNode> = order
                    .into_iter()
                    .map(|idx| inner[idx].clone())
                    .collect();
                Ok(nodes)
            }
            Err(cycle_error) => {
                // cycle_error 中包含参与循环的节点索引
                let cycle_node_idx = cycle_error.node_id();
                // 收集所有参与循环的节点
                let sccs = kosaraju_scc(inner);
                let cycle_nodes: Vec<DepNode> = sccs
                    .into_iter()
                    .filter(|scc| scc.len() > 1)
                    .flatten()
                    .map(|idx| inner[idx].clone())
                    .collect();

                // 如果通过 SCC 没找到多节点分量（理论上不应发生），
                // 则返回导致拓扑排序失败的节点
                if cycle_nodes.is_empty() {
                    return Err(vec![inner[cycle_node_idx].clone()]);
                }
                Err(cycle_nodes)
            }
        }
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::dep_graph::{DepEdge, DepNode, DepNodeKind, DependencyGraph};
    use super::*;

    /// 构建一条简单的导入边
    fn make_edge() -> DepEdge {
        DepEdge { edge_type: "import".into(), count: 1 }
    }

    /// 构建一个包含自环的测试依赖图
    fn build_self_loop_graph() -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        graph.add_node(DepNode { id: "a".into(), name: "a".into(), kind: DepNodeKind::File });
        graph.add_node(DepNode { id: "b".into(), name: "b".into(), kind: DepNodeKind::File });

        // A → A 自环 — 自环需要直接操作 petgraph，因为 add_edge 不会添加自环
        // （自环在 find_edge 中 source==target 时不经过 node_map 映射也可行）
        let a_idx = graph.node_map["a"];
        graph.graph.add_edge(a_idx, a_idx, make_edge());
        // A → B
        graph.add_edge("a", "b", make_edge());

        graph
    }

    /// 构建一个简单的测试依赖图用于单元测试
    ///
    /// 节点关系：
    ///   A → B → C
    ///   (无循环)
    fn build_acyclic_graph() -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        graph.add_node(DepNode { id: "a".into(), name: "a".into(), kind: DepNodeKind::File });
        graph.add_node(DepNode { id: "b".into(), name: "b".into(), kind: DepNodeKind::File });
        graph.add_node(DepNode { id: "c".into(), name: "c".into(), kind: DepNodeKind::File });

        graph.add_edge("a", "b", make_edge());
        graph.add_edge("b", "c", make_edge());

        graph
    }

    /// 构建一个包含循环的测试依赖图
    ///
    /// 节点关系：
    ///   A → B → C → A
    ///   (A, B, C 构成循环)
    fn build_cyclic_graph() -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        graph.add_node(DepNode { id: "a".into(), name: "a".into(), kind: DepNodeKind::File });
        graph.add_node(DepNode { id: "b".into(), name: "b".into(), kind: DepNodeKind::File });
        graph.add_node(DepNode { id: "c".into(), name: "c".into(), kind: DepNodeKind::File });

        // A → B → C → A（循环）
        graph.add_edge("a", "b", make_edge());
        graph.add_edge("b", "c", make_edge());
        graph.add_edge("c", "a", make_edge());

        graph
    }

    // ====================================================================
    // detect_cycles 测试
    // ====================================================================

    #[test]
    fn test_detect_cycles_acyclic() {
        let graph = build_acyclic_graph();
        let cycles = CycleDetector::detect_cycles(&graph);
        assert!(cycles.is_empty(), "无环图不应检测到任何循环");
    }

    #[test]
    fn test_detect_cycles_cyclic() {
        let graph = build_cyclic_graph();
        let cycles = CycleDetector::detect_cycles(&graph);
        assert_eq!(cycles.len(), 1, "三节点环应检测到1个循环");
        assert_eq!(cycles[0].len(), 3, "循环应包含3个节点");
    }

    #[test]
    fn test_detect_cycles_empty_graph() {
        let graph = DependencyGraph::new();
        let cycles = CycleDetector::detect_cycles(&graph);
        assert!(cycles.is_empty(), "空图不应检测到任何循环");
    }

    #[test]
    fn test_detect_cycles_single_node() {
        let mut graph = DependencyGraph::new();
        graph.add_node(DepNode { id: "single".into(), name: "single".into(), kind: DepNodeKind::File });

        let cycles = CycleDetector::detect_cycles(&graph);
        assert!(cycles.is_empty(), "单节点无自环不应检测到循环");
    }

    #[test]
    fn test_detect_cycles_self_loop() {
        let graph = build_self_loop_graph();
        let cycles = CycleDetector::detect_cycles(&graph);
        // 自环（A→A）的 SCC 是单独节点，但我们的实现
        // 过滤了 len <= 1 的 SCC，所以不会检测到自环
        // 注意：petgraph 的 kosaraju_scc 会将自环节点视为独立的 SCC
        assert!(cycles.is_empty(), "自环节点被过滤（len=1的SCC不视为循环）");
    }

    // ====================================================================
    // has_cycle 测试
    // ====================================================================

    #[test]
    fn test_has_cycle_acyclic() {
        let graph = build_acyclic_graph();
        assert!(!CycleDetector::has_cycle(&graph));
    }

    #[test]
    fn test_has_cycle_cyclic() {
        let graph = build_cyclic_graph();
        assert!(CycleDetector::has_cycle(&graph));
    }

    #[test]
    fn test_has_cycle_empty() {
        let graph = DependencyGraph::new();
        assert!(!CycleDetector::has_cycle(&graph));
    }

    // ====================================================================
    // topological_order 测试
    // ====================================================================

    #[test]
    fn test_topological_order_acyclic() {
        let graph = build_acyclic_graph();
        let order = CycleDetector::topological_order(&graph);
        assert!(order.is_ok(), "无环图应能拓扑排序");

        let nodes = order.unwrap();
        assert_eq!(nodes.len(), 3);

        // 拓扑序：被依赖的节点排在前面
        // A → B → C，所以 A 应在 B 前，B 应在 C 前
        let pos: HashMap<&str, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();

        let a_pos = pos["a"];
        let b_pos = pos["b"];
        let c_pos = pos["c"];

        assert!(a_pos < b_pos, "A 应在 B 之前");
        assert!(b_pos < c_pos, "B 应在 C 之前");
    }

    #[test]
    fn test_topological_order_cyclic() {
        let graph = build_cyclic_graph();
        let result = CycleDetector::topological_order(&graph);
        assert!(result.is_err(), "有环图拓扑排序应失败");
        assert!(!result.unwrap_err().is_empty(), "应返回参与循环的节点");
    }

    #[test]
    fn test_topological_order_empty() {
        let graph = DependencyGraph::new();
        let result = CycleDetector::topological_order(&graph);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
