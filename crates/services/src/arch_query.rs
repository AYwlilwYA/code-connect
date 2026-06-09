//! 架构查询服务
//!
//! 封装依赖图构建和循环检测，提供架构级的查询能力：
//! - `get_dependency_graph` — 获取文件级依赖图
//! - `detect_cycles` — 检测依赖循环
//! - `check_rules` — 验证自定义架构规则

use codeconnect_core::error::CodeConnectError;
use codeconnect_graph::cycle_detect::CycleDetector;
use codeconnect_graph::dep_graph::{DepNode, DependencyGraph};

/// 架构规则类型
///
/// 定义架构约束的类型。
#[derive(Debug, Clone)]
pub enum RuleType {
    /// 禁止依赖 — source 不能依赖 target
    ForbiddenDependency,
    /// 必须依赖 — source 必须依赖 target
    RequiredDependency,
    /// 层级约束 — 只能从下层依赖上层（如 UI → Service → Data）
    LayerConstraint,
}

/// 架构规则定义
///
/// 描述一条架构约束规则，用于检查代码依赖是否符合架构设计。
#[derive(Debug, Clone)]
pub struct ArchitectureRule {
    /// 规则名称（用于日志和报告）
    pub name: String,
    /// 规则类型
    pub rule_type: RuleType,
    /// 规则描述
    pub description: String,
    /// 源模式（文件/模块的 glob 匹配模式）
    pub source_pattern: String,
    /// 目标模式（文件/模块的 glob 匹配模式）
    pub target_pattern: String,
}

/// 规则违规记录
///
/// 当架构规则被违反时生成的详情。
#[derive(Debug, Clone)]
pub struct RuleViolation {
    /// 被违反的规则名称
    pub rule_name: String,
    /// 违规说明
    pub description: String,
    /// 涉及的源节点
    pub source: String,
    /// 涉及的目标节点
    pub target: String,
}

/// 架构查询结果
///
/// 汇总依赖分析、循环检测和规则检查的结果。
#[derive(Debug, Clone)]
pub struct ArchitectureResult {
    /// 依赖图节点总数
    pub node_count: usize,
    /// 依赖图边总数
    pub edge_count: usize,
    /// 检测到的循环依赖列表
    pub cycles: Vec<Vec<DepNode>>,
    /// 是否存在循环依赖
    pub has_cycle: bool,
    /// 规则检查结果（违规列表）
    pub violations: Vec<RuleViolation>,
}

/// 架构查询服务
///
/// 封装依赖图构建和循环检测，提供依赖图获取、循环检测和规则验证。
pub struct ArchQuery {
    /// 文件级依赖图（从 sled 构建）
    file_graph: DependencyGraph,
}

impl ArchQuery {
    /// 从 sled 存储构建架构查询服务
    ///
    /// 扫描 sled 中的 `imports:` 命名空间，构建文件级依赖图。
    ///
    /// # 参数
    /// - `sled` — 已打开的 sled 数据库实例
    pub fn new(
        sled: &codeconnect_index::sled_store::SledStore,
    ) -> Result<Self, CodeConnectError> {
        let file_graph = DependencyGraph::build_file_graph(sled)?;
        Ok(Self { file_graph })
    }

    /// 从已有的依赖图构建架构查询服务
    ///
    /// # 参数
    /// - `graph` — 已构建好的依赖图（文件级或符号级均可）
    pub fn from_graph(graph: DependencyGraph) -> Self {
        Self { file_graph: graph }
    }

    // =========================================================================
    // 依赖图获取
    // =========================================================================

    /// 获取完整依赖图
    ///
    /// 返回包含所有节点和边的依赖图克隆。
    /// 如果只需要读取部分数据，优先使用 `get_dependencies` 和 `get_dependents`。
    pub fn get_dependency_graph(&self) -> (Vec<DepNode>, Vec<(DepNode, DepNode, DepEdgeInfo)>) {
        let nodes = self.file_graph.nodes();
        let edges: Vec<(DepNode, DepNode, DepEdgeInfo)> = self
            .file_graph
            .edges()
            .into_iter()
            .map(|(src, tgt, edge)| {
                (
                    src,
                    tgt,
                    DepEdgeInfo {
                        edge_type: edge.edge_type,
                        count: edge.count,
                    },
                )
            })
            .collect();
        (nodes, edges)
    }

    /// 获取指定节点的直接依赖
    ///
    /// # 参数
    /// - `node_id` — 节点 ID（文件路径或符号 ID）
    pub fn get_dependencies(&self, node_id: &str) -> Vec<DepNode> {
        self.file_graph.get_dependencies(node_id)
    }

    /// 获取指定节点的被依赖者
    ///
    /// # 参数
    /// - `node_id` — 节点 ID（文件路径或符号 ID）
    pub fn get_dependents(&self, node_id: &str) -> Vec<DepNode> {
        self.file_graph.get_dependents(node_id)
    }

    // =========================================================================
    // 循环检测
    // =========================================================================

    /// 检测依赖图中的所有循环依赖
    ///
    /// 使用 Kosaraju SCC 算法找出所有强连通分量（多节点 SCC），
    /// 每个多节点 SCC 即为一组形成循环依赖的节点。
    pub fn detect_cycles(&self) -> Vec<Vec<DepNode>> {
        CycleDetector::detect_cycles(&self.file_graph)
    }

    /// 检查是否存在循环依赖
    ///
    /// 比 `detect_cycles` 更快，因为找到第一个多节点 SCC 即可返回。
    pub fn has_cycle(&self) -> bool {
        CycleDetector::has_cycle(&self.file_graph)
    }

    // =========================================================================
    // 规则检查
    // =========================================================================

    /// 验证架构规则
    ///
    /// 对当前依赖图执行一组架构规则，返回所有违规项。
    /// 当前支持的规则类型：
    /// - `ForbiddenDependency` — 源不能依赖目标
    ///
    /// # 参数
    /// - `rules` — 架构规则列表
    pub fn check_rules(&self, rules: &[ArchitectureRule]) -> Vec<RuleViolation> {
        let mut violations = Vec::new();

        // 获取依赖图中的所有边用于检查
        let edges = self.file_graph.edges();

        for rule in rules {
            match rule.rule_type {
                RuleType::ForbiddenDependency => {
                    // 检查所有边是否匹配禁止依赖规则
                    for (source, target, _edge) in &edges {
                        if Self::glob_match(&rule.source_pattern, &source.id)
                            && Self::glob_match(&rule.target_pattern, &target.id)
                        {
                            violations.push(RuleViolation {
                                rule_name: rule.name.clone(),
                                description: format!(
                                    "禁止依赖违反: {} 不应依赖 {}",
                                    source.name, target.name
                                ),
                                source: source.id.clone(),
                                target: target.id.clone(),
                            });
                        }
                    }
                }
                RuleType::RequiredDependency => {
                    // 检查源节点是否确实依赖了目标节点
                    let all_nodes = self.file_graph.nodes();
                    let source_nodes: Vec<&DepNode> = all_nodes
                        .iter()
                        .filter(|n| Self::glob_match(&rule.source_pattern, &n.id))
                        .collect();

                    for source in source_nodes {
                        let deps = self.file_graph.get_dependencies(&source.id);
                        let has_required = deps
                            .iter()
                            .any(|d| Self::glob_match(&rule.target_pattern, &d.id));

                        if !has_required {
                            violations.push(RuleViolation {
                                rule_name: rule.name.clone(),
                                description: format!(
                                    "必须依赖缺失: {} 应依赖 {}",
                                    source.name, rule.target_pattern
                                ),
                                source: source.id.clone(),
                                target: rule.target_pattern.clone(),
                            });
                        }
                    }
                }
                RuleType::LayerConstraint => {
                    // 层级约束：检查是否所有依赖都满足方向性
                    for (source, target, _edge) in &edges {
                        // 源和目标如果分属不同层级，检查方向是否合法
                        let source_in_lower = Self::glob_match(&rule.source_pattern, &source.id);
                        let target_in_upper = Self::glob_match(&rule.target_pattern, &target.id);

                        // 如果源在上层、目标在下层，并且层级约束规定下层可以依赖上层
                        // 这里简化为：检查是否存在反向依赖
                        // 如果目标是 source_pattern 匹配的、源是 target_pattern 匹配的，
                        // 那就是反向依赖（违反层级约束）
                        if source_in_lower && target_in_upper {
                            // 源在下层，目标在上层 — 这是允许的（下层依赖上层）
                            // 无需记录
                        } else if Self::glob_match(&rule.target_pattern, &source.id)
                            && Self::glob_match(&rule.source_pattern, &target.id)
                        {
                            // 源在上层，目标在下层 — 这是禁止的（反向依赖）
                            violations.push(RuleViolation {
                                rule_name: rule.name.clone(),
                                description: format!(
                                    "层级约束违反: {}（上层）不应依赖 {}（下层）",
                                    source.name, target.name
                                ),
                                source: source.id.clone(),
                                target: target.id.clone(),
                            });
                        }
                    }
                }
            }
        }

        violations
    }

    /// 运行完整的架构分析（依赖图 + 循环检测 + 规则检查）
    ///
    /// 一次性执行所有架构查询操作，返回汇总结果。
    ///
    /// # 参数
    /// - `rules` — 要验证的架构规则列表（可为空）
    pub fn analyze(&self, rules: &[ArchitectureRule]) -> ArchitectureResult {
        let has_cycle = self.has_cycle();
        let cycles = if has_cycle {
            self.detect_cycles()
        } else {
            Vec::new()
        };

        let violations = self.check_rules(rules);

        ArchitectureResult {
            node_count: self.file_graph.node_count(),
            edge_count: self.file_graph.edge_count(),
            cycles,
            has_cycle,
            violations,
        }
    }

    // =========================================================================
    // 内部辅助
    // =========================================================================

    /// 简单的 glob 风格模式匹配
    ///
    /// 支持通配符 `*`（匹配任意字符序列）。
    /// 如果模式不含通配符，则执行精确字符串比较。
    fn glob_match(pattern: &str, value: &str) -> bool {
        if pattern == "*" {
            return true;
        }
        if !pattern.contains('*') {
            return pattern == value;
        }

        // 按 `*` 分割模式
        let parts: Vec<&str> = pattern.split('*').collect();

        // 如果 pattern 以 `*` 开头，不要求从头匹配
        let anchored_start = !pattern.starts_with('*');
        // 如果 pattern 以 `*` 结尾，不要求匹配到末尾
        let anchored_end = !pattern.ends_with('*');

        let mut pos = 0;

        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }

            // 在 value 中查找 part
            if let Some(found) = value[pos..].find(*part) {
                let abs_pos = pos + found;

                // 第一部分，如果 anchored_start，必须从开头匹配
                if i == 0 && anchored_start && abs_pos != 0 {
                    return false;
                }

                pos = abs_pos + part.len();
            } else {
                return false;
            }
        }

        // 最后一部分，如果 anchored_end，必须匹配到末尾
        if anchored_end && pos != value.len() {
            return false;
        }

        true
    }
}

/// 依赖图边的精简信息（用于服务层输出）
///
/// 避免在服务层暴露 graph crate 内部类型。
#[derive(Debug, Clone)]
pub struct DepEdgeInfo {
    /// 边的类型（import/call/reference）
    pub edge_type: String,
    /// 该类型边的数量
    pub count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeconnect_graph::dep_graph::{DepEdge, DepNodeKind, DependencyGraph};

    /// 构建测试依赖图：A → B → C → D，B → A（形成循环）
    fn build_cyclic_test_graph() -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        graph.add_node(DepNode {
            id: "src/a.rs".into(),
            name: "a.rs".into(),
            kind: DepNodeKind::File,
        });
        graph.add_node(DepNode {
            id: "src/b.rs".into(),
            name: "b.rs".into(),
            kind: DepNodeKind::File,
        });
        graph.add_node(DepNode {
            id: "src/c.rs".into(),
            name: "c.rs".into(),
            kind: DepNodeKind::File,
        });
        graph.add_node(DepNode {
            id: "src/d.rs".into(),
            name: "d.rs".into(),
            kind: DepNodeKind::File,
        });

        let make_edge = || DepEdge {
            edge_type: "import".into(),
            count: 1,
        };

        // A → B, B → C, C → D, B → A（循环：A ↔ B）
        graph.add_edge("src/a.rs", "src/b.rs", make_edge());
        graph.add_edge("src/b.rs", "src/c.rs", make_edge());
        graph.add_edge("src/c.rs", "src/d.rs", make_edge());
        graph.add_edge("src/b.rs", "src/a.rs", make_edge());

        graph
    }

    /// 构建无环测试依赖图
    fn build_acyclic_test_graph() -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        graph.add_node(DepNode {
            id: "src/ui.rs".into(),
            name: "ui.rs".into(),
            kind: DepNodeKind::File,
        });
        graph.add_node(DepNode {
            id: "src/service.rs".into(),
            name: "service.rs".into(),
            kind: DepNodeKind::File,
        });
        graph.add_node(DepNode {
            id: "src/data.rs".into(),
            name: "data.rs".into(),
            kind: DepNodeKind::File,
        });

        let make_edge = || DepEdge {
            edge_type: "import".into(),
            count: 1,
        };

        // ui → service → data
        graph.add_edge("src/ui.rs", "src/service.rs", make_edge());
        graph.add_edge("src/service.rs", "src/data.rs", make_edge());

        graph
    }

    #[test]
    fn test_detect_cycles_cyclic() {
        let graph = build_cyclic_test_graph();
        let arch = ArchQuery::from_graph(graph);

        assert!(arch.has_cycle());
        let cycles = arch.detect_cycles();
        assert!(!cycles.is_empty());
    }

    #[test]
    fn test_detect_cycles_acyclic() {
        let graph = build_acyclic_test_graph();
        let arch = ArchQuery::from_graph(graph);

        assert!(!arch.has_cycle());
        let cycles = arch.detect_cycles();
        assert!(cycles.is_empty());
    }

    #[test]
    fn test_check_forbidden_dependency_rule() {
        let graph = build_acyclic_test_graph();
        let arch = ArchQuery::from_graph(graph);

        let rules = vec![ArchitectureRule {
            name: "禁止 UI 直接依赖 Data".into(),
            rule_type: RuleType::ForbiddenDependency,
            description: "UI 层不应直接引用数据层".into(),
            source_pattern: "src/ui.rs".into(),
            target_pattern: "src/data.rs".into(),
        }];

        let violations = arch.check_rules(&rules);
        // ui → service → data，ui 不直接依赖 data，所以没有违规
        assert!(violations.is_empty());
    }

    #[test]
    fn test_check_forbidden_dependency_violated() {
        let mut graph = DependencyGraph::new();

        graph.add_node(DepNode {
            id: "src/ui.rs".into(),
            name: "ui.rs".into(),
            kind: DepNodeKind::File,
        });
        graph.add_node(DepNode {
            id: "src/data.rs".into(),
            name: "data.rs".into(),
            kind: DepNodeKind::File,
        });

        let make_edge = || DepEdge {
            edge_type: "import".into(),
            count: 1,
        };

        // 直接依赖：ui → data（违反规则）
        graph.add_edge("src/ui.rs", "src/data.rs", make_edge());

        let arch = ArchQuery::from_graph(graph);

        let rules = vec![ArchitectureRule {
            name: "禁止 UI 直接依赖 Data".into(),
            rule_type: RuleType::ForbiddenDependency,
            description: "UI 层不应直接引用数据层".into(),
            source_pattern: "src/ui.rs".into(),
            target_pattern: "src/data.rs".into(),
        }];

        let violations = arch.check_rules(&rules);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule_name, "禁止 UI 直接依赖 Data");
    }

    #[test]
    fn test_glob_match() {
        assert!(ArchQuery::glob_match("src/*.rs", "src/main.rs"));
        assert!(ArchQuery::glob_match("*.rs", "main.rs"));
        assert!(ArchQuery::glob_match("*", "anything"));
        assert!(ArchQuery::glob_match("src/ui.rs", "src/ui.rs"));
        assert!(!ArchQuery::glob_match("src/ui.rs", "src/service.rs"));
        assert!(ArchQuery::glob_match("src/**/*.rs", "src/ui/components/button.rs"));
    }

    #[test]
    fn test_analyze_comprehensive() {
        let graph = build_acyclic_test_graph();
        let arch = ArchQuery::from_graph(graph);

        let rules = vec![ArchitectureRule {
            name: "禁止反向依赖".into(),
            rule_type: RuleType::ForbiddenDependency,
            description: "数据层不应依赖 UI 层".into(),
            source_pattern: "src/data.rs".into(),
            target_pattern: "src/ui.rs".into(),
        }];

        let result = arch.analyze(&rules);

        assert_eq!(result.node_count, 3);
        assert_eq!(result.edge_count, 2);
        assert!(!result.has_cycle);
        assert!(result.cycles.is_empty());
        // data 不依赖 ui，所以无违规
        assert!(result.violations.is_empty());
    }

    #[test]
    fn test_get_dependencies_and_dependents() {
        let graph = build_acyclic_test_graph();
        let arch = ArchQuery::from_graph(graph);

        // ui 依赖 service
        let ui_deps = arch.get_dependencies("src/ui.rs");
        assert_eq!(ui_deps.len(), 1);
        assert_eq!(ui_deps[0].id, "src/service.rs");

        // service 被 ui 依赖
        let service_dependents = arch.get_dependents("src/service.rs");
        assert_eq!(service_dependents.len(), 1);
        assert_eq!(service_dependents[0].id, "src/ui.rs");
    }
}
