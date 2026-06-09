//! 符号级调用图
//!
//! 基于 petgraph 构建的符号间调用关系图，
//! 支持 BFS 遍历、双向查询（调用者/被调用者）。
//!
//! # 核心结构
//! - [`SymbolNode`] — 图节点，精简的符号信息
//! - [`CallGraph`] — petgraph DiGraph 封装，带有 id→NodeIndex 映射
//! - [`CallChainNode`] — BFS 遍历结果节点，携带深度与调用类型
//!
//! # 使用示例
//! ```ignore
//! use codeconnect_graph::call_graph::CallGraph;
//! use codeconnect_index::sled_store::SledStore;
//!
//! let sled = SledStore::open(path)?;
//! let graph = CallGraph::build_from_sled(&sled)?;
//! let callers = graph.trace_callers("my_symbol", 3);
//! ```

use std::collections::{HashMap, VecDeque};

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::{Bfs, EdgeRef};
use petgraph::Direction;

use codeconnect_core::types::{CallEdge, CallType, Symbol, SymbolKind};
use codeconnect_core::error::CodeConnectError;
use codeconnect_index::sled_store::SledStore;

// ============================================================================
// 图节点
// ============================================================================

/// 调用图中的节点，表示一个符号的精简信息
///
/// 相比完整的 [`Symbol`]，只保留调用图分析所需的关键字段，
/// 以降低内存占用并加速图遍历。
#[derive(Debug, Clone)]
pub struct SymbolNode {
    /// 稳定的全局唯一符号 ID
    pub symbol_id: String,
    /// 符号名称（源代码中的标识符）
    pub name: String,
    /// 符号种类
    pub kind: SymbolKind,
    /// 符号所在的源文件路径
    pub file_path: String,
}

impl SymbolNode {
    /// 从完整的 [`Symbol`] 创建精简节点
    pub fn from_symbol(symbol: &Symbol) -> Self {
        Self {
            symbol_id: symbol.id.clone(),
            name: symbol.name.clone(),
            kind: symbol.kind.clone(),
            file_path: symbol.location.file_path.clone(),
        }
    }
}

// ============================================================================
// BFS 遍历结果
// ============================================================================

/// BFS 调用链中的节点
///
/// 相比 [`SymbolNode`]，额外携带了 BFS 深度和调用类型信息，
/// 便于前端渲染调用链视图。
#[derive(Debug, Clone)]
pub struct CallChainNode {
    /// 符号 ID
    pub symbol_id: String,
    /// 符号名称
    pub name: String,
    /// 距离起始符号的 BFS 深度（起始节点深度为 0，直接调用者为 1，以此类推）
    pub depth: usize,
    /// 调用类型（如 Direct、Virtual、Callback 等）
    pub call_type: String,
    /// 符号所在的源文件路径
    pub file_path: String,
}

// ============================================================================
// 调用图
// ============================================================================

/// 符号间调用关系图
///
/// 内部使用 petgraph 的有向图 [`DiGraph`] 存储节点和边，
/// 同时维护 `id_to_index` 映射以支持按 stable_id 快速查找节点。
///
/// 边的方向：caller → callee（调用者指向被调用者）。
pub struct CallGraph {
    /// 有向图：调用者 → 被调用者
    graph: DiGraph<SymbolNode, CallEdge>,
    /// 符号 ID → NodeIndex 映射，用于 O(1) 按 ID 查找节点
    id_to_index: HashMap<String, NodeIndex>,
}

impl CallGraph {
    /// 创建一个空的调用图
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            id_to_index: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // 节点与边的操作
    // -----------------------------------------------------------------------

    /// 添加一个节点到图中
    ///
    /// 如果符号 ID 已经存在，返回已有的 NodeIndex 而不是重复插入。
    /// 这样可以在构建图时处理重复符号的情况。
    pub fn add_node(&mut self, node: SymbolNode) -> NodeIndex {
        if let Some(&idx) = self.id_to_index.get(&node.symbol_id) {
            return idx;
        }
        let idx = self.graph.add_node(node.clone());
        self.id_to_index.insert(node.symbol_id, idx);
        idx
    }

    /// 添加一条调用边
    ///
    /// 调用者和被调用者节点必须已经存在于图中（通过 `add_node` 添加），
    /// 否则此方法不会有任何效果。
    ///
    /// 边的方向：from caller → to callee。
    pub fn add_edge(&mut self, caller: &str, callee: &str, edge: CallEdge) {
        let from = self.id_to_index.get(caller);
        let to = self.id_to_index.get(callee);
        if let (Some(&from), Some(&to)) = (from, to) {
            self.graph.add_edge(from, to, edge);
        }
    }

    /// 按符号 ID 查找图中的节点引用
    ///
    /// 返回 `None` 如果该符号不在图中。
    pub fn get_node_by_id(&self, symbol_id: &str) -> Option<&SymbolNode> {
        self.id_to_index
            .get(symbol_id)
            .map(|&idx| &self.graph[idx])
    }

    // -----------------------------------------------------------------------
    // BFS 遍历 — 向上查找调用者（谁调用了目标符号）
    // -----------------------------------------------------------------------

    /// 向上查找调用者链条
    ///
    /// 从目标符号出发，沿 incoming edges（谁调用了它）方向进行 BFS，
    /// 返回所有调用者节点，按发现顺序排列。
    ///
    /// # 参数
    /// - `symbol_id` — 目标符号 ID
    /// - `max_depth` — 最大搜索深度（起始节点深度为 0）
    ///
    /// # 返回
    /// 调用链节点列表，不包含起始节点自身。
    pub fn trace_callers(
        &self,
        symbol_id: &str,
        max_depth: usize,
    ) -> Vec<CallChainNode> {
        let start_idx = match self.id_to_index.get(symbol_id) {
            Some(&idx) => idx,
            None => return Vec::new(),
        };

        self.trace_bfs(start_idx, max_depth, Direction::Incoming)
    }

    // -----------------------------------------------------------------------
    // BFS 遍历 — 向下查找被调用者（目标符号调用了谁）
    // -----------------------------------------------------------------------

    /// 向下查找被调用者链条
    ///
    /// 从目标符号出发，沿 outgoing edges（它调用了谁）方向进行 BFS，
    /// 返回所有被调用者节点，按发现顺序排列。
    ///
    /// # 参数
    /// - `symbol_id` — 目标符号 ID
    /// - `max_depth` — 最大搜索深度（起始节点深度为 0）
    ///
    /// # 返回
    /// 调用链节点列表，不包含起始节点自身。
    pub fn trace_callees(
        &self,
        symbol_id: &str,
        max_depth: usize,
    ) -> Vec<CallChainNode> {
        let start_idx = match self.id_to_index.get(symbol_id) {
            Some(&idx) => idx,
            None => return Vec::new(),
        };

        self.trace_bfs(start_idx, max_depth, Direction::Outgoing)
    }

    /// BFS 核心遍历逻辑
    ///
    /// 沿指定方向遍历图，用 visited 集合去重，用 depth_map 追踪深度。
    /// 同时维护 parent_map 记录每个节点是从哪个前驱节点发现的，
    /// 以便获取连接边的调用类型。
    ///
    /// - Direction::Incoming → 查找调用者（谁连向我）
    /// - Direction::Outgoing → 查找被调用者（我连向谁）
    fn trace_bfs(
        &self,
        start: NodeIndex,
        max_depth: usize,
        direction: Direction,
    ) -> Vec<CallChainNode> {
        let mut result: Vec<CallChainNode> = Vec::new();
        let mut visited: HashMap<NodeIndex, bool> = HashMap::new();
        let mut depth_map: HashMap<NodeIndex, usize> = HashMap::new();
        let mut parent_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();

        // 将起始节点的所有邻居加入队列，记录父节点为 start
        let start_depth = 0;
        let mut neighbors: Vec<NodeIndex> = self
            .graph
            .neighbors_directed(start, direction)
            .collect();

        for neighbor in &neighbors {
            depth_map.insert(*neighbor, start_depth + 1);
            parent_map.insert(*neighbor, start);
            visited.insert(*neighbor, true);
            queue.push_back(*neighbor);
        }

        // BFS 主循环
        while let Some(current) = queue.pop_front() {
            let depth = *depth_map.get(&current).unwrap_or(&0);

            // 收集结果
            let node = &self.graph[current];

            // 获取从父节点到当前节点的边的调用类型
            let call_type = self.get_edge_between(parent_map.get(&current).copied(), current);

            result.push(CallChainNode {
                symbol_id: node.symbol_id.clone(),
                name: node.name.clone(),
                depth,
                call_type,
                file_path: node.file_path.clone(),
            });

            // 如果已达到最大深度，不再继续展开
            if depth >= max_depth {
                continue;
            }

            // 探索当前节点的邻居
            neighbors = self
                .graph
                .neighbors_directed(current, direction)
                .collect();

            for neighbor in &neighbors {
                if visited.contains_key(neighbor) {
                    continue;
                }
                visited.insert(*neighbor, true);
                depth_map.insert(*neighbor, depth + 1);
                parent_map.insert(*neighbor, current);
                queue.push_back(*neighbor);
            }
        }

        result
    }

    /// 获取从 from_node 到 to_node 的边的调用类型
    ///
    /// from 是前驱节点（调用者），to 是当前节点（被调用者）。
    fn get_edge_between(
        &self,
        from: Option<NodeIndex>,
        to: NodeIndex,
    ) -> String {
        let from = match from {
            Some(f) => f,
            None => return "Unknown".to_string(),
        };

        // 根据方向确定 caller 和 callee
        // Incoming：from 通过调用边连接到 to，from 是 caller
        // Outgoing：from 通过调用边连接到 to（petgraph neighbors_directed 保证）
        let (caller_idx, callee_idx) = (from, to);

        // 查找 caller → callee 的边
        if let Some(edge) = self.graph.find_edge(caller_idx, callee_idx) {
            call_type_to_str(&self.graph[edge].call_type).to_string()
        } else {
            // 如果精确匹配没找到，尝试反向查找
            // (某些情况下边的方向和遍历方向可能不一致)
            if let Some(edge) = self.graph.find_edge(callee_idx, caller_idx) {
                call_type_to_str(&self.graph[edge].call_type).to_string()
            } else {
                "Unknown".to_string()
            }
        }
    }

    // -----------------------------------------------------------------------
    // 从 sled 构建调用图
    // -----------------------------------------------------------------------

    /// 从 sled 存储中构建完整调用图
    ///
    /// # 构建过程
    /// 1. 扫描 edges: 前缀的所有键，收集所有不重复的符号 ID
    /// 2. 对每个符号 ID，从 sled symbols 命名空间读取 [`Symbol`] 并反序列化
    /// 3. 将所有符号作为节点加入图
    /// 4. 再次遍历所有边，将 [`CallEdge`] 加入图
    ///
    /// # 参数
    /// - `sled` — 已打开的 sled 数据库实例
    ///
    /// # 错误
    /// 在反序列化 Symbol 或 CallEdge 失败时返回错误。
    pub fn build_from_sled(sled: &SledStore) -> Result<Self, CodeConnectError> {
        let mut call_graph = CallGraph::new();

        // ================================================================
        // 第一步：扫描所有调用边，收集节点和边的数据
        // ================================================================
        let edges_prefix = b"edges:";
        let mut edge_records: Vec<(String, String, CallEdge)> = Vec::new();
        let mut seen_symbols: HashMap<String, bool> = HashMap::new();

        for item in sled.scan_prefix(edges_prefix) {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key).to_string();

            // 键格式：edges:{caller_id}::{callee_id}
            // 去掉 "edges:" 前缀后按 "::" 分割
            let body = &key_str["edges:".len()..];
            let parts: Vec<&str> = body.split("::").collect();
            if parts.len() < 2 {
                continue; // 跳过格式不符的键
            }
            let caller_id = parts[0].to_string();
            let callee_id = parts[1..].join("::"); // callee_id 可能包含 ::

            // 标记出现过的符号
            seen_symbols.insert(caller_id.clone(), true);
            seen_symbols.insert(callee_id.clone(), true);

            // 反序列化 CallEdge
            let call_edge: CallEdge = serde_json::from_slice(&value).map_err(|e| {
                CodeConnectError::Serialization(e)
            })?;

            edge_records.push((caller_id, callee_id, call_edge));
        }

        // ================================================================
        // 第二步：为每个出现过的符号创建图节点
        // ================================================================
        for symbol_id in seen_symbols.keys() {
            match sled.get_symbol(symbol_id) {
                Ok(Some(data)) => {
                    let symbol: Symbol = serde_json::from_slice(&data).map_err(|e| {
                        CodeConnectError::Serialization(e)
                    })?;
                    let node = SymbolNode::from_symbol(&symbol);
                    call_graph.add_node(node);
                }
                Ok(None) => {
                    // 符号在 edges 中被引用但不在 symbols 中存储
                    // 创建一个占位节点，避免边无法添加
                    let placeholder = SymbolNode {
                        symbol_id: symbol_id.clone(),
                        name: format!("<未知符号:{}>", symbol_id),
                        kind: SymbolKind::Unknown(format!("未索引")),
                        file_path: String::new(),
                    };
                    call_graph.add_node(placeholder);
                }
                Err(_) => {
                    // 读取失败也创建占位节点
                    let placeholder = SymbolNode {
                        symbol_id: symbol_id.clone(),
                        name: format!("<未知符号:{}>", symbol_id),
                        kind: SymbolKind::Unknown(format!("读取失败")),
                        file_path: String::new(),
                    };
                    call_graph.add_node(placeholder);
                }
            }
        }

        // ================================================================
        // 第三步：添加所有调用边
        // ================================================================
        for (caller_id, callee_id, edge) in edge_records {
            call_graph.add_edge(&caller_id, &callee_id, edge);
        }

        Ok(call_graph)
    }

    // -----------------------------------------------------------------------
    // 出入度统计
    // -----------------------------------------------------------------------

    /// 便捷方法：只用名称添加一条调用边（用于测试和组合构建）
    ///
    /// 与 `add_edge` 不同，此方法使用 `symbol_id` 作为节点的唯一标识，
    /// 并且会自动创建不存在的节点。
    pub fn add_edge_raw(&mut self, caller_id: &str, callee_id: &str) {
        // 确保节点存在
        if !self.id_to_index.contains_key(caller_id) {
            let node = SymbolNode {
                symbol_id: caller_id.to_string(),
                name: caller_id.to_string(),
                kind: SymbolKind::Function,
                file_path: "test.rs".to_string(),
            };
            self.add_node(node);
        }
        if !self.id_to_index.contains_key(callee_id) {
            let node = SymbolNode {
                symbol_id: callee_id.to_string(),
                name: callee_id.to_string(),
                kind: SymbolKind::Function,
                file_path: "test.rs".to_string(),
            };
            self.add_node(node);
        }

        use codeconnect_core::types::SourceLocation;
        let edge = CallEdge {
            caller_id: caller_id.to_string(),
            callee_id: callee_id.to_string(),
            location: SourceLocation {
                file_path: "test.rs".to_string(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            call_type: CallType::Direct,
            confidence: 1.0,
        };
        self.add_edge(caller_id, callee_id, edge);
    }

    /// 获取指定符号的出入度
    ///
    /// 返回 `(fan_in, fan_out)` 元组：
    /// - `fan_in` — 入度（被其他符号调用的次数）
    /// - `fan_out` — 出度（调用其他符号的次数）
    ///
    /// 按 symbol_id 查询（与 `add_node`/`add_edge` 的键一致）。
    /// 如果符号不在图中，返回 `(0, 0)`。
    pub fn degree(&self, symbol_id: &str) -> (u64, u64) {
        let idx = match self.id_to_index.get(symbol_id) {
            Some(&i) => i,
            None => return (0, 0),
        };

        let fan_in = self.graph.edges_directed(idx, Direction::Incoming).count() as u64;
        let fan_out = self.graph.edges_directed(idx, Direction::Outgoing).count() as u64;
        (fan_in, fan_out)
    }

    /// 获取所有调用者（按 symbol_id 查询）
    ///
    /// 返回所有通过入边连接到该符号的符号 ID 列表。
    pub fn get_callers(&self, symbol_id: &str) -> Vec<String> {
        let idx = match self.id_to_index.get(symbol_id) {
            Some(&i) => i,
            None => return Vec::new(),
        };

        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|e| self.graph[e.source()].symbol_id.clone())
            .collect()
    }

    /// 获取所有被调用者（按 symbol_id 查询）
    ///
    /// 返回该符号通过出边调用的所有符号 ID 列表。
    pub fn get_callees(&self, symbol_id: &str) -> Vec<String> {
        let idx = match self.id_to_index.get(symbol_id) {
            Some(&i) => i,
            None => return Vec::new(),
        };

        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|e| self.graph[e.target()].symbol_id.clone())
            .collect()
    }

    /// BFS 遍历从指定符号出发可到达的所有符号
    ///
    /// 使用 petgraph 内置 BFS 算法，沿出边方向遍历。
    pub fn bfs_reachable(&self, symbol_id: &str) -> Vec<String> {
        let start = match self.id_to_index.get(symbol_id) {
            Some(&i) => i,
            None => return Vec::new(),
        };

        let mut bfs = Bfs::new(&self.graph, start);
        let mut result = Vec::new();
        while let Some(node) = bfs.next(&self.graph) {
            result.push(self.graph[node].symbol_id.clone());
        }
        result
    }

    /// 获取图中节点总数
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// 获取图中边总数
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 将 CallType 枚举转换为中文描述字符串
fn call_type_to_str(ct: &CallType) -> &'static str {
    match ct {
        CallType::Direct => "直接调用",
        CallType::Virtual => "虚函数调用",
        CallType::Callback => "回调调用",
        CallType::MacroExpansion => "宏展开调用",
        CallType::Unknown => "未知调用类型",
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助函数：创建测试用的 SymbolNode
    fn make_node(id: &str, name: &str) -> SymbolNode {
        SymbolNode {
            symbol_id: id.to_string(),
            name: name.to_string(),
            kind: SymbolKind::Function,
            file_path: format!("{}.rs", name),
        }
    }

    /// 辅助函数：创建测试用的 CallEdge
    fn make_edge(caller: &str, callee: &str, ct: CallType) -> CallEdge {
        use codeconnect_core::types::SourceLocation;
        CallEdge {
            caller_id: caller.to_string(),
            callee_id: callee.to_string(),
            location: SourceLocation {
                file_path: String::new(),
                line: 0,
                column: 0,
                end_line: 0,
                end_column: 0,
            },
            call_type: ct,
            confidence: 1.0,
        }
    }

    #[test]
    fn test_new_graph_is_empty() {
        let g = CallGraph::new();
        assert!(g.id_to_index.is_empty());
        assert_eq!(g.graph.node_count(), 0);
        assert_eq!(g.graph.edge_count(), 0);
    }

    #[test]
    fn test_add_node_and_find_by_id() {
        let mut g = CallGraph::new();
        let node = make_node("func_a", "func_a");
        g.add_node(node);

        let found = g.get_node_by_id("func_a");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "func_a");
    }

    #[test]
    fn test_add_duplicate_node_returns_same_index() {
        let mut g = CallGraph::new();
        let n1 = make_node("dup", "dup");
        let idx1 = g.add_node(n1);

        let n2 = make_node("dup", "dup_v2");
        let idx2 = g.add_node(n2);

        assert_eq!(idx1, idx2);
        // 节点名称保持第一次插入的值
        let node = g.get_node_by_id("dup").unwrap();
        assert_eq!(node.name, "dup");
    }

    #[test]
    fn test_get_node_by_id_not_found() {
        let g = CallGraph::new();
        assert!(g.get_node_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_trace_callers_simple_chain() {
        let mut g = CallGraph::new();

        // 构建调用链: main → helper → util
        g.add_node(make_node("main", "main"));
        g.add_node(make_node("helper", "helper"));
        g.add_node(make_node("util", "util"));

        g.add_edge("main", "helper", make_edge("main", "helper", CallType::Direct));
        g.add_edge("helper", "util", make_edge("helper", "util", CallType::Direct));

        // 以 util 为目标，向上查找调用者
        let callers = g.trace_callers("util", 10);

        // helper 和 main 都应该被找到
        let names: Vec<String> = callers.iter().map(|c| c.name.clone()).collect();
        assert!(names.contains(&"helper".to_string()));
        assert!(names.contains(&"main".to_string()));

        // helper 应该是深度 1（直接调用者）
        let helper = callers.iter().find(|c| c.name == "helper").unwrap();
        assert_eq!(helper.depth, 1);

        // main 应该是深度 2（间接调用者）
        let main = callers.iter().find(|c| c.name == "main").unwrap();
        assert_eq!(main.depth, 2);
    }

    #[test]
    fn test_trace_callees_simple_chain() {
        let mut g = CallGraph::new();

        // 构建调用链: main → helper → util
        g.add_node(make_node("main", "main"));
        g.add_node(make_node("helper", "helper"));
        g.add_node(make_node("util", "util"));

        g.add_edge("main", "helper", make_edge("main", "helper", CallType::Direct));
        g.add_edge("helper", "util", make_edge("helper", "util", CallType::Direct));

        // 以 main 为目标，向下查找被调用者
        let callees = g.trace_callees("main", 10);

        let names: Vec<String> = callees.iter().map(|c| c.name.clone()).collect();
        assert!(names.contains(&"helper".to_string()));
        assert!(names.contains(&"util".to_string()));

        let helper = callees.iter().find(|c| c.name == "helper").unwrap();
        assert_eq!(helper.depth, 1);

        let util = callees.iter().find(|c| c.name == "util").unwrap();
        assert_eq!(util.depth, 2);
    }

    #[test]
    fn test_max_depth_limit() {
        let mut g = CallGraph::new();

        // 构建: a → b → c → d → e
        let ids = ["a", "b", "c", "d", "e"];
        for id in &ids {
            g.add_node(make_node(id, id));
        }
        for i in 0..ids.len() - 1 {
            g.add_edge(
                ids[i],
                ids[i + 1],
                make_edge(ids[i], ids[i + 1], CallType::Direct),
            );
        }

        // max_depth = 2：只搜索两层
        let callees = g.trace_callees("a", 2);
        let max_depth = callees.iter().map(|c| c.depth).max().unwrap_or(0);
        assert!(max_depth <= 2, "深度不应超过 max_depth");
        // 应包含 b(depth=1) 和 c(depth=2)，但不包含 d(depth=3)
        let names: Vec<String> = callees.iter().map(|c| c.name.clone()).collect();
        assert!(names.contains(&"b".to_string()));
        assert!(names.contains(&"c".to_string()));
        assert!(!names.contains(&"d".to_string()));
    }

    #[test]
    fn test_trace_nonexistent_symbol() {
        let g = CallGraph::new();
        let callers = g.trace_callers("ghost", 10);
        assert!(callers.is_empty());

        let callees = g.trace_callees("ghost", 10);
        assert!(callees.is_empty());
    }

    #[test]
    fn test_call_type_in_result() {
        let mut g = CallGraph::new();
        g.add_node(make_node("a", "a"));
        g.add_node(make_node("b", "b"));

        g.add_edge("a", "b", make_edge("a", "b", CallType::Virtual));

        let callees = g.trace_callees("a", 10);
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].call_type, "虚函数调用");
    }
}
