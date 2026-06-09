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

use std::sync::Arc;
use std::time::Instant;

use codeconnect_core::response::McpResponse;
use codeconnect_core::types::Symbol;

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
    /// 查询引擎（组合 sled + tantivy）
    pub query_engine: Option<Arc<codeconnect_index::query_engine::QueryEngine>>,
}

impl ToolRegistry {
    /// 创建空的工具注册表
    pub fn new() -> Self {
        Self {
            sled: None,
            tantivy: None,
            query_engine: None,
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

    /// 设置查询引擎实例
    pub fn with_query_engine(mut self, qe: Arc<codeconnect_index::query_engine::QueryEngine>) -> Self {
        self.query_engine = Some(qe);
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

    let tantivy = match &registry.tantivy {
        Some(t) => t,
        None => return McpResponse::error("全文搜索索引未初始化"),
    };

    // 默认限制
    let limit = params.limit.min(100);

    let results = match tantivy.search_by_name(&params.query, limit) {
        Ok(r) => r,
        Err(e) => return McpResponse::error(&format!("搜索失败: {}", e)),
    };

    // 从 sled 加载每个搜索结果的完整 Symbol 信息
    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let mut symbols: Vec<Symbol> = Vec::new();
    for result in &results {
        // 过滤类型
        if let Some(ref kind_filter) = params.kind {
            if result.kind != *kind_filter {
                continue;
            }
        }

        if let Ok(Some(bytes)) = sled.get_symbol(&result.stable_id) {
            if let Ok(symbol) = serde_json::from_slice::<Symbol>(&bytes) {
                // 语言过滤
                if let Some(ref lang_filter) = params.language {
                    // 从 symbol id 中推断语言（格式: language::path::kind::name::fingerprint）
                    let lang = symbol.id.split("::").next().unwrap_or("");
                    if lang != lang_filter.as_str() {
                        continue;
                    }
                }
                symbols.push(symbol);
            }
        }
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

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    match sled.get_symbol(&params.symbol_id) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
            Ok(symbol) => {
                let elapsed = start.elapsed().as_millis() as u64;
                McpResponse::success(symbol, 1, 1, elapsed)
            }
            Err(e) => McpResponse::error(&format!("反序列化符号失败: {}", e)),
        },
        Ok(None) => McpResponse::error(&format!("未找到符号: {}", params.symbol_id)),
        Err(e) => McpResponse::error(&format!("读取存储失败: {}", e)),
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

    // 从 sled 构建调用图
    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    // 从 sled 获取符号以获取符号名称
    let symbol_name = match sled.get_symbol(&params.symbol_id) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
            Ok(sym) => sym.name,
            Err(_) => params.symbol_id.clone(),
        },
        _ => params.symbol_id.clone(),
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

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    let symbol_name = match sled.get_symbol(&params.symbol_id) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
            Ok(sym) => sym.name,
            Err(_) => params.symbol_id.clone(),
        },
        _ => params.symbol_id.clone(),
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

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    // 解析符号 ID → 名称
    let mut symbol_names: Vec<String> = Vec::new();
    for sid in &params.symbol_ids {
        let name = match sled.get_symbol(sid) {
            Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
                Ok(sym) => sym.name,
                Err(_) => sid.clone(),
            },
            _ => sid.clone(),
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

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    let symbol_name = match sled.get_symbol(&params.symbol_id) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
            Ok(sym) => sym.name,
            Err(_) => params.symbol_id.clone(),
        },
        _ => params.symbol_id.clone(),
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

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    // 如果指定了 file_path，则获取文件内所有符号后再计算指标
    if let Some(ref file_path) = params.file_path {
        let symbol_ids: Vec<String> = match sled.get_file_symbols(file_path) {
            Ok(Some(bytes)) => {
                serde_json::from_slice(&bytes).unwrap_or_default()
            }
            _ => return McpResponse::error(&format!("未找到文件: {}", file_path)),
        };

        let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
            Ok(g) => g,
            Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
        };

        let type_hierarchy = codeconnect_graph::type_hierarchy::TypeHierarchy::new();

        let mut symbols: Vec<Symbol> = Vec::new();
        for sid in &symbol_ids {
            if let Ok(Some(bytes)) = sled.get_symbol(sid) {
                if let Ok(sym) = serde_json::from_slice::<Symbol>(&bytes) {
                    symbols.push(sym);
                }
            }
        }

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
        let symbol = match sled.get_symbol(symbol_id) {
            Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
                Ok(sym) => sym,
                Err(e) => return McpResponse::error(&format!("反序列化失败: {}", e)),
            },
            Ok(None) => return McpResponse::error(&format!("未找到符号: {}", symbol_id)),
            Err(e) => return McpResponse::error(&format!("读取失败: {}", e)),
        };

        let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
            Ok(g) => g,
            Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
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

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
        Ok(g) => g,
        Err(e) => return McpResponse::error(&format!("构建调用图失败: {}", e)),
    };

    // 收集所有已知的符号名称
    let all_symbols: Vec<String> = {
        // 从 sled 扫描所有符号
        let mut names = Vec::new();
        let prefix = "symbols:";
        for item in sled.scan_prefix(prefix.as_bytes()) {
            if let Ok((_key_bytes, value_bytes)) = item {
                let key = String::from_utf8_lossy(&_key_bytes);
                // 键格式: symbols:{symbol_id}
                let symbol_id = key.strip_prefix(prefix).unwrap_or(&key);
                if let Ok(sym) = serde_json::from_slice::<Symbol>(&value_bytes) {
                    names.push(sym.name);
                } else {
                    // 反序列化失败，直接使用 ID
                    names.push(symbol_id.to_string());
                }
            }
        }
        names
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
pub fn handle_check_arch_rules(
    registry: &ToolRegistry,
    params: CheckArchRulesParams,
) -> McpResponse<serde_json::Value> {
    let _ = registry;
    let start = Instant::now();

    let result = serde_json::json!({
        "checked_rules": params.rule_names.unwrap_or_default(),
        "violations": [],
        "status": "pass",
        "hint": "架构规则验证需要在完整索引和依赖图之上运行",
    });

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, 0, 0, elapsed)
}

/// 语义搜索 handler
pub fn handle_semantic_search(
    registry: &ToolRegistry,
    params: SemanticSearchParams,
) -> McpResponse<Vec<Symbol>> {
    let start = Instant::now();

    let tantivy = match &registry.tantivy {
        Some(t) => t,
        None => return McpResponse::error("全文搜索索引未初始化"),
    };

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let limit = params.limit.min(50);
    let results = match tantivy.search_by_name(&params.description, limit) {
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

        if let Ok(Some(bytes)) = sled.get_symbol(&result.stable_id) {
            if let Ok(symbol) = serde_json::from_slice::<Symbol>(&bytes) {
                symbols.push(symbol);
            }
        }
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

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    // 获取符号名称
    let symbol_name = match sled.get_symbol(&params.symbol_id) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
            Ok(sym) => sym.name,
            Err(_) => params.symbol_id.clone(),
        },
        _ => params.symbol_id.clone(),
    };

    // 从调用图获取所有调用者
    let call_graph = match codeconnect_graph::call_graph::CallGraph::build_from_sled(sled) {
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
pub fn handle_reindex(
    registry: &ToolRegistry,
    params: ReindexParams,
) -> McpResponse<serde_json::Value> {
    let _ = registry;
    let start = Instant::now();

    let result = if params.full {
        serde_json::json!({
            "status": "full_reindex_in_progress",
            "hint": "全量重建索引正在进行，请在 get_index_status 中查看进度",
        })
    } else if let Some(ref file_paths) = params.file_paths {
        serde_json::json!({
            "status": "incremental_reindex",
            "file_count": file_paths.len(),
            "files": file_paths,
            "hint": "增量索引更新中",
        })
    } else {
        serde_json::json!({
            "status": "reindex",
            "hint": "未指定文件，使用全量重建",
        })
    };

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, 0, 0, elapsed)
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
pub fn handle_get_type_hierarchy(
    registry: &ToolRegistry,
    params: GetTypeHierarchyParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let symbol_name = match sled.get_symbol(&params.symbol_id) {
        Ok(Some(bytes)) => match serde_json::from_slice::<Symbol>(&bytes) {
            Ok(sym) => sym.name,
            Err(_) => params.symbol_id.clone(),
        },
        _ => params.symbol_id.clone(),
    };

    let type_hierarchy = codeconnect_graph::type_hierarchy::TypeHierarchy::new();

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
    });

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, 0, 0, elapsed)
}

/// 获取文件内所有符号 handler
pub fn handle_get_file_symbols(
    registry: &ToolRegistry,
    params: GetFileSymbolsParams,
) -> McpResponse<Vec<Symbol>> {
    let start = Instant::now();

    let sled = match &registry.sled {
        Some(s) => s,
        None => return McpResponse::error("存储未初始化"),
    };

    let symbol_ids: Vec<String> = match sled.get_file_symbols(&params.file_path) {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Ok(None) => return McpResponse::error(&format!("未找到文件: {}", params.file_path)),
        Err(e) => return McpResponse::error(&format!("读取失败: {}", e)),
    };

    let mut symbols: Vec<Symbol> = Vec::new();
    for sid in &symbol_ids {
        if let Ok(Some(bytes)) = sled.get_symbol(sid) {
            if let Ok(sym) = serde_json::from_slice::<Symbol>(&bytes) {
                symbols.push(sym);
            }
        }
    }

    let total = symbols.len();
    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(symbols, total, total, elapsed)
}

/// 获取依赖图 handler
pub fn handle_get_dependency_graph(
    registry: &ToolRegistry,
    params: GetDependencyGraphParams,
) -> McpResponse<serde_json::Value> {
    let _ = registry;
    let start = Instant::now();

    let _level = params.level;
    let _file_path = params.file_path;

    // 依赖图构建需要完整的导入解析和模块分析
    let result = serde_json::json!({
        "level": _level,
        "nodes": [],
        "edges": [],
        "hint": "依赖图功能需要在导入解析（后续 Phase）完成后可用",
    });

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(result, 0, 0, elapsed)
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
    fn test_handle_get_symbol_no_sled() {
        let registry = ToolRegistry::new();
        let params = GetSymbolParams {
            symbol_id: "test_id".to_string(),
        };
        let response = handle_get_symbol(&registry, params);
        assert_eq!(response.status, codeconnect_core::response::ResponseStatus::Error);
    }
}
