//! 代码质量指标
//!
//! 提供单一符号和批量代码质量指标计算：
//! - **圈复杂度** — 基于 AST 分支节点计数的控制流复杂度
//! - **出入度** — 基于调用图的 fan-in / fan-out 统计
//! - **继承深度** — 基于类型层次图的继承链深度
//! - **死代码检测** — 基于调用图可达性分析

use std::collections::{HashMap, HashSet, VecDeque};

use codeconnect_core::types::Symbol;

use crate::call_graph::CallGraph;
use crate::type_hierarchy::TypeHierarchy;

/// 代码质量指标
///
/// 汇总单个符号的复杂度、耦合度、继承深度等指标。
#[derive(Debug, Clone)]
pub struct CodeMetrics {
    /// 符号唯一 ID
    pub symbol_id: String,
    /// 符号名称
    pub name: String,
    /// 圈复杂度（基于分支节点统计）
    pub cyclomatic_complexity: u64,
    /// 入度（被其他符号调用的次数）
    pub fan_in: u64,
    /// 出度（调用其他符号的次数）
    pub fan_out: u64,
    /// 继承深度（从当前类型到继承树根的距离）
    pub depth_of_inheritance: u64,
}

/// 死代码条目
///
/// 表示一个从入口点不可达的、可能为死代码的符号。
#[derive(Debug, Clone)]
pub struct DeadCodeEntry {
    /// 符号唯一 ID
    pub symbol_id: String,
    /// 符号名称
    pub name: String,
    /// 置信度 0.0-1.0，值越低越可能是死代码
    /// 1.0 = 完全不可达（肯定是死代码）
    /// 0.5 = 可能存在间接引用但未检测到
    pub confidence: f64,
    /// 标记为死代码的原因描述
    pub reason: String,
}

/// 指标计算器
///
/// 提供批量计算和死代码检测功能。
/// 自身不维护状态，所有方法均为纯函数。
pub struct MetricCalculator;

impl MetricCalculator {
    // ========================================================================
    // 圈复杂度
    // ========================================================================

    /// 计算单个符号的圈复杂度
    ///
    /// ## 算法
    /// 1. 如果 `symbol.complexity` 已有值，直接返回（来自解析器的预计算结果）
    /// 2. 否则对符号所在函数的源码进行轻量级分析，统计分支关键字：
    ///    - 条件分支: `if`, `else if`
    ///    - 循环: `for`, `while`, `loop`
    ///    - 模式匹配: `match`, `case`, `switch`
    ///    - 异常处理: `catch`, `except`, `rescue`
    ///    - 逻辑运算符（短路求值）: `&&`, `||`
    ///    - 空值合并: `?.`, `??`
    ///    - 三元运算符: `?`
    /// 3. 圈复杂度 = 1（函数入口） + 分支节点总数
    ///
    pub fn compute_complexity(symbol: &Symbol, source: &str) -> u64 {
        // 优先使用解析器预计算的值
        if let Some(complexity) = symbol.complexity {
            return complexity;
        }

        // 对源文本做词法级别的分支关键字计数
        let mut complexity: u64 = 1; // 函数入口基准值

        // 使用简单的空格分隔进行词法分析（非完整解析，但足够准确）
        let tokens: Vec<&str> = source
            .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '{' || c == '}' || c == ';' || c == ',')
            .filter(|s| !s.is_empty())
            .collect();

        for &token in &tokens {
            match token {
                // 条件分支
                "if" | "elif" | "elsif" | "elseif" => complexity += 1,
                // 循环
                "for" | "while" | "loop" => complexity += 1,
                // 模式匹配
                "match" | "case" | "switch" => complexity += 1,
                // 异常处理
                "catch" | "except" | "rescue" | "finally" => complexity += 1,
                // 逻辑运算符（短路求值产生分支）
                "&&" | "||" => complexity += 1,
                _ => {}
            }
        }

        // 统计三元运算符 `?`（在 token 中单独出现）
        complexity += tokens.iter().filter(|&&t| t == "?").count() as u64;

        // 统计空值合并运算符 `??` 和可选链 `?.`
        // 这需要在原始源文本上统计
        complexity += source.matches("??").count() as u64;
        complexity += source.matches("?.").count() as u64;

        complexity
    }

    /// 从文件源码中提取符号对应行范围的文本
    ///
    /// 根据 `symbol.location` 的行范围（line 到 end_line）截取文本。
    /// 如果源文本行数不大（不超过 end_line 较多），说明源文本可能就是函数体而非整个文件，
    /// 此时直接返回完整源码。
    fn extract_symbol_source(symbol: &Symbol, file_source: &str) -> Option<String> {
        let loc = &symbol.location;
        // 行范围无效时不截取
        if loc.line == 0 || loc.end_line == 0 || loc.line > loc.end_line {
            return None;
        }

        let lines: Vec<&str> = file_source.lines().collect();
        let total_lines = lines.len() as u64;

        // 如果源文本总行数不大于符号结束行号的 1.5 倍，
        // 说明传入的可能就是函数体而非整个文件，不截取
        // 使用比例检测而非固定偏移量，避免对大文件也做无意义截取
        if total_lines <= loc.end_line || total_lines <= 20 {
            return None;
        }
        // 如果符号范围覆盖了整个文件的大部分（>80%），不做截取
        let span_lines = loc.end_line - loc.line + 1;
        if total_lines > 0 && span_lines as f64 / total_lines as f64 > 0.8 {
            return None;
        }

        // 行号是 1-based，转换为 0-based 索引
        let start_idx = (loc.line as usize).saturating_sub(1).min(lines.len());
        let end_idx = (loc.end_line as usize).min(lines.len());

        if start_idx >= end_idx {
            return None;
        }

        Some(lines[start_idx..end_idx].join("\n"))
    }

    // ========================================================================
    // 批量指标计算
    // ========================================================================

    /// 批量计算所有符号的代码质量指标
    ///
    /// ## 计算内容
    /// - **圈复杂度** — 优先使用 `symbol.complexity` 预计算值；
    ///   如果为空且提供了 `source_cache`，则通过文本扫描回退计算；
    ///   回退计算时会根据 `symbol.location` 的行范围从文件源码中截取函数体，
    ///   确保每个符号的复杂度基于其自身的代码，而非整个文件；
    ///   否则默认为 1
    /// - **fan_in** — 调用图中指向当前符号的入边数量
    /// - **fan_out** — 调用图中从当前符号出发的出边数量
    /// - **继承深度** — 类型层次图中当前符号到根的距离
    ///
    /// ## 参数
    /// - `symbols` — 符号列表
    /// - `call_graph` — 调用图
    /// - `type_hierarchy` — 类型层次图
    /// - `source_cache` — 可选的文件路径→整个文件源码内容映射，
    ///   `compute_complexity` 内部会根据符号行范围自动截取函数体
    pub fn compute_all(
        symbols: &[Symbol],
        call_graph: &CallGraph,
        type_hierarchy: &TypeHierarchy,
        source_cache: Option<&HashMap<String, String>>,
    ) -> Vec<CodeMetrics> {
        symbols
            .iter()
            .map(|symbol| {
                // 圈复杂度：优先用预计算值 → 回退文本扫描 → 默认 1
                let cyclomatic_complexity = match symbol.complexity {
                    Some(c) => c,
                    None => {
                        // 回退计算：从源码缓存中查找对应文件的源码做文本扫描
                        if let Some(cache) = source_cache {
                            let file_path = &symbol.location.file_path;
                            if let Some(file_source) = cache.get(file_path) {
                                // 尝试从文件源码中截取符号对应的行范围，
                                // 以获取该符号自身的源码（而非整个文件）
                                let scoped_source = Self::extract_symbol_source(symbol, file_source);
                                let source = scoped_source.as_deref().unwrap_or(file_source);
                                Self::compute_complexity(symbol, source)
                            } else {
                                1 // 找不到源码，默认值
                            }
                        } else {
                            1 // 没有源码缓存，默认值
                        }
                    }
                };

                // 出入度：从调用图获取
                // 注意：按符号的稳定 ID 查询（id_to_index 以 stable_id 为键），
                // 回退按名称查询（name_to_index 以符号名称为键）
                let (fan_in, fan_out) = call_graph.degree(&symbol.id);

                // 继承深度：从类型层次图计算祖先数量
                let ancestors = type_hierarchy.get_ancestors(&symbol.name);
                let depth_of_inheritance = ancestors.len() as u64;

                CodeMetrics {
                    symbol_id: symbol.id.clone(),
                    name: symbol.name.clone(),
                    cyclomatic_complexity,
                    fan_in,
                    fan_out,
                    depth_of_inheritance,
                }
            })
            .collect()
    }

    // ========================================================================
    // 死代码检测
    // ========================================================================

    /// 检测死代码
    ///
    /// ## 算法
    /// 从入口点（main 函数、导出的 pub 函数等）出发，在调用图上做正向 BFS
    /// 可达性分析。即从每个入口点开始，沿"调用者→被调用者"方向（出边）遍历，
    /// 将能到达的节点标记为"可达"。
    ///
    /// 最后，所有未被标记为可达的符号即为**可能死代码**。
    ///
    /// ## 置信度计算
    /// - 1.0 — 完全不可达（没有任何入边）
    /// - 0.8 — 不可达但至少有一条入边（可能是反射/回调等间接调用）
    /// - 0.5 — 不可达但名称匹配常见模式（如 `test_*`）
    pub fn detect_dead_code(
        all_symbols: &[String],
        call_graph: &CallGraph,
        entry_points: &[String],
    ) -> Vec<DeadCodeEntry> {
        // 第1步: 构建可达性集合
        let mut reachable = HashSet::new();
        let mut queue = VecDeque::new();

        // 从入口点开始
        for entry in entry_points {
            reachable.insert(entry.clone());
            queue.push_back(entry.clone());
        }

        // BFS 正向遍历：从入口点沿出边找到被调用的符号
        while let Some(current) = queue.pop_front() {
            // 获取 current 调用的所有符号（被调用者）
            let callees = call_graph.get_callees(&current);
            for callee in callees {
                if reachable.insert(callee.clone()) {
                    queue.push_back(callee);
                }
            }
        }

        // 第2步: 找出不可达符号并计算置信度
        let mut dead_entries = Vec::new();
        for symbol_name in all_symbols {
            if reachable.contains(symbol_name.as_str()) {
                continue;
            }

            let (fan_in, _fan_out) = call_graph.degree(symbol_name);

            // 确定置信度和原因
            // 注意：名称模式匹配优先于 fan_in 检查，
            // 这样 test_* 和 _ 前缀符号即使 fan_in=0 也能获得正确的置信度
            let (confidence, reason) = if symbol_name.starts_with("test_") || symbol_name.contains("_test") {
                // 可能是测试代码
                (0.5, "未被入口点可达，但名称匹配测试模式".to_string())
            } else if symbol_name.starts_with('_') {
                // 可能是内部/私有函数
                (0.7, "未被入口点可达，名称前缀暗示内部函数".to_string())
            } else if fan_in == 0 {
                // 完全孤立
                (1.0, "从任何入口点都不可达，且未被任何符号调用".to_string())
            } else {
                // 有入边但不可达（可能是反射、动态调用等）
                (0.8, format!(
                    "未被入口点可达，但被 {} 个符号调用（可能是间接/动态调用）",
                    fan_in
                ))
            };

            dead_entries.push(DeadCodeEntry {
                symbol_id: format!("dead_{}", symbol_name),
                name: symbol_name.clone(),
                confidence,
                reason,
            });
        }

        dead_entries
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_graph::CallGraph;
    use crate::type_hierarchy::{InheritEdge, TypeHierarchy, TypeNode};
    use codeconnect_core::types::{SourceLocation, Symbol, SymbolKind};

    /// 创建测试用 Symbol
    fn make_symbol(id: &str, name: &str, complexity: Option<u64>) -> Symbol {
        Symbol {
            id: id.to_string(),
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

    /// 创建测试用 TypeNode
    fn make_type_node(name: &str, kind: &str) -> TypeNode {
        TypeNode {
            symbol_id: format!("id_{}", name),
            name: name.to_string(),
            kind: kind.to_string(),
            file_path: "test.rs".to_string(),
        }
    }

    /// 构建一个简单的类型层次：Dog → Animal → Object
    fn build_sample_hierarchy() -> TypeHierarchy {
        let mut h = TypeHierarchy::new();
        h.add_node(make_type_node("Object", "class"));
        h.add_node(make_type_node("Animal", "class"));
        h.add_node(make_type_node("Dog", "class"));

        h.add_edge(
            "Dog",
            "Animal",
            InheritEdge {
                relation: "extends".to_string(),
                confidence: 0.9,
            },
        );
        h.add_edge(
            "Animal",
            "Object",
            InheritEdge {
                relation: "extends".to_string(),
                confidence: 0.9,
            },
        );

        h
    }

    // ========================================================================
    // 圈复杂度测试
    // ========================================================================

    #[test]
    fn test_complexity_uses_precomputed_value() {
        let symbol = make_symbol("f1", "my_func", Some(42));
        // 即使 source 很简单，也使用预计算值
        let complexity = MetricCalculator::compute_complexity(&symbol, "hello world");
        assert_eq!(complexity, 42);
    }

    #[test]
    fn test_complexity_minimal_function() {
        let symbol = make_symbol("f2", "simple", None);
        let source = "fn simple() { return 1; }";
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 基础值为 1（函数入口），无分支
        assert_eq!(complexity, 1);
    }

    #[test]
    fn test_complexity_single_if() {
        let symbol = make_symbol("f3", "single_if", None);
        let source = r#"
            fn single_if(x: i32) {
                if x > 0 {
                    println!("positive");
                }
            }
        "#;
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 1（入口） + 1（if） = 2
        assert_eq!(complexity, 2);
    }

    #[test]
    fn test_complexity_multiple_branches() {
        let symbol = make_symbol("f4", "multi_branch", None);
        let source = r#"
            fn multi_branch(x: i32) {
                if x > 0 {
                    return 1;
                } else if x < 0 {
                    return -1;
                }
                for i in 0..10 {
                    while true {
                        break;
                    }
                }
                match x {
                    1 => {},
                    _ => {},
                }
            }
        "#;
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 1（入口） + 2（if + else if） + 1（for） + 1（while） + 1（match） = 6
        assert_eq!(complexity, 6);
    }

    #[test]
    fn test_complexity_with_logical_operators() {
        let symbol = make_symbol("f5", "logical_ops", None);
        let source = "fn check(x: bool, y: bool) { return x && y || !x; }";
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 1（入口） + 1（&&） + 1（||） = 3
        assert_eq!(complexity, 3);
    }

    #[test]
    fn test_complexity_loop_keyword() {
        let symbol = make_symbol("f6", "loop_fn", None);
        let source = "fn forever() { loop { break; } }";
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 1（入口） + 1（loop） = 2
        assert_eq!(complexity, 2);
    }

    #[test]
    fn test_complexity_catch_try() {
        let symbol = make_symbol("f7", "try_catch", None);
        let source = r#"
            function try_catch() {
                try {
                    risky();
                } catch (e) {
                    handle(e);
                } finally {
                    cleanup();
                }
            }
        "#;
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 1（入口） + 1（catch） + 1（finally） = 3
        assert_eq!(complexity, 3);
    }

    #[test]
    fn test_complexity_switch_statement() {
        let symbol = make_symbol("f8", "switch_fn", None);
        let source = r#"
            void switch_fn(int x) {
                switch (x) {
                    case 1: break;
                    case 2: break;
                    default: break;
                }
            }
        "#;
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 1（入口） + 1（switch） + 2（case） = 4
        // 注意: 源代码中有 2 个 case 语句 + 1 个 default（default 不增加复杂度）
        assert_eq!(complexity, 4);
    }

    #[test]
    fn test_complexity_ternary_operator() {
        let symbol = make_symbol("f9", "ternary", None);
        let source = "let x = a ? b : c;";
        let complexity = MetricCalculator::compute_complexity(&symbol, source);
        // 1（入口） + 1（?）= 2
        assert_eq!(complexity, 2);
    }

    // ========================================================================
    // 批量计算测试
    // ========================================================================

    #[test]
    fn test_compute_all_integration() {
        // 构建调用图
        let mut cg = CallGraph::new();
        cg.add_edge_raw("main", "helper");
        cg.add_edge_raw("main", "logger");
        cg.add_edge_raw("helper", "logger");
        cg.add_edge_raw("orphan", "logger");

        // 构建类型层次
        let type_hierarchy = build_sample_hierarchy();

        // 符号列表
        // 注意：Symbol.id 需要与图中 add_edge_raw 使用的节点 ID 一致，
        // 这样 compute_all 中 call_graph.degree(&symbol.id) 才能正确匹配。
        // add_edge_raw 内部会将 symbol_id 和 name 都设为同一个值。
        let symbols = vec![
            make_symbol("main", "main", Some(3)),
            make_symbol("helper", "helper", Some(2)),
            make_symbol("logger", "logger", Some(1)),
            make_symbol("orphan", "orphan", None), // 无预计算复杂度
            make_symbol("Dog", "Dog", Some(5)),
        ];

        let metrics = MetricCalculator::compute_all(&symbols, &cg, &type_hierarchy, None);

        // 验证 main
        let main = metrics.iter().find(|m| m.name == "main").unwrap();
        assert_eq!(main.cyclomatic_complexity, 3);
        assert_eq!(main.fan_in, 0); // 没有被调用
        assert_eq!(main.fan_out, 2); // 调用了 helper 和 logger
        assert_eq!(main.depth_of_inheritance, 0); // 不在类型层次中

        // 验证 helper
        let helper = metrics.iter().find(|m| m.name == "helper").unwrap();
        assert_eq!(helper.cyclomatic_complexity, 2);
        assert_eq!(helper.fan_in, 1); // 被 main 调用
        assert_eq!(helper.fan_out, 1); // 调用 logger
        assert_eq!(helper.depth_of_inheritance, 0);

        // 验证 logger
        let logger = metrics.iter().find(|m| m.name == "logger").unwrap();
        assert_eq!(logger.cyclomatic_complexity, 1);
        assert_eq!(logger.fan_in, 3); // 被 main, helper, orphan 调用
        assert_eq!(logger.fan_out, 0);
        assert_eq!(logger.depth_of_inheritance, 0);

        // 验证 orphan
        let orphan = metrics.iter().find(|m| m.name == "orphan").unwrap();
        assert_eq!(orphan.cyclomatic_complexity, 1); // 回退默认值
        assert_eq!(orphan.fan_in, 0);
        assert_eq!(orphan.fan_out, 1); // 调用 logger

        // 验证 Dog（在类型层次中）
        let dog = metrics.iter().find(|m| m.name == "Dog").unwrap();
        assert_eq!(dog.cyclomatic_complexity, 5);
        assert_eq!(dog.fan_in, 0);
        assert_eq!(dog.fan_out, 0);
        assert_eq!(dog.depth_of_inheritance, 2); // Dog → Animal → Object
    }

    // ========================================================================
    // 死代码检测测试
    // ========================================================================

    #[test]
    fn test_detect_dead_code_no_entry_points() {
        let mut cg = CallGraph::new();
        cg.add_edge_raw("main", "helper");

        let all_symbols: Vec<String> = vec!["main".into(), "helper".into()];
        let entry_points: Vec<String> = vec![];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        // 没有入口点 → 所有符号都不可达
        assert_eq!(dead.len(), 2);
    }

    #[test]
    fn test_detect_dead_code_all_reachable() {
        let mut cg = CallGraph::new();
        cg.add_edge_raw("main", "helper");
        cg.add_edge_raw("helper", "util");
        cg.add_edge_raw("util", "logger");

        let all_symbols: Vec<String> = vec![
            "main".into(),
            "helper".into(),
            "util".into(),
            "logger".into(),
        ];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        // main → helper → util → logger 都在同一条链上，全部可达
        assert!(dead.is_empty());
    }

    #[test]
    fn test_detect_dead_code_partially_reachable() {
        let mut cg = CallGraph::new();
        cg.add_edge_raw("main", "helper");
        cg.add_edge_raw("main", "util");
        cg.add_edge_raw("orphan", "dead_func");

        let all_symbols: Vec<String> = vec![
            "main".into(),
            "helper".into(),
            "util".into(),
            "orphan".into(),
            "dead_func".into(),
        ];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        // main, helper, util 可达
        // orphan 和 dead_func 不可达
        assert_eq!(dead.len(), 2);

        let dead_names: Vec<_> = dead.iter().map(|d| d.name.as_str()).collect();
        assert!(dead_names.contains(&"orphan"));
        assert!(dead_names.contains(&"dead_func"));
    }

    #[test]
    fn test_detect_dead_code_confidence_isolated() {
        let cg = CallGraph::new();
        // orphan 完全孤立 — 没有调用任何人，也没人调用它

        let all_symbols: Vec<String> = vec!["main".into(), "orphan".into()];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].name, "orphan");
        assert_eq!(dead[0].confidence, 1.0); // 完全孤立
    }

    #[test]
    fn test_detect_dead_code_confidence_has_callers() {
        let mut cg = CallGraph::new();
        cg.add_edge_raw("orphan", "dead_func");

        let all_symbols: Vec<String> = vec!["main".into(), "orphan".into(), "dead_func".into()];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        // orphan 不可达但调用了 dead_func
        assert_eq!(dead.len(), 2);

        let orphan_dead = dead.iter().find(|d| d.name == "orphan").unwrap();
        // orphan 有入度为 0 但从入口不可达 → 置信度为 1.0（fan_in=0）
        assert_eq!(orphan_dead.confidence, 1.0);

        let dead_func_dead = dead.iter().find(|d| d.name == "dead_func").unwrap();
        // dead_func 有 1 个入边（orphan 调用），但不可达
        assert_eq!(dead_func_dead.confidence, 0.8);
    }

    #[test]
    fn test_detect_dead_code_test_pattern() {
        let cg = CallGraph::new();

        let all_symbols: Vec<String> = vec!["main".into(), "test_my_func".into()];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        let test_dead = dead.iter().find(|d| d.name == "test_my_func").unwrap();
        assert_eq!(test_dead.confidence, 0.5); // 测试模式降低置信度
    }

    #[test]
    fn test_detect_dead_code_underscore_prefix() {
        let cg = CallGraph::new();

        let all_symbols: Vec<String> = vec!["main".into(), "_internal_fn".into()];
        let entry_points: Vec<String> = vec!["main".into()];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        let internal = dead.iter().find(|d| d.name == "_internal_fn").unwrap();
        assert_eq!(internal.confidence, 0.7);
    }

    #[test]
    fn test_detect_dead_code_multiple_entry_points() {
        let mut cg = CallGraph::new();
        cg.add_edge_raw("pub_fn", "helper");
        cg.add_edge_raw("orphan", "unused");

        let all_symbols: Vec<String> = vec![
            "pub_fn".into(),
            "helper".into(),
            "main".into(),
            "orphan".into(),
            "unused".into(),
        ];
        let entry_points: Vec<String> = vec!["main".into(), "pub_fn".into()];

        let dead = MetricCalculator::detect_dead_code(&all_symbols, &cg, &entry_points);
        assert_eq!(dead.len(), 2);
        let dead_names: Vec<_> = dead.iter().map(|d| d.name.as_str()).collect();
        assert!(dead_names.contains(&"orphan"));
        assert!(dead_names.contains(&"unused"));
    }

    #[test]
    fn test_empty_symbols_list() {
        let cg = CallGraph::new();
        let dead = MetricCalculator::detect_dead_code(&[], &cg, &[]);
        assert!(dead.is_empty());
    }
}
