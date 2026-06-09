//! 三级依赖图
//!
//! 构建文件级、符号级、模块级三级依赖关系图，
//! 支持逐级展开和聚合分析。
//!
//! # 依赖图级别
//!
//! - **文件级** — 节点=文件路径，边=import 关系，从 sled 的 `imports:` 命名空间扫描
//! - **符号级** — 节点=symbol stable_id，边=调用/引用关系，从 sled 的 `edges:` 和 `refs:` 命名空间扫描
//! - **模块级** — 节点=模块/包名，边=跨模块引用，从文件级图聚合/从符号级图按 parent 聚合生成
//!
//! # 示例
//!
//! ```ignore
//! use codeconnect_index::sled_store::SledStore;
//! use codeconnect_graph::dep_graph::DependencyGraph;
//!
//! let graph = DependencyGraph::build_file_graph(&sled_store)?;
//! let deps = graph.get_dependencies("src/main.rs");
//! let dependents = graph.get_dependents("src/lib.rs");
//! ```

use std::collections::HashMap;

use petgraph::graph::{DiGraph, NodeIndex};

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::types::Import;

/// 依赖图中节点的类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepNodeKind {
    /// 文件节点
    File,
    /// 符号节点
    Symbol,
    /// 模块节点
    Module,
}

/// 依赖图中的节点
///
/// `id` 根据 `kind` 的不同有不同的含义：
/// - File → 相对于项目根目录的文件路径
/// - Symbol → 符号的 stable_id
/// - Module → 模块/包的限定名
#[derive(Debug, Clone)]
pub struct DepNode {
    /// 节点的唯一标识（文件路径/模块名/symbol_id）
    pub id: String,
    /// 节点显示名称
    pub name: String,
    /// 节点类型
    pub kind: DepNodeKind,
}

/// 依赖图中的边
#[derive(Debug, Clone)]
pub struct DepEdge {
    /// 边的类型（import/call/reference）
    pub edge_type: String,
    /// 该类型边的数量（聚合计数）
    pub count: u64,
}

/// 三级依赖关系图
///
/// 基于 petgraph 的有向图，节点是 [`DepNode`]，边是 [`DepEdge`]。
/// 内部维护一个 `node_map` 用于通过字符串 ID 快速定位节点索引。
pub struct DependencyGraph {
    /// petgraph 有向图
    pub(crate) graph: DiGraph<DepNode, DepEdge>,
    /// 节点 ID → 节点索引 的映射，用于 O(1) 查找
    pub(crate) node_map: HashMap<String, NodeIndex>,
}

impl DependencyGraph {
    // ========================================================================
    // 构造方法
    // ========================================================================

    /// 创建空的依赖图
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            node_map: HashMap::new(),
        }
    }

    /// 从 sled 的 imports 命名空间构建文件级依赖图
    ///
    /// 扫描所有以 `imports:` 为前缀的键，解析导入关系，
    /// 构建文件→被导入文件/模块的边。
    ///
    /// 键格式：`imports:{file_path}::{import_path}`
    /// 值格式：`Import` 的 JSON 序列化字节
    pub fn build_file_graph(
        sled: &codeconnect_index::sled_store::SledStore,
    ) -> Result<Self, CodeConnectError> {
        let mut graph = Self::new();

        // 扫描 imports 命名空间下所有键
        let imports = sled.scan_prefix(b"imports:");
        for item in imports {
            let (key_bytes, value_bytes) = item?;
            let key = String::from_utf8_lossy(&key_bytes).to_string();

            // 解析键：格式为 "imports:{file_path}::{import_path}"
            let key_without_prefix = match key.strip_prefix("imports:") {
                Some(rest) => rest,
                None => continue,
            };

            let parts: Vec<&str> = key_without_prefix.splitn(2, "::").collect();
            if parts.len() != 2 {
                continue;
            }

            let source_file = parts[0].to_string();
            let _import_path = parts[1].to_string();

            // 反序列化导入信息
            let import: Import = match serde_json::from_slice(&value_bytes) {
                Ok(import) => import,
                Err(_) => continue,
            };

            // 解析目标：根据 ImportResolution 确定目标文件或模块
            let target = Self::resolve_import_target(&import);
            if target.is_empty() {
                continue;
            }

            // 添加节点和边
            let source_idx = graph.get_or_create_node(
                &source_file,
                &source_file,
                DepNodeKind::File,
            );
            let target_idx = graph.get_or_create_node(
                &target,
                &target,
                DepNodeKind::File,
            );

            // 检查是否已存在相同类型的边，如果存在则计数+1
            if let Some(existing_edge) = graph.graph.find_edge(source_idx, target_idx) {
                if let Some(edge) = graph.graph.edge_weight_mut(existing_edge) {
                    edge.count += 1;
                }
            } else {
                graph.graph.add_edge(
                    source_idx,
                    target_idx,
                    DepEdge {
                        edge_type: "import".to_string(),
                        count: 1,
                    },
                );
            }
        }

        Ok(graph)
    }

    /// 从 sled 的 edges 和 refs 命名空间构建符号级依赖图
    ///
    /// 扫描调用边和引用边，构建符号之间的依赖关系。
    pub fn build_symbol_graph(
        sled: &codeconnect_index::sled_store::SledStore,
    ) -> Result<Self, CodeConnectError> {
        let mut graph = Self::new();

        // 扫描调用边：键格式 "edges:{caller_id}::{callee_id}"
        let call_edges = sled.scan_prefix(b"edges:");
        for item in call_edges {
            let (key_bytes, _value_bytes) = item?;
            let key = String::from_utf8_lossy(&key_bytes).to_string();

            let key_without_prefix = match key.strip_prefix("edges:") {
                Some(rest) => rest,
                None => continue,
            };

            let parts: Vec<&str> = key_without_prefix.splitn(2, "::").collect();
            if parts.len() != 2 {
                continue;
            }

            let caller_id = parts[0].to_string();
            let callee_id = parts[1].to_string();

            let caller_idx = graph.get_or_create_node(
                &caller_id,
                &caller_id,
                DepNodeKind::Symbol,
            );
            let callee_idx = graph.get_or_create_node(
                &callee_id,
                &callee_id,
                DepNodeKind::Symbol,
            );

            if let Some(existing_edge) = graph.graph.find_edge(caller_idx, callee_idx) {
                if let Some(edge) = graph.graph.edge_weight_mut(existing_edge) {
                    edge.count += 1;
                }
            } else {
                graph.graph.add_edge(
                    caller_idx,
                    callee_idx,
                    DepEdge {
                        edge_type: "call".to_string(),
                        count: 1,
                    },
                );
            }
        }

        // 扫描引用边：键格式 "refs:{ref_id}::{target_id}"
        let ref_edges = sled.scan_prefix(b"refs:");
        for item in ref_edges {
            let (key_bytes, _value_bytes) = item?;
            let key = String::from_utf8_lossy(&key_bytes).to_string();

            let key_without_prefix = match key.strip_prefix("refs:") {
                Some(rest) => rest,
                None => continue,
            };

            let parts: Vec<&str> = key_without_prefix.splitn(2, "::").collect();
            if parts.len() != 2 {
                continue;
            }

            let ref_id = parts[0].to_string();
            let target_id = parts[1].to_string();

            let ref_idx = graph.get_or_create_node(
                &ref_id,
                &ref_id,
                DepNodeKind::Symbol,
            );
            let target_idx = graph.get_or_create_node(
                &target_id,
                &target_id,
                DepNodeKind::Symbol,
            );

            if let Some(existing_edge) = graph.graph.find_edge(ref_idx, target_idx) {
                if let Some(edge) = graph.graph.edge_weight_mut(existing_edge) {
                    edge.count += 1;
                }
            } else {
                graph.graph.add_edge(
                    ref_idx,
                    target_idx,
                    DepEdge {
                        edge_type: "reference".to_string(),
                        count: 1,
                    },
                );
            }
        }

        Ok(graph)
    }

    // ========================================================================
    // 查询方法
    // ========================================================================

    /// 获取某个节点的直接依赖（出边指向的节点列表）
    ///
    /// 即当前节点依赖于哪些其他节点。
    pub fn get_dependencies(&self, node_id: &str) -> Vec<DepNode> {
        let node_idx = match self.node_map.get(node_id) {
            Some(&idx) => idx,
            None => return Vec::new(),
        };

        self.graph
            .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
            .map(|idx| self.graph[idx].clone())
            .collect()
    }

    /// 获取某个节点的直接被依赖者（入边来源的节点列表）
    ///
    /// 即哪些其他节点依赖于当前节点。
    pub fn get_dependents(&self, node_id: &str) -> Vec<DepNode> {
        let node_idx = match self.node_map.get(node_id) {
            Some(&idx) => idx,
            None => return Vec::new(),
        };

        self.graph
            .neighbors_directed(node_idx, petgraph::Direction::Incoming)
            .map(|idx| self.graph[idx].clone())
            .collect()
    }

    /// 获取所有节点
    pub fn nodes(&self) -> Vec<DepNode> {
        self.graph.raw_nodes().iter().map(|n| n.weight.clone()).collect()
    }

    /// 获取所有边
    pub fn edges(&self) -> Vec<(DepNode, DepNode, DepEdge)> {
        self.graph
            .edge_indices()
            .filter_map(|edge_idx| {
                let (source, target) = self.graph.edge_endpoints(edge_idx)?;
                let edge = self.graph[edge_idx].clone();
                Some((
                    self.graph[source].clone(),
                    self.graph[target].clone(),
                    edge,
                ))
            })
            .collect()
    }

    /// 节点数量
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// 边数量
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// 获取底层 petgraph 图的引用（供 cycle_detect 等模块使用）
    pub fn inner_graph(&self) -> &DiGraph<DepNode, DepEdge> {
        &self.graph
    }

    // ========================================================================
    // 增删改方法
    // ========================================================================

    /// 添加节点
    ///
    /// 返回节点索引。如果同 ID 的节点已存在，则返回已有索引而不重复添加。
    pub fn add_node(&mut self, node: DepNode) -> NodeIndex {
        if let Some(&idx) = self.node_map.get(&node.id) {
            return idx;
        }
        let idx = self.graph.add_node(node.clone());
        self.node_map.insert(node.id.clone(), idx);
        idx
    }

    /// 添加边
    ///
    /// 如果 source 或 target 节点不存在，返回 `None`。
    /// 如果同方向边已存在，则计数 +1。
    pub fn add_edge(
        &mut self,
        source_id: &str,
        target_id: &str,
        edge: DepEdge,
    ) -> Option<()> {
        let source_idx = *self.node_map.get(source_id)?;
        let target_idx = *self.node_map.get(target_id)?;

        if let Some(existing) = self.graph.find_edge(source_idx, target_idx) {
            if let Some(existing_edge) = self.graph.edge_weight_mut(existing) {
                existing_edge.count += 1;
            }
        } else {
            self.graph.add_edge(source_idx, target_idx, edge);
        }
        Some(())
    }

    // ========================================================================
    // 内部辅助方法
    // ========================================================================

    /// 根据导入信息解析目标文件或模块名
    ///
    /// - `Resolved(path)` → 返回解析后的路径
    /// - `External(pkg)` → 返回包名作为模块标识
    /// - `Unresolved` → 返回空字符串表示无法解析
    fn resolve_import_target(import: &Import) -> String {
        use codeconnect_core::types::ImportResolution;
        match &import.resolution {
            ImportResolution::Resolved(path) => path.clone(),
            ImportResolution::External(pkg) => pkg.clone(),
            ImportResolution::Unresolved => String::new(),
        }
    }

    /// 获取或创建节点
    ///
    /// 如果节点已存在则返回已有索引，否则创建新节点。
    fn get_or_create_node(&mut self, id: &str, name: &str, kind: DepNodeKind) -> NodeIndex {
        if let Some(&idx) = self.node_map.get(id) {
            return idx;
        }

        let idx = self.graph.add_node(DepNode {
            id: id.to_string(),
            name: name.to_string(),
            kind,
        });
        self.node_map.insert(id.to_string(), idx);
        idx
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试空依赖图的基本操作
    #[test]
    fn test_empty_graph() {
        let graph = DependencyGraph::new();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.get_dependencies("nonexistent").is_empty());
        assert!(graph.get_dependents("nonexistent").is_empty());
    }

    /// 测试手动构建依赖图
    #[test]
    fn test_manual_graph_construction() {
        let mut graph = DependencyGraph::new();

        let a = DepNode {
            id: "a.rs".to_string(),
            name: "a.rs".to_string(),
            kind: DepNodeKind::File,
        };
        let b = DepNode {
            id: "b.rs".to_string(),
            name: "b.rs".to_string(),
            kind: DepNodeKind::File,
        };

        let a_idx = graph.graph.add_node(a);
        let b_idx = graph.graph.add_node(b);
        graph.node_map.insert("a.rs".to_string(), a_idx);
        graph.node_map.insert("b.rs".to_string(), b_idx);

        graph.graph.add_edge(
            a_idx,
            b_idx,
            DepEdge {
                edge_type: "import".to_string(),
                count: 1,
            },
        );

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);

        let deps = graph.get_dependencies("a.rs");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].id, "b.rs");

        let dependents = graph.get_dependents("b.rs");
        assert_eq!(dependents.len(), 1);
        assert_eq!(dependents[0].id, "a.rs");
    }

    /// 测试 get_or_create_node 的去重行为
    #[test]
    fn test_get_or_create_node_dedup() {
        let mut graph = DependencyGraph::new();

        let idx1 = graph.get_or_create_node("a.rs", "a.rs", DepNodeKind::File);
        let idx2 = graph.get_or_create_node("b.rs", "b.rs", DepNodeKind::File);

        // 重复创建同一节点应返回已有索引
        let idx1_again = graph.get_or_create_node("a.rs", "a.rs", DepNodeKind::File);

        assert_eq!(idx1, idx1_again);
        assert_ne!(idx1, idx2);
        assert_eq!(graph.node_count(), 2);
    }

    /// 测试边计数聚合
    #[test]
    fn test_edge_count_aggregation() {
        let mut graph = DependencyGraph::new();
        let a = graph.get_or_create_node("a.rs", "a.rs", DepNodeKind::File);
        let b = graph.get_or_create_node("b.rs", "b.rs", DepNodeKind::File);

        // 添加两条同类型边
        graph.graph.add_edge(a, b, DepEdge { edge_type: "import".to_string(), count: 1 });

        // 模拟第二次 import（在实际 build_file_graph 中会自动聚合）
        if let Some(edge_idx) = graph.graph.find_edge(a, b) {
            if let Some(edge) = graph.graph.edge_weight_mut(edge_idx) {
                edge.count += 1;
            }
        }

        if let Some(edge_idx) = graph.graph.find_edge(a, b) {
            assert_eq!(graph.graph[edge_idx].count, 2);
        }
    }

    /// 测试 resolve_import_target
    #[test]
    fn test_resolve_import_target() {
        use codeconnect_core::types::{Import, ImportResolution};

        // Resolved
        let import = Import {
            file_path: "src/main.rs".to_string(),
            import_path: "crate::lib".to_string(),
            alias: None,
            line: 1,
            resolution: ImportResolution::Resolved("src/lib.rs".to_string()),
        };
        assert_eq!(DependencyGraph::resolve_import_target(&import), "src/lib.rs");

        // External
        let import = Import {
            file_path: "src/main.rs".to_string(),
            import_path: "serde::Serialize".to_string(),
            alias: None,
            line: 1,
            resolution: ImportResolution::External("serde".to_string()),
        };
        assert_eq!(DependencyGraph::resolve_import_target(&import), "serde");

        // Unresolved
        let import = Import {
            file_path: "src/main.rs".to_string(),
            import_path: "missing::Module".to_string(),
            alias: None,
            line: 1,
            resolution: ImportResolution::Unresolved,
        };
        assert_eq!(DependencyGraph::resolve_import_target(&import), "");
    }

    /// 测试 nodes() 和 edges() 方法
    #[test]
    fn test_nodes_and_edges() {
        let mut graph = DependencyGraph::new();
        let a = graph.get_or_create_node("a.rs", "a.rs", DepNodeKind::File);
        let b = graph.get_or_create_node("b.rs", "b.rs", DepNodeKind::File);
        let c = graph.get_or_create_node("c.rs", "c.rs", DepNodeKind::File);

        graph.graph.add_edge(a, b, DepEdge { edge_type: "import".to_string(), count: 1 });
        graph.graph.add_edge(b, c, DepEdge { edge_type: "import".to_string(), count: 1 });

        let nodes = graph.nodes();
        assert_eq!(nodes.len(), 3);

        let edges = graph.edges();
        assert_eq!(edges.len(), 2);
    }

    /// 测试 get_dependencies 和 get_dependents 对不存在的节点返回空
    #[test]
    fn test_nonexistent_node() {
        let graph = DependencyGraph::new();
        assert!(graph.get_dependencies("nope").is_empty());
        assert!(graph.get_dependents("nope").is_empty());
    }

    /// 测试 Default trait
    #[test]
    fn test_default() {
        let graph = DependencyGraph::default();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }
}
