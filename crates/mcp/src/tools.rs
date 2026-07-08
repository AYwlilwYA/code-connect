//! MCP 工具注册
//!
//! 注册所有 CodeConnect MCP 工具及对应的 handler 函数。
//! 所有工具均返回统一的 [`McpResponse`] 信封。
//!
//! ## 已注册工具列表（16 个）
//!
//! | 工具名称 | 功能 | 参数结构 |
//! |----------|------|----------|
//! | `search_symbol` | 符号搜索 | [`SearchSymbolParams`] |
//! | `get_symbol` | 获取符号详情 | [`GetSymbolParams`] |
//! | `trace_callers` | 追溯调用者（上游） | [`TraceCallersParams`] |
//! | `trace_callees` | 追溯被调用者（下游） | [`TraceCalleesParams`] |
//! | `analyze_impact` | 变更影响分析 | [`AnalyzeImpactParams`] |
//! | `get_call_graph` | 获取调用子图 | [`GetCallGraphParams`] |
//! | `get_metrics` | 代码质量指标 | [`GetMetricsParams`] |
//! | `detect_dead_code` | 死代码检测 | [`DetectDeadCodeParams`] |
//! | `check_arch_rules` | 架构规则验证 | [`CheckArchRulesParams`] |
//! | `semantic_search` | 语义搜索 | [`SemanticSearchParams`] |
//! | `find_references` | 查找引用 | [`FindReferencesParams`] |
//! | `reindex` | 重新索引 | [`ReindexParams`] |
//! | `get_index_status` | 索引状态 | [`GetIndexStatusParams`] |
//! | `list_files` | 列出已索引文件 | [`ListFilesParams`] |
//! | `get_type_hierarchy` | 类型继承链 | [`GetTypeHierarchyParams`] |
//! | `get_file_symbols` | 文件内符号列表 | [`GetFileSymbolsParams`] |
//! | `get_dependency_graph` | 获取依赖图 | [`GetDependencyGraphParams`] |

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use codeconnect_core::config::CodeConnectConfig;
use codeconnect_core::response::McpResponse;
use codeconnect_core::types::Symbol;
use codeconnect_index::full_indexer::{FullIndexer, IndexStats};
use codeconnect_index::query_engine::QueryEngine;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};
use codeconnect_parser::factory::ParserRegistry;

use crate::schemas::*;

// ============================================================================
// 工具注册表 — 共享状态
// ============================================================================

/// MCP 工具注册表
///
/// 持有共享的服务层实例，所有工具 handler 通过此结构
/// 访问索引、调用图等后端数据。
pub struct ToolRegistry {
    /// sled K/V 存储实例
    pub sled: Option<Arc<codeconnect_index::sled_store::SledStore>>,
    /// tantivy 全文搜索索引实例
    pub tantivy: Option<Arc<codeconnect_index::tantivy_index::TantivyIndex>>,
    /// tantivy 调用边索引实例（替代 sled edges 命名空间）
    pub call_edge_index: Option<Arc<codeconnect_index::tantivy_index::CallEdgeIndex>>,
    /// 查询引擎（组合 sled + tantivy）
    pub query_engine: Option<Arc<codeconnect_index::query_engine::QueryEngine>>,
    /// 项目根目录路径（用于重新索引时传递给 CLI）
    pub project_root: Option<PathBuf>,
    /// 索引数据目录路径（用于重新索引时传递给 CLI）
    pub data_dir: Option<PathBuf>,
    /// CodeConnect 配置（用于 reindex 时构建解析器）
    pub config: Option<CodeConnectConfig>,
    /// 解析器注册表（用于 reindex 时进程内构建索引）
    pub parser_registry: Option<Arc<ParserRegistry>>,
}

impl ToolRegistry {
    /// 创建空的工具注册表
    pub fn new() -> Self {
        Self {
            sled: None,
            tantivy: None,
            call_edge_index: None,
            query_engine: None,
            project_root: None,
            data_dir: None,
            config: None,
            parser_registry: None,
        }
    }

    /// 设置 sled 存储实例
    pub fn with_sled(mut self, sled: Arc<codeconnect_index::sled_store::SledStore>) -> Self {
        self.sled = Some(sled);
        self
    }

    /// 设置 tantivy 搜索索引实例
    pub fn with_tantivy(mut self, tantivy: Arc<codeconnect_index::tantivy_index::TantivyIndex>) -> Self {
        self.tantivy = Some(tantivy);
        self
    }

    /// 设置 tantivy 调用边索引实例
    pub fn with_call_edge_index(
        mut self,
        call_edge_index: Arc<codeconnect_index::tantivy_index::CallEdgeIndex>,
    ) -> Self {
        self.call_edge_index = Some(call_edge_index);
        self
    }

    /// 设置查询引擎实例
    pub fn with_query_engine(mut self, qe: Arc<QueryEngine>) -> Self {
        self.query_engine = Some(qe);
        self
    }

    /// 可选地设置查询引擎（索引不存在时跳过）
    ///
    /// 同时接收 tantivy 和 sled 的所有权，如果两者都存在则：
    /// 1. 将它们包装为 `Arc` 并 clone 到 `self.tantivy` / `self.sled`
    /// 2. 用 `from_arc` 创建 `QueryEngine` 设置到 `self.query_engine`
    ///
    /// 这样后续 handler 中 `registry.sled` / `registry.tantivy` 不再为 None。
    pub fn with_query_engine_opt(
        mut self,
        tantivy: Option<TantivyIndex>,
        sled: Option<SledStore>,
    ) -> Self {
        if let (Some(tantivy), Some(sled)) = (tantivy, sled) {
            let tantivy_arc = Arc::new(tantivy);
            let sled_arc = Arc::new(sled);
            self.tantivy = Some(tantivy_arc.clone());
            self.sled = Some(sled_arc.clone());
            self.query_engine = Some(Arc::new(QueryEngine::from_arc(tantivy_arc, sled_arc)));
        }
        self
    }

    /// 可选地设置调用边索引（索引不存在时跳过）
    pub fn with_call_edge_index_opt(mut self, cei: Option<CallEdgeIndex>) -> Self {
        if let Some(cei) = cei {
            self.call_edge_index = Some(Arc::new(cei));
        }
        self
    }

    /// 设置项目根目录路径
    pub fn with_project_root(mut self, path: PathBuf) -> Self {
        self.project_root = Some(path);
        self
    }

    /// 设置索引数据目录路径
    pub fn with_data_dir(mut self, path: PathBuf) -> Self {
        self.data_dir = Some(path);
        self
    }

    /// 设置 CodeConnect 配置（用于 reindex 时获知语言开关）
    pub fn with_config(mut self, config: CodeConnectConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// 设置解析器注册表（用于 reindex 时进程内构建索引）
    pub fn with_parser_registry(mut self, registry: Arc<ParserRegistry>) -> Self {
        self.parser_registry = Some(registry);
        self
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Handler 函数
// ============================================================================

/// 符号搜索 handler
///
/// 通过 tantivy 全文索引按名称搜索符号，支持类型和语言过滤。
pub fn handle_search_symbol(
    registry: &ToolRegistry,
    params: SearchSymbolParams,
) -> McpResponse<Vec<Symbol>> {
    let start = Instant::now();

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    // 默认限制
    let limit = params.limit.min(100);

    let results = match query_engine.search_by_name(&params.query, None, None, limit) {
        Ok(r) => r,
        Err(e) => return McpResponse::error(&format!("搜索失败: {}", e)),
    };

    // 搜索结果已包含完整的符号信息（从 tantivy STORED 字段），直接转换即可
    let mut symbols: Vec<Symbol> = Vec::new();
    for result in &results {
        // 过滤类型
        if let Some(ref kind_filter) = params.kind {
            if result.kind != *kind_filter {
                continue;
            }
        }

        // 语言过滤（从 stable_id 推断，格式: language::path::kind::name::fingerprint）
        if let Some(ref lang_filter) = params.language {
            let lang = result.stable_id.split("::").next().unwrap_or("");
            if lang != lang_filter.as_str() {
                continue;
            }
        }

        let symbol = codeconnect_index::query_engine::symbol_search_result_to_symbol(result);
        symbols.push(symbol);
    }

    let total = symbols.len();
    let elapsed = start.elapsed().as_millis() as u64;

    McpResponse::success(symbols, total, total, elapsed)
}

/// 获取符号详情 handler
pub fn handle_get_symbol(
    registry: &ToolRegistry,
    params: GetSymbolParams,
) -> McpResponse<Symbol> {
    let start = Instant::now();

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    match query_engine.get_symbol_by_id(&params.symbol_id) {
        Ok(Some(symbol)) => {
            let elapsed = start.elapsed().as_millis() as u64;
            McpResponse::success(symbol, 1, 1, elapsed)
        }
        Ok(None) => McpResponse::error(&format!("未找到符号: {}", params.symbol_id)),
        Err(e) => McpResponse::error(&format!("查询失败: {}", e)),
    }
}

/// 追溯调用者 handler
///
/// 反向遍历调用图，找出目标符号的上游调用链。
pub fn handle_trace_callers(
    registry: &ToolRegistry,
    params: TraceCallersParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let call_edge_index = match &registry.call_edge_index {
        Some(e) => e,
        None => return McpResponse::error("调用边索引未初始化"),
    };

    // 从 tantivy 构建调用图（调用边从 tantivy 调用边索引读取）
    let all_ids = match &registry.query_engine {
        Some(q) => match q.scan_all_ids() {
            Ok(ids) => ids,
            Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
        },
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_tantivy_edges(call_edge_index, &all_ids) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    // 从 tantivy 获取符号以获取符号名称
    let symbol_name = match &registry.query_engine {
        Some(q) => match q.get_symbol_by_id(&params.symbol_id) {
            Ok(Some(sym)) => sym.name,
            _ => params.symbol_id.clone(),
        },
        None => params.symbol_id.clone(),
    };

    let callers = call_graph.trace_callers(&symbol_name, params.max_depth);

    // 构建 JSON 响应
    let result = serde_json::json!({
        "target": {
            "symbol_id": params.symbol_id,
            "name": symbol_name,
        },
        "callers": callers.iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "total_callers": callers.len(),
    });

    let total = callers.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

/// 追溯被调用者 handler
///
/// 正向遍历调用图，找出目标符号调用的下游符号。
pub fn handle_trace_callees(
    registry: &ToolRegistry,
    params: TraceCalleesParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let call_edge_index = match &registry.call_edge_index {
        Some(e) => e,
        None => return McpResponse::error("调用边索引未初始化"),
    };

    let all_ids = match &registry.query_engine {
        Some(q) => match q.scan_all_ids() {
            Ok(ids) => ids,
            Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
        },
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_tantivy_edges(call_edge_index, &all_ids) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    let symbol_name = match &registry.query_engine {
        Some(q) => match q.get_symbol_by_id(&params.symbol_id) {
            Ok(Some(sym)) => sym.name,
            _ => params.symbol_id.clone(),
        },
        None => params.symbol_id.clone(),
    };

    let callees = call_graph.trace_callees(&symbol_name, params.max_depth);

    let result = serde_json::json!({
        "source": {
            "symbol_id": params.symbol_id,
            "name": symbol_name,
        },
        "callees": callees.iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "total_callees": callees.len(),
    });

    let total = callees.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

/// 变更影响分析 handler
///
/// 基于 BFS 调用链传播，评估修改指定符号后的影响范围，
/// 输出按严重度分类的影响报告。
pub fn handle_analyze_impact(
    registry: &ToolRegistry,
    params: AnalyzeImpactParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let call_edge_index = match &registry.call_edge_index {
        Some(e) => e,
        None => return McpResponse::error("调用边索引未初始化"),
    };

    let all_ids = match &registry.query_engine {
        Some(q) => match q.scan_all_ids() {
            Ok(ids) => ids,
            Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
        },
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_tantivy_edges(call_edge_index, &all_ids) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    // 解析符号 ID → 名称（从 tantivy 获取）
    let mut symbol_names: Vec<String> = Vec::new();
    for sid in &params.symbol_ids {
        let name = match &registry.query_engine {
            Some(q) => match q.get_symbol_by_id(sid) {
                Ok(Some(sym)) => sym.name,
                _ => sid.clone(),
            },
            None => sid.clone(),
        };
        symbol_names.push(name);
    }

    let analyzer = codeconnect_services::impact_analyzer::ImpactAnalyzer::from_graph(call_graph, params.max_depth);
    let report = analyzer.analyze(&symbol_names);

    // 手动构建 JSON（因为 ImpactReport 未派生 Serialize）
    let direct_impacts: Vec<_> = report.direct_impacts.iter().map(|e| {
        serde_json::json!({
            "symbol_id": e.symbol_id,
            "name": e.name,
            "distance": e.distance,
            "level": "Direct",
            "caused_by": e.caused_by,
        })
    }).collect();

    let transitive_impacts: Vec<_> = report.transitive_impacts.iter().map(|e| {
        serde_json::json!({
            "symbol_id": e.symbol_id,
            "name": e.name,
            "distance": e.distance,
            "level": "Transitive",
            "caused_by": e.caused_by,
        })
    }).collect();

    let result = serde_json::json!({
        "changed_symbols": params.symbol_ids.iter().enumerate().map(|(i, sid)| {
            serde_json::json!({
                "symbol_id": sid,
                "name": symbol_names.get(i).unwrap_or(sid),
            })
        }).collect::<Vec<_>>(),
        "direct_impacts": direct_impacts,
        "transitive_impacts": transitive_impacts,
        "total_affected": report.total_affected(),
        "max_depth": params.max_depth,
    });

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, 1, 1, elapsed)
}

/// 获取调用子图 handler
pub fn handle_get_call_graph(
    registry: &ToolRegistry,
    params: GetCallGraphParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let call_edge_index = match &registry.call_edge_index {
        Some(e) => e,
        None => return McpResponse::error("调用边索引未初始化"),
    };

    let all_ids = match &registry.query_engine {
        Some(q) => match q.scan_all_ids() {
            Ok(ids) => ids,
            Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
        },
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_tantivy_edges(call_edge_index, &all_ids) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    let symbol_name = match &registry.query_engine {
        Some(q) => match q.get_symbol_by_id(&params.symbol_id) {
            Ok(Some(sym)) => sym.name,
            _ => params.symbol_id.clone(),
        },
        None => params.symbol_id.clone(),
    };

    let callers = call_graph.trace_callers(&symbol_name, params.caller_depth);
    let callees = call_graph.trace_callees(&symbol_name, params.callee_depth);

    let result = serde_json::json!({
        "center": {
            "symbol_id": params.symbol_id,
            "name": symbol_name,
        },
        "callers": callers.iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "callees": callees.iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "total_nodes": callers.len() + callees.len() + 1,
    });

    let total = callers.len() + callees.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

/// 获取代码质量指标 handler
pub fn handle_get_metrics(
    registry: &ToolRegistry,
    params: GetMetricsParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let call_edge_index = match &registry.call_edge_index {
        Some(e) => e,
        None => return McpResponse::error("调用边索引未初始化"),
    };

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let all_ids = match query_engine.scan_all_ids() {
        Ok(ids) => ids,
        Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_tantivy_edges(call_edge_index, &all_ids) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    // 如果指定了 file_path，则获取文件内所有符号后再计算指标
    if let Some(ref file_path) = params.file_path {
        let symbols: Vec<Symbol> = match query_engine.get_file_symbols_tantivy(file_path) {
            Ok(syms) => syms,
            Err(e) => return McpResponse::error(&format!("查询文件符号失败: {}", e)),
        };

        if symbols.is_empty() {
            return McpResponse::error(&format!("文件内无符号: {}", file_path));
        }

        let type_hierarchy = codeconnect_graph::type_hierarchy::TypeHierarchy::new();

        let metrics = codeconnect_graph::metrics::MetricCalculator::compute_all(
            &symbols,
            &call_graph,
            &type_hierarchy,
            None,
        );

        let result = serde_json::json!({
            "file_path": file_path,
            "symbol_count": symbols.len(),
            "metrics": metrics.iter().map(|m| {
                serde_json::json!({
                    "symbol_id": m.symbol_id,
                    "name": m.name,
                    "cyclomatic_complexity": m.cyclomatic_complexity,
                    "fan_in": m.fan_in,
                    "fan_out": m.fan_out,
                    "depth_of_inheritance": m.depth_of_inheritance,
                })
            }).collect::<Vec<_>>(),
        });

        let total = metrics.len();
        let elapsed = start.elapsed().as_millis() as u64;
        return McpResponse::success(result, total, total, elapsed);
    }

    // 如果指定了单个符号 ID
    if let Some(ref symbol_id) = params.symbol_id {
        let symbol = match query_engine.get_symbol_by_id(symbol_id) {
            Ok(Some(sym)) => sym,
            Ok(None) => return McpResponse::error(&format!("未找到符号: {}", symbol_id)),
            Err(e) => return McpResponse::error(&format!("查询失败: {}", e)),
        };

        let type_hierarchy = codeconnect_graph::type_hierarchy::TypeHierarchy::new();
        let metrics = codeconnect_graph::metrics::MetricCalculator::compute_all(
            &[symbol],
            &call_graph,
            &type_hierarchy,
            None,
        );

        let m = &metrics[0];
        let result = serde_json::json!({
            "symbol_id": m.symbol_id,
            "name": m.name,
            "cyclomatic_complexity": m.cyclomatic_complexity,
            "fan_in": m.fan_in,
            "fan_out": m.fan_out,
            "depth_of_inheritance": m.depth_of_inheritance,
        });

        let elapsed = start.elapsed().as_millis() as u64;
        return McpResponse::success(result, 1, 1, elapsed);
    }

    // 无参数则返回整体摘要
    let doc_count = match &registry.tantivy {
        Some(t) => t.doc_count().unwrap_or(0),
        None => 0,
    };

    let result = serde_json::json!({
        "total_indexed_symbols": doc_count,
        "hint": "请指定 symbol_id 或 file_path 以获取具体指标",
    });

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, 1, 1, elapsed)
}

/// 死代码检测 handler
pub fn handle_detect_dead_code(
    registry: &ToolRegistry,
    params: DetectDeadCodeParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let call_edge_index = match &registry.call_edge_index {
        Some(e) => e,
        None => return McpResponse::error("调用边索引未初始化"),
    };

    // 收集所有已知的符号 ID 和名称（从 tantivy 获取，不再从 sled 扫描）
    let all_ids = match &registry.query_engine {
        Some(q) => match q.scan_all_ids() {
            Ok(ids) => ids,
            Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
        },
        None => return McpResponse::error("查询引擎未初始化"),
    };

    // 提取所有符号名称供死代码检测使用
    let all_symbols: Vec<String> = all_ids.iter().map(|(_, name)| name.clone()).collect();

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_tantivy_edges(call_edge_index, &all_ids) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    // 确定入口点：优先用参数指定，其次用配置文件
    let entry_points = params.entry_points.unwrap_or_else(|| {
        vec!["main".to_string()]
    });

    let dead_entries = codeconnect_graph::metrics::MetricCalculator::detect_dead_code(
        &all_symbols,
        &call_graph,
        &entry_points,
    );

    let result = serde_json::json!({
        "entry_points": entry_points,
        "total_symbols": all_symbols.len(),
        "dead_code_count": dead_entries.len(),
        "dead_entries": dead_entries.iter().map(|d| {
            serde_json::json!({
                "symbol_id": d.symbol_id,
                "name": d.name,
                "confidence": d.confidence,
                "reason": d.reason,
            })
        }).collect::<Vec<_>>(),
    });

    let total = dead_entries.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

/// 架构规则验证 handler
///
/// 检查依赖图是否违反架构约束规则。
/// 当前 `CheckArchRulesParams` 仅接受 `rule_names`（名称列表），
/// 不包含规则的 source_pattern / target_pattern 等具体定义，
/// 因此无法执行实际规则检查。此功能预留待后续扩展启用了规则定义的 API 后启用。
pub fn handle_check_arch_rules(
    registry: &ToolRegistry,
    params: CheckArchRulesParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    // 构建依赖图以验证底层基础设施可用
    let arch_query = match codeconnect_services::arch_query::ArchQuery::new(sled) {
        Ok(aq) => aq,
        Err(e) => return McpResponse::error(&format!("构建依赖图失败: {}", e)),
    };

    let has_cycle = arch_query.has_cycle();
    let cycles = if has_cycle { arch_query.detect_cycles() } else { Vec::new() };

    let result = serde_json::json!({
        "status": "pending",
        "requested_rules": params.rule_names.unwrap_or_default(),
        "graph_stats": {
            "node_count": arch_query.get_dependency_graph().0.len(),
            "edge_count": arch_query.get_dependency_graph().1.len(),
            "has_cycle": has_cycle,
            "cycle_count": cycles.len(),
        },
        "violations": [],
        "hint": "该功能需要在 MCP 工具参数中提供完整的规则定义（source_pattern、target_pattern、rule_type），当前仅支持依赖图结构查询。请使用 get_dependency_graph 获取依赖关系。",
    });

    let total = 0;
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

/// 语义搜索 handler
pub fn handle_semantic_search(
    registry: &ToolRegistry,
    params: SemanticSearchParams,
) -> McpResponse<Vec<Symbol>> {
    let start = Instant::now();

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let limit = params.limit.min(50);
    let results = match query_engine.search_by_name(&params.description, None, None, limit) {
        Ok(r) => r,
        Err(e) => return McpResponse::error(&format!("语义搜索失败: {}", e)),
    };

    let mut symbols: Vec<Symbol> = Vec::new();
    for result in &results {
        // 语言过滤
        if let Some(ref lang_filter) = params.language {
            let lang = result.stable_id.split("::").next().unwrap_or("");
            if lang != lang_filter.as_str() {
                continue;
            }
        }

        let symbol = codeconnect_index::query_engine::symbol_search_result_to_symbol(result);
        symbols.push(symbol);
    }

    let total = symbols.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(symbols, total, total, elapsed)
}

/// 查找引用 handler
pub fn handle_find_references(
    registry: &ToolRegistry,
    params: FindReferencesParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let call_edge_index = match &registry.call_edge_index {
        Some(e) => e,
        None => return McpResponse::error("调用边索引未初始化"),
    };

    // 获取符号名称（从 tantivy 获取）
    let symbol_name = match &registry.query_engine {
        Some(q) => match q.get_symbol_by_id(&params.symbol_id) {
            Ok(Some(sym)) => sym.name,
            _ => params.symbol_id.clone(),
        },
        None => params.symbol_id.clone(),
    };

    // 从调用图获取所有调用者（从 tantivy 构建）
    let all_ids = match &registry.query_engine {
        Some(q) => match q.scan_all_ids() {
            Ok(ids) => ids,
            Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
        },
        None => return McpResponse::error("查询引擎未初始化"),
    };
    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_tantivy_edges(call_edge_index, &all_ids) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    let callers = call_graph.trace_callers(&symbol_name, 10);

    let references: Vec<serde_json::Value> = callers
        .iter()
        .take(params.limit)
        .map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        })
        .collect();

    let result = serde_json::json!({
        "target": {
            "symbol_id": params.symbol_id,
            "name": symbol_name,
        },
        "references": references,
        "total_references": callers.len(),
    });

    let total = references.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

/// 重新索引 handler
///
/// 在进程内直接调用 [`FullIndexer`] 构建索引，不再 spawn 子进程，
/// 从而避免子进程与父进程的 sled 文件锁冲突。
pub async fn handle_reindex(
    registry: &ToolRegistry,
    params: ReindexParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    // 检查必要的路径参数
    let project_root = match &registry.project_root {
        Some(p) => p.clone(),
        None => return McpResponse::error("项目根目录未配置，无法执行重新索引"),
    };

    // 检查数据目录是否配置（索引存储已由 serve 打开）
    if registry.data_dir.is_none() {
        return McpResponse::error("数据目录未配置，无法执行重新索引");
    }

    // 收集索引存储实例（必须是已打开的共享引用）
    let tantivy = match &registry.tantivy {
        Some(t) => Arc::clone(t),
        None => return McpResponse::error("tantivy 索引未加载，请先启动 MCP 服务器并确保索引已就绪"),
    };
    let sled = match &registry.sled {
        Some(s) => Arc::clone(s),
        None => return McpResponse::error("sled 存储未加载，请先启动 MCP 服务器并确保索引已就绪"),
    };
    let call_edge_index = match &registry.call_edge_index {
        Some(c) => Arc::clone(c),
        None => return McpResponse::error("调用边索引未加载，请先启动 MCP 服务器并确保索引已就绪"),
    };
    let parser_registry = match &registry.parser_registry {
        Some(r) => Arc::clone(r),
        None => return McpResponse::error("解析器注册表未初始化，无法执行重新索引"),
    };

    // 全量索引在 spawn_blocking 中运行以避免阻塞 MCP 事件循环
    let result = tokio::task::spawn_blocking(move || -> Result<IndexStats, String> {
        let indexer = FullIndexer::new(
            &project_root,
            tantivy,
            call_edge_index,
            sled,
            parser_registry,
        );
        indexer.run().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("索引任务异常终止: {}", e));

    match result {
        Ok(Ok(stats)) => {
            let result = serde_json::json!({
                "status": "reindex_complete",
                "mode": if params.full { "full" } else { "incremental" },
                "stats": {
                    "files_scanned": stats.files_scanned,
                    "files_parsed": stats.files_parsed,
                    "symbols_found": stats.symbols_found,
                    "calls_found": stats.calls_found,
                    "imports_found": stats.imports_found,
                    "failed_files": stats.failed_files.len(),
                },
            });
            let elapsed = start.elapsed().as_millis() as u64;
            McpResponse::success(result, 1, 1, elapsed)
        }
        Ok(Err(e)) => {
            McpResponse::error(&format!("索引构建失败: {}", e))
        }
        Err(e) => {
            McpResponse::error(&format!("索引任务执行失败: {}", e))
        }
    }
}

/// 获取索引状态 handler
pub fn handle_get_index_status(
    registry: &ToolRegistry,
    params: GetIndexStatusParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let sled = registry.sled.as_ref();
    let tantivy = registry.tantivy.as_ref();

    let doc_count = match tantivy {
        Some(t) => t.doc_count().unwrap_or(0),
        None => 0,
    };

    let sled_size = sled.map(|s| s.size()).unwrap_or(0);

    let schema_version = sled
        .and_then(|s| s.get_schema_version().ok().flatten())
        .unwrap_or(0);

    let mut result = serde_json::json!({
        "status": if doc_count > 0 { "ready" } else { "empty" },
        "indexed_documents": doc_count,
        "store_entries": sled_size,
        "schema_version": schema_version,
    });

    if params.verbose {
        // 扫描各语言统计
        let mut lang_counts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();

        if let Some(s) = sled {
            let prefix = "meta:";
            for item in s.scan_prefix(prefix.as_bytes()) {
                if let Ok((_key, value)) = item {
                    if let Ok(meta) =
                        serde_json::from_slice::<codeconnect_core::types::FileMeta>(&value)
                    {
                        *lang_counts.entry(meta.language).or_insert(0) += 1;
                    }
                }
            }
        }

        result["language_distribution"] = serde_json::json!(lang_counts);
    }

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, 0, 0, elapsed)
}

/// 列出已索引文件 handler
pub fn handle_list_files(
    registry: &ToolRegistry,
    params: ListFilesParams,
) -> McpResponse<Vec<codeconnect_core::types::FileMeta>> {
    let start = Instant::now();

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let mut files: Vec<codeconnect_core::types::FileMeta> = Vec::new();
    let prefix = "meta:";
    for item in sled.scan_prefix(prefix.as_bytes()) {
        if let Ok((_key, value)) = item {
            if let Ok(meta) =
                serde_json::from_slice::<codeconnect_core::types::FileMeta>(&value)
            {
                // 语言过滤
                if let Some(ref lang) = params.language {
                    if meta.language != *lang {
                        continue;
                    }
                }
                files.push(meta);
            }
        }
    }

    // 按路径排序
    files.sort_by(|a, b| a.file_path.cmp(&b.file_path));

    let total = files.len();
    let returned = files
        .iter()
        .skip(params.offset)
        .take(params.limit)
        .cloned()
        .collect::<Vec<_>>();

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(returned, total, total, elapsed)
}

/// 获取类型继承链 handler
///
/// 从 tantivy 存储中的符号构建类型层次图，然后查询目标符号的祖先/后代。
pub fn handle_get_type_hierarchy(
    registry: &ToolRegistry,
    params: GetTypeHierarchyParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    // 从 tantivy 扫描所有符号的 ID，然后按 ID 逐个获取完整符号信息
    let all_ids = match query_engine.scan_all_ids() {
        Ok(ids) => ids,
        Err(e) => return McpResponse::error(&format!("扫描符号 ID 失败: {}", e)),
    };

    let mut all_symbols: Vec<codeconnect_core::types::Symbol> = Vec::new();
    for (stable_id, _name) in &all_ids {
        match query_engine.get_symbol_by_id(stable_id) {
            Ok(Some(sym)) => all_symbols.push(sym),
            Ok(None) => {} // 符号可能已被删除，跳过
            Err(e) => {
                tracing::warn!("获取符号 {} 失败: {}", stable_id, e);
            }
        }
    }

    // 从 tantivy 符号列表构建完整的类型层次图
    let type_hierarchy = match codeconnect_graph::type_hierarchy::TypeHierarchy::build_from_symbols(&all_symbols) {
        Ok(h) => h,
        Err(e) => return McpResponse::error(&format!("构建类型层次图失败: {}", e)),
    };

    // 从 tantivy 获取符号名称（用于在层次图中查找）
    let symbol_name = match query_engine.get_symbol_by_id(&params.symbol_id) {
        Ok(Some(sym)) => sym.name,
        _ => params.symbol_id.clone(),
    };

    let mut ancestors = Vec::new();
    let mut descendants = Vec::new();

    if params.direction == "ancestors" || params.direction == "both" {
        ancestors = type_hierarchy
            .get_ancestors(&symbol_name)
            .into_iter()
            .map(|n| {
                serde_json::json!({
                    "name": n.name,
                    "symbol_id": n.symbol_id,
                    "kind": n.kind,
                })
            })
            .collect();
    }

    if params.direction == "descendants" || params.direction == "both" {
        descendants = type_hierarchy
            .get_descendants(&symbol_name)
            .into_iter()
            .map(|n| {
                serde_json::json!({
                    "name": n.name,
                    "symbol_id": n.symbol_id,
                    "kind": n.kind,
                })
            })
            .collect();
    }

    let result = serde_json::json!({
        "target": {
            "symbol_id": params.symbol_id,
            "name": symbol_name,
        },
        "ancestors": ancestors,
        "descendants": descendants,
        "graph_stats": {
            "total_types": type_hierarchy.node_count(),
            "total_edges": type_hierarchy.edge_count(),
        },
    });

    let total = ancestors.len() + descendants.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

/// 获取文件内所有符号 handler
///
/// 直接从 tantivy 按 file_path 精确搜索，不再通过 sled 的 file_symbols 映射。
pub fn handle_get_file_symbols(
    registry: &ToolRegistry,
    params: GetFileSymbolsParams,
) -> McpResponse<Vec<Symbol>> {
    let start = Instant::now();

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let symbols = match query_engine.get_file_symbols_tantivy(&params.file_path) {
        Ok(syms) => syms,
        Err(e) => return McpResponse::error(&format!("查询文件符号失败: {}", e)),
    };

    if symbols.is_empty() {
        return McpResponse::error(&format!("文件内无符号: {}", params.file_path));
    }

    let total = symbols.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(symbols, total, total, elapsed)
}

/// 获取依赖图 handler
///
/// 从 sled 的 import 记录构建文件级依赖图并返回。
pub fn handle_get_dependency_graph(
    registry: &ToolRegistry,
    params: GetDependencyGraphParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let arch_query = match codeconnect_services::arch_query::ArchQuery::new(sled) {
        Ok(aq) => aq,
        Err(e) => return McpResponse::error(&format!("构建依赖图失败: {}", e)),
    };

    let (nodes, edges) = arch_query.get_dependency_graph();

    // 如果指定了 file_path，过滤只包含与该文件相关的节点和边
    let (filtered_nodes, filtered_edges) = if let Some(ref file_path) = params.file_path {
        // 包含该文件本身及其直接依赖和被依赖节点
        let mut relevant_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        relevant_ids.insert(file_path.clone());

        // 该文件的直接依赖
        for dep in arch_query.get_dependencies(file_path) {
            relevant_ids.insert(dep.id.clone());
        }
        // 该文件的被依赖节点
        for dep in arch_query.get_dependents(file_path) {
            relevant_ids.insert(dep.id.clone());
        }

        let filtered_nodes: Vec<_> = nodes
            .into_iter()
            .filter(|n| relevant_ids.contains(&n.id))
            .collect();
        let filtered_edges: Vec<_> = edges
            .into_iter()
            .filter(|(src, tgt, _)| relevant_ids.contains(&src.id) && relevant_ids.contains(&tgt.id))
            .collect();
        (filtered_nodes, filtered_edges)
    } else {
        (nodes, edges)
    };

    let result = serde_json::json!({
        "level": params.level,
        "nodes": filtered_nodes.iter().map(|n| {
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "kind": format!("{:?}", n.kind),
            })
        }).collect::<Vec<_>>(),
        "edges": filtered_edges.iter().map(|(src, tgt, edge)| {
            serde_json::json!({
                "source": src.id,
                "target": tgt.id,
                "edge_type": edge.edge_type,
                "count": edge.count,
            })
        }).collect::<Vec<_>>(),
        "total_nodes": filtered_nodes.len(),
        "total_edges": filtered_edges.len(),
    });

    let total = filtered_nodes.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, total, total, elapsed)
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_registry_creation() {
        let registry = ToolRegistry::new();
        assert!(registry.sled.is_none());
        assert!(registry.tantivy.is_none());
        assert!(registry.call_edge_index.is_none());
        assert!(registry.query_engine.is_none());
    }

    #[test]
    fn test_tool_registry_default() {
        let registry = ToolRegistry::default();
        assert!(registry.sled.is_none());
    }

    #[test]
    fn test_handle_search_symbol_no_tantivy() {
        let registry = ToolRegistry::new();
        let params = SearchSymbolParams {
            query: "test".to_string(),
            kind: None,
            language: None,
            limit: 10,
        };
        let response = handle_search_symbol(&registry, params);
        assert_eq!(response.status, codeconnect_core::response::ResponseStatus::Error);
    }

    #[test]
    fn test_handle_get_symbol_no_query_engine() {
        let registry = ToolRegistry::new();
        let params = GetSymbolParams {
            symbol_id: "test_id".to_string(),
        };
        let response = handle_get_symbol(&registry, params);
        assert_eq!(response.status, codeconnect_core::response::ResponseStatus::Error);
    }
}
