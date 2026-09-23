//! MCP 工具注册
//!
//! 注册所有 CodeConnect MCP 工具及对应的 handler 函数。
//! 所有工具均返回统一的 [`McpResponse`] 信封。
//!
//! ## 已注册工具列表
//!
//! 数量以 `server.rs` 中实际的 `#[rmcp::tool]` 标注为准，此处不写死数字以免漂移。
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
//! | `get_project_map` | 项目语义地图（上下文重建） | [`GetProjectMapParams`] |

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
    /// 索引最后构建时间（Unix 秒）
    ///
    /// `None` 表示索引由旧版本构建、未记录时间 —— 此时不得谎报「刚刚构建」，
    /// 响应中会提示索引时间未知。
    pub index_built_at_unix: Option<i64>,
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
            index_built_at_unix: None,
        }
    }

    /// 设置索引最后构建时间（Unix 秒）
    pub fn with_index_built_at(mut self, unix_secs: Option<i64>) -> Self {
        self.index_built_at_unix = unix_secs;
        self
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
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let limit = params.limit.min(100);

    // 语言/类型过滤下推到检索阶段 —— 此前是「先取 limit 条再在内存里过滤」，
    // 会导致「库里有却只回几条」的假象
    let results = match query_engine.search_by_name(
        &params.query,
        params.language.as_deref(),
        params.kind.as_deref(),
        limit,
    ) {
        Ok(r) => r,
        Err(e) => return McpResponse::error(&format!("搜索失败: {}", e)),
    };

    if results.is_empty() {
        return no_match_response(registry, &params, start);
    }

    // 搜索结果已包含完整的符号信息（从 tantivy STORED 字段）
    let symbols: Vec<serde_json::Value> = if params.detail == "full" {
        results
            .iter()
            .map(|r| {
                serde_json::to_value(codeconnect_index::query_engine::symbol_search_result_to_symbol(r))
                    .unwrap_or(serde_json::Value::Null)
            })
            .collect()
    } else {
        results.iter().map(brief_symbol_json).collect()
    };

    let total = symbols.len();
    let elapsed = start.elapsed().as_millis() as u64;

    McpResponse::success(serde_json::Value::Array(symbols), total, total, elapsed)
}

/// brief 模式下签名保留的最大字符数
const BRIEF_SIGNATURE_MAX_CHARS: usize = 150;

/// 构造 brief 模式的符号投影
///
/// 只保留「定位并进一步调用」所必需的字段。搜索用于定位，
/// 详情应由 get_symbol 按需获取，避免单次调用吃掉大量上下文。
fn brief_symbol_json(r: &codeconnect_index::tantivy_index::SymbolSearchResult) -> serde_json::Value {
    serde_json::json!({
        "symbol_id": r.stable_id,
        "name": r.name,
        "kind": r.kind,
        "language": r.language,
        "file_path": r.file_path,
        "line": r.line,
        "signature": truncate_chars(&r.signature, BRIEF_SIGNATURE_MAX_CHARS),
    })
}

/// 单次响应中列表类结果的数量硬上限
const MAX_RESULT_LIMIT: usize = 200;

/// 截断结果列表，返回（保留的条数, 真实总数）
///
/// 保留**真实总数**是为了让调用方知道「还有多少没拿到」——
/// 只回截断后的数量会让它以为这就是全部。
fn clip<T>(items: &[T], limit: usize) -> (usize, usize) {
    let total = items.len();
    let shown = total.min(limit.min(MAX_RESULT_LIMIT));
    (shown, total)
}

/// 生成截断告警文案；未截断时返回 None（不制造噪音）
fn truncation_warning(shown: usize, total: usize, what: &str) -> Option<String> {
    (total > shown).then(|| {
        format!(
            "{}共 {} 条，本次仅返回前 {} 条（已截断）。需要更多请调大 limit（上限 {}），或缩小查询范围。",
            what, total, shown, MAX_RESULT_LIMIT
        )
    })
}

/// 按字符数截断字符串（按字符而非字节，避免切断多字节字符）
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut truncated: String = s.chars().take(max).collect();
    truncated.push('…');
    truncated
}

/// 构造「无匹配」响应
///
/// 关键点：不返回看起来正常的空 Success —— 那会让调用方把「没匹配上」
/// 误读为「该符号不存在」并转而重复实现。此处显式给出相近候选与排查方向。
fn no_match_response(
    registry: &ToolRegistry,
    params: &SearchSymbolParams,
    start: Instant,
) -> McpResponse<serde_json::Value> {
    let mut message = format!("未找到匹配 '{}' 的符号。", params.query);

    if params.language.is_some() || params.kind.is_some() {
        message.push_str(&format!(
            "（当前已施加过滤：语言={} 类型={}，若不确定可去掉过滤重试）",
            params.language.as_deref().unwrap_or("不限"),
            params.kind.as_deref().unwrap_or("不限"),
        ));
    }

    message.push_str(&describe_similar_symbols(registry, &params.query));

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(serde_json::Value::Array(Vec::new()), 0, 0, elapsed)
        .with_warning(message)
}

/// 描述与给定名称相近的符号候选
///
/// 无候选时返回排查建议，使调用方能把「拼写不对」「语言未启用」
/// 「索引未建立」这几种情况区分开。
fn describe_similar_symbols(registry: &ToolRegistry, query: &str) -> String {
    let suggestions = registry
        .query_engine
        .as_ref()
        .and_then(|q| q.suggest_similar_names(query, 8).ok())
        .unwrap_or_default();

    if suggestions.is_empty() {
        return " 没有相近的符号名。请检查：① 符号名拼写；② 该语言是否在 .codeconnect.toml 的 [languages] 中启用；③ 是否已运行 codeconnect index 建立索引。".to_string();
    }

    let candidates: Vec<String> = suggestions
        .iter()
        .map(|s| format!("{} [{}] {}:{}", s.name, s.kind, s.file_path, s.line))
        .collect();
    format!(" 相近候选：{}", candidates.join(" | "))
}

// ============================================================================
// 符号引用解析 — 允许用符号名代替 symbol_id
// ============================================================================

/// 符号引用解析结果
enum SymbolRef {
    /// 唯一确定
    Resolved(Box<Symbol>),
    /// 匹配到多个候选，需调用方抉择
    Ambiguous(Vec<Symbol>),
    /// 既不是有效 ID，也不匹配任何名称
    NotFound,
}

/// 将「符号 ID 或符号名」解析为唯一符号
///
/// 工具入参历史上只接受精确 symbol_id，但调用方手里往往只有名字，
/// 被迫先 search_symbol 拿 ID 再调目标工具，每次多一轮往返。
/// 此处允许直接传名字：精确 ID 命中直接用；否则按名称检索，
/// 唯一匹配则采用、多个匹配返回候选、无匹配给出相近建议。
fn resolve_symbol_ref(registry: &ToolRegistry, input: &str) -> Result<SymbolRef, String> {
    let query_engine = registry
        .query_engine
        .as_ref()
        .ok_or_else(|| "查询引擎未初始化".to_string())?;

    // 快路径：输入本身就是精确 symbol_id
    if let Ok(Some(symbol)) = query_engine.get_symbol_by_id(input) {
        return Ok(SymbolRef::Resolved(Box::new(symbol)));
    }

    // 慢路径：按名称检索
    let matches = match query_engine.search_by_name(input, None, None, 20) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("按名称解析符号 '{}' 失败: {}", input, e);
            return Ok(SymbolRef::NotFound);
        }
    };

    // 名称完全一致的优先，避免被前缀命中淹没
    let exact: Vec<Symbol> = matches
        .iter()
        .filter(|r| r.name == input)
        .map(codeconnect_index::query_engine::symbol_search_result_to_symbol)
        .collect();

    let candidates: Vec<Symbol> = if exact.is_empty() {
        matches
            .iter()
            .map(codeconnect_index::query_engine::symbol_search_result_to_symbol)
            .collect()
    } else {
        exact
    };

    match candidates.len() {
        0 => Ok(SymbolRef::NotFound),
        1 => Ok(SymbolRef::Resolved(Box::new(
            candidates.into_iter().next().expect("长度已确认为 1"),
        ))),
        _ => Ok(SymbolRef::Ambiguous(candidates)),
    }
}

/// 解析符号引用，失败时直接返回构造好的响应
///
/// 泛型 `T` 使各 handler 无需转换返回类型即可直接 `return` 该响应。
/// 失败一律显式报错并附候选 —— 不再像此前那样把查不到的输入当作名称继续空跑。
fn resolve_or_respond<T: serde::Serialize>(
    registry: &ToolRegistry,
    input: &str,
    tool: &str,
) -> Result<Symbol, McpResponse<T>> {
    match resolve_symbol_ref(registry, input) {
        Ok(SymbolRef::Resolved(symbol)) => Ok(*symbol),
        Ok(SymbolRef::Ambiguous(candidates)) => {
            let listed: Vec<String> = candidates
                .iter()
                .take(10)
                .map(|s| format!("{} ({}:{})", s.id, s.location.file_path, s.location.line))
                .collect();
            Err(McpResponse::error(&format!(
                "{}: 符号引用 '{}' 不唯一，匹配到 {} 个符号，请改用其中一个完整 symbol_id：{}",
                tool,
                input,
                candidates.len(),
                listed.join(" | ")
            )))
        }
        Ok(SymbolRef::NotFound) => Err(McpResponse::error(&format!(
            "{}: 未找到符号 '{}' —— 它既不是有效的 symbol_id，也不匹配任何已索引的符号名。{}",
            tool,
            input,
            describe_similar_symbols(registry, input)
        ))),
        Err(e) => Err(McpResponse::error(&format!("{}: {}", tool, e))),
    }
}

/// 获取符号详情 handler
///
/// 返回值是符号对象本身，`include_source` 为真时额外附加 `source` 字段。
pub fn handle_get_symbol(
    registry: &ToolRegistry,
    params: GetSymbolParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    // 允许直接传符号名，内部解析为唯一符号
    let symbol = match resolve_or_respond(registry, &params.symbol_id, "get_symbol") {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    // 保留原有字段布局，仅追加 source，既有调用方读 data.name 等仍可用
    let mut data = serde_json::to_value(&symbol).unwrap_or(serde_json::Value::Null);
    if params.include_source {
        if let Some(obj) = data.as_object_mut() {
            obj.insert("source".to_string(), extract_source(registry, &symbol));
        }
    }

    let elapsed = start.elapsed().as_millis() as u64;
    McpResponse::success(data, 1, 1, elapsed)
}

/// 单个符号返回的源码行数上限
const SOURCE_MAX_LINES: usize = 200;

/// 单个符号返回的源码字符数上限
///
/// 只限行数挡不住 min.js / 生成代码里的超长单行 ——
/// 200 行 × 每行上万字符足以一次吃掉整个上下文。
const SOURCE_MAX_CHARS: usize = 8000;

/// `get_file_symbols` 单次响应附带的源码字符总量上限
///
/// 单符号有上限不代表整次响应有上限 —— 一个文件几百个符号叠加起来仍会把上下文吃光。
const FILE_SOURCE_TOTAL_MAX_CHARS: usize = 40000;

/// 提取符号对应的源码片段
///
/// 索引中已记录 `file_path` / `line` / `end_line`，直接对源文件切片即可 ——
/// 这一步让「看这个函数怎么写的」从「read 整个文件」降为一次工具调用。
///
/// 任何取不到源码的情况都如实返回 `available: false` 及原因，
/// **不返回空串伪装成功** —— 否则调用方会把「文件读不到」当成「函数是空的」。
fn extract_source(registry: &ToolRegistry, symbol: &Symbol) -> serde_json::Value {
    let Some(root) = registry.project_root.as_ref() else {
        return serde_json::json!({
            "available": false,
            "reason": "未配置项目根目录，无法定位源文件",
        });
    };

    let relative = symbol.location.file_path.clone();
    let path = root.join(&relative);

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({
                "available": false,
                "file_path": relative,
                "reason": format!("读取源文件失败（文件可能已删除或移出项目）: {}", e),
            });
        }
    };

    slice_source(&content, &relative, symbol)
}

/// 从已读取的文件内容中切出符号对应的源码片段
///
/// 与 [`extract_source`] 分离，是为了让 `get_file_symbols` 只读一次盘 ——
/// 否则同一文件有多少符号就要读多少次。
fn slice_source(content: &str, relative: &str, symbol: &Symbol) -> serde_json::Value {
    let lines: Vec<&str> = content.lines().collect();
    let start = symbol.location.line.max(1) as usize;
    if start > lines.len() {
        return serde_json::json!({
            "available": false,
            "file_path": relative,
            "reason": format!(
                "行号越界：索引记录该符号位于第 {} 行，但文件只有 {} 行 —— 索引可能已过期，请先调用 reindex",
                start,
                lines.len()
            ),
        });
    }

    // end_line 是脏数据时（早于 line，或与 line 同为 0）退化为单行。
    // 注意必须先与钳制后的 start 取 max：只比 line 会漏掉 line=0/end_line=0 的情形，
    // 那样 end_clamped - start 会 usize 下溢，release 下回绕成「空源码 + 成功」
    let end = (symbol.location.end_line as usize).max(start);
    let end_clamped = end.min(lines.len()).max(start);
    let over_line_limit = (end_clamped - start + 1) > SOURCE_MAX_LINES;
    let slice_end = (start + SOURCE_MAX_LINES - 1).min(end_clamped).max(start);

    let mut code = lines[start - 1..slice_end].join("\n");
    let mut truncated = end > lines.len() || over_line_limit;
    let mut truncation_reason: Option<&str> = over_line_limit.then_some("超出行数上限");

    if code.chars().count() > SOURCE_MAX_CHARS {
        code = truncate_chars(&code, SOURCE_MAX_CHARS);
        truncated = true;
        truncation_reason = Some("超出字符数上限");
    }

    let mut result = serde_json::json!({
        "available": true,
        "file_path": relative,
        "start_line": start,
        "end_line": slice_end,
        "code": code,
        "truncated": truncated,
    });
    if let Some(reason) = truncation_reason {
        if let Some(obj) = result.as_object_mut() {
            obj.insert(
                "truncated_reason".to_string(),
                serde_json::Value::String(reason.to_string()),
            );
        }
    }
    result
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

    // 支持直接传符号名 —— 解析为唯一符号后取其名称
    let target = match resolve_or_respond(registry, &params.symbol_id, "trace_callers") {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let all_callers = call_graph.trace_callers(&target.name, params.max_depth);
    // 热点函数可能有数千个调用者，必须截断后再返回
    let (shown, total_callers) = clip(&all_callers, params.limit);
    let callers = &all_callers[..shown];

    // 构建 JSON 响应
    let result = serde_json::json!({
        "target": {
            "symbol_id": target.id,
            "name": target.name,
        },
        "callers": callers.iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "total_callers": total_callers,
        "returned_callers": shown,
        "truncated": total_callers > shown,
    });

    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(result, total_callers, shown, elapsed);
    if let Some(w) = truncation_warning(shown, total_callers, "调用者") {
        response = response.with_warning(w);
    }
    response
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

    // 支持直接传符号名 —— 解析为唯一符号后取其名称
    let source = match resolve_or_respond(registry, &params.symbol_id, "trace_callees") {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let all_callees = call_graph.trace_callees(&source.name, params.max_depth);
    let (shown, total_callees) = clip(&all_callees, params.limit);
    let callees = &all_callees[..shown];

    let result = serde_json::json!({
        "source": {
            "symbol_id": source.id,
            "name": source.name,
        },
        "callees": callees.iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "total_callees": total_callees,
        "returned_callees": shown,
        "truncated": total_callees > shown,
    });

    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(result, total_callees, shown, elapsed);
    if let Some(w) = truncation_warning(shown, total_callees, "被调用者") {
        response = response.with_warning(w);
    }
    response
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

    // 解析每个符号引用（ID 或名称）为唯一符号
    let mut symbol_names: Vec<String> = Vec::new();
    for sid in &params.symbol_ids {
        let symbol = match resolve_or_respond(registry, sid, "analyze_impact") {
            Ok(s) => s,
            Err(resp) => return resp,
        };
        symbol_names.push(symbol.name);
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

    // 影响面可能很大，两个列表都要截断
    let total_affected = report.total_affected();
    let (shown_direct, _) = clip(&direct_impacts, params.limit / 2 + 1);
    let (shown_trans, total_trans) = clip(&transitive_impacts, params.limit / 2);
    let shown_total = shown_direct + shown_trans;
    let total_all = direct_impacts.len() + total_trans;

    let result = serde_json::json!({
        "changed_symbols": params.symbol_ids.iter().enumerate().map(|(i, sid)| {
            serde_json::json!({
                "symbol_id": sid,
                "name": symbol_names.get(i).unwrap_or(sid),
            })
        }).collect::<Vec<_>>(),
        "direct_impacts": direct_impacts[..shown_direct],
        "transitive_impacts": transitive_impacts[..shown_trans],
        "total_affected": total_affected,
        "returned_impacts": shown_total,
        "truncated": total_all > shown_total,
        "max_depth": params.max_depth,
    });

    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(result, total_affected, shown_total, elapsed);
    if let Some(w) = truncation_warning(shown_total, total_all, "受影响符号") {
        response = response.with_warning(w);
    }
    response
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

    // 支持直接传符号名
    let center = match resolve_or_respond(registry, &params.symbol_id, "get_call_graph") {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let all_callers = call_graph.trace_callers(&center.name, params.caller_depth);
    let all_callees = call_graph.trace_callees(&center.name, params.callee_depth);

    // 两个方向各分一半配额
    let (shown_callers, total_callers) = clip(&all_callers, params.limit / 2);
    let (shown_callees, total_callees) = clip(&all_callees, params.limit / 2 + 1);
    let shown_total = shown_callers + shown_callees;
    let total_all = total_callers + total_callees;

    let result = serde_json::json!({
        "center": {
            "symbol_id": center.id,
            "name": center.name,
        },
        "callers": all_callers[..shown_callers].iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "callees": all_callees[..shown_callees].iter().map(|n| {
            serde_json::json!({
                "symbol_id": n.symbol_id,
                "name": n.name,
                "depth": n.depth,
                "call_type": n.call_type,
            })
        }).collect::<Vec<_>>(),
        "total_nodes": total_all + 1,
        "returned_nodes": shown_total + 1,
        "truncated": total_all > shown_total,
    });

    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(result, total_all, shown_total, elapsed);
    if let Some(w) = truncation_warning(shown_total, total_all, "调用子图节点") {
        response = response.with_warning(w);
    }
    response
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

        // 一个文件可能有几百个符号，指标列表必须截断
        let (shown_metrics, total_metrics) = clip(&metrics, params.limit);

        let result = serde_json::json!({
            "file_path": file_path,
            "symbol_count": symbols.len(),
            "metrics": metrics[..shown_metrics].iter().map(|m| {
                serde_json::json!({
                    "symbol_id": m.symbol_id,
                    "name": m.name,
                    "cyclomatic_complexity": m.cyclomatic_complexity,
                    "fan_in": m.fan_in,
                    "fan_out": m.fan_out,
                    "depth_of_inheritance": m.depth_of_inheritance,
                })
            }).collect::<Vec<_>>(),
            "returned_metrics": shown_metrics,
            "truncated": total_metrics > shown_metrics,
        });

        let elapsed = start.elapsed().as_millis() as u64;
        let mut response = McpResponse::success(result, total_metrics, shown_metrics, elapsed);
        if let Some(w) = truncation_warning(shown_metrics, total_metrics, "文件内符号指标") {
            response = response.with_warning(w);
        }
        return response;
    }

    // 如果指定了单个符号（ID 或名称）
    if let Some(ref symbol_id) = params.symbol_id {
        let symbol = match resolve_or_respond(registry, symbol_id, "get_metrics") {
            Ok(s) => s,
            Err(resp) => return resp,
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

    // 大型项目的死代码可能有几千条，必须截断
    let (shown_dead, total_dead) = clip(&dead_entries, params.limit);

    let result = serde_json::json!({
        "entry_points": entry_points,
        "total_symbols": all_symbols.len(),
        "dead_code_count": total_dead,
        "dead_entries": dead_entries[..shown_dead].iter().map(|d| {
            serde_json::json!({
                "symbol_id": d.symbol_id,
                "name": d.name,
                "confidence": d.confidence,
                "reason": d.reason,
            })
        }).collect::<Vec<_>>(),
        "returned_entries": shown_dead,
        "truncated": total_dead > shown_dead,
    });

    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(result, total_dead, shown_dead, elapsed);
    if let Some(w) = truncation_warning(shown_dead, total_dead, "死代码条目") {
        response = response.with_warning(w);
    }
    response
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

    // 支持直接传符号名，解析为唯一符号
    let target = match resolve_or_respond(registry, &params.symbol_id, "find_references") {
        Ok(s) => s,
        Err(resp) => return resp,
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

    let callers = call_graph.trace_callers(&target.name, 10);

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
            "symbol_id": target.id,
            "name": target.name,
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

    // 收集索引存储实例：优先用已加载的共享引用；未加载时从 data_dir 自举打开（open_or_create 会自动创建缺失目录）
    let data_dir = registry.data_dir.as_ref().unwrap();

    let tantivy = match &registry.tantivy {
        Some(t) => Arc::clone(t),
        None => match TantivyIndex::open_or_create(&data_dir.join("tantivy")) {
            Ok(t) => Arc::new(t),
            Err(e) => return McpResponse::error(&format!("tantivy 索引自举打开失败: {}", e)),
        },
    };
    let sled = match &registry.sled {
        Some(s) => Arc::clone(s),
        None => match SledStore::open(&data_dir.join("sled")) {
            Ok(s) => Arc::new(s),
            Err(e) => return McpResponse::error(&format!("sled 存储自举打开失败: {}", e)),
        },
    };
    let call_edge_index = match &registry.call_edge_index {
        Some(c) => Arc::clone(c),
        None => match CallEdgeIndex::open_or_create(&data_dir.join("tantivy_edges")) {
            Ok(c) => Arc::new(c),
            Err(e) => return McpResponse::error(&format!("调用边索引自举打开失败: {}", e)),
        },
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

    // 支持直接传符号名（用于在层次图中查找）
    let target = match resolve_or_respond(registry, &params.symbol_id, "get_type_hierarchy") {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let symbol_name = target.name.clone();

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

    // 继承链两个方向都要截断（深层继承树可能很长）
    let (shown_anc, total_anc) = clip(&ancestors, params.limit / 2);
    let (shown_desc, total_desc) = clip(&descendants, params.limit / 2 + 1);
    let shown_total = shown_anc + shown_desc;
    let total_all = total_anc + total_desc;

    let result = serde_json::json!({
        "target": {
            "symbol_id": target.id,
            "name": symbol_name,
        },
        "ancestors": ancestors[..shown_anc],
        "descendants": descendants[..shown_desc],
        "returned_types": shown_total,
        "truncated": total_all > shown_total,
        "graph_stats": {
            "total_types": type_hierarchy.node_count(),
            "total_edges": type_hierarchy.edge_count(),
        },
    });

    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(result, total_all, shown_total, elapsed);
    if let Some(w) = truncation_warning(shown_total, total_all, "继承链节点") {
        response = response.with_warning(w);
    }
    response
}

/// 获取文件内所有符号 handler
///
/// 直接从 tantivy 按 file_path 精确搜索，不再通过 sled 的 file_symbols 映射。
pub fn handle_get_file_symbols(
    registry: &ToolRegistry,
    params: GetFileSymbolsParams,
) -> McpResponse<serde_json::Value> {
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

    // include_source 缺省关闭：一个文件的符号可能很多，
    // 全部带源码会一次吃掉大量上下文
    let mut source_budget_exhausted = false;
    let data: Vec<serde_json::Value> = if params.include_source {
        // 整份文件只读一次 —— 按符号逐个读会把同一文件读 N 遍
        let relative = symbols[0].location.file_path.clone();
        let content = registry
            .project_root
            .as_ref()
            .and_then(|root| std::fs::read_to_string(root.join(&relative)).ok());

        let mut used_chars = 0usize;
        symbols
            .iter()
            .map(|s| {
                let mut v = serde_json::to_value(s).unwrap_or(serde_json::Value::Null);
                let source = match &content {
                    Some(c) if used_chars < FILE_SOURCE_TOTAL_MAX_CHARS => {
                        let sliced = slice_source(c, &relative, s);
                        used_chars += sliced
                            .get("code")
                            .and_then(|c| c.as_str())
                            .map(|c| c.chars().count())
                            .unwrap_or(0);
                        sliced
                    }
                    Some(_) => {
                        source_budget_exhausted = true;
                        serde_json::json!({
                            "available": false,
                            "reason": format!(
                                "本次响应的源码总量已达上限（{} 字符），后续符号未附源码；请用 get_symbol 按需单独获取",
                                FILE_SOURCE_TOTAL_MAX_CHARS
                            ),
                        })
                    }
                    None => serde_json::json!({
                        "available": false,
                        "file_path": relative,
                        "reason": "读取源文件失败（文件可能已删除或移出项目）",
                    }),
                };
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("source".to_string(), source);
                }
                v
            })
            .collect()
    } else {
        symbols
            .iter()
            .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
            .collect()
    };

    let total = data.len();
    let elapsed = start.elapsed().as_millis() as u64;
    let response = McpResponse::success(serde_json::Value::Array(data), total, total, elapsed);

    // 预算耗尽必须说出来，不能让调用方以为「这些符号本来就没有源码」
    if source_budget_exhausted {
        response.with_warning(format!(
            "本次响应的源码总量已达 {} 字符上限，后续符号未附源码。需要逐个查看请改用 get_symbol。",
            FILE_SOURCE_TOTAL_MAX_CHARS
        ))
    } else {
        response
    }
}

// ============================================================================
// 项目地图 — 上下文重建
// ============================================================================

/// 项目地图的详细程度，按信息量从高到低排列
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapLevel {
    /// 模块 → 文件 → 符号名 + 截断签名
    Detailed,
    /// 模块 → 文件 → 符号名
    Names,
    /// 模块 → 文件与符号计数
    Counts,
    /// 仅总量与语言分布
    Summary,
}

impl MapLevel {
    fn name(self) -> &'static str {
        match self {
            MapLevel::Detailed => "detailed",
            MapLevel::Names => "names",
            MapLevel::Counts => "counts",
            MapLevel::Summary => "summary",
        }
    }
}

/// `counts` 层级下每个文件列出的关键符号数
const COUNT_LEVEL_SYMBOLS_PER_FILE: usize = 6;

/// 粗略估算文本的 token 开销
///
/// 代码约 4 字符/token，中文约 1.5 字符/token，取 3 作偏保守的折中值 ——
/// 宁可低估预算导致提前降级，也不要把上下文撑爆。
fn estimate_tokens(text: &str) -> usize {
    (text.chars().count() / 3).max(1)
}

/// 获取项目地图 handler
///
/// 在上下文丢失后一次性重建对项目的整体认知，
/// 替代逐个文件重读。结果同时落盘供 CLAUDE.md 引用。
pub fn handle_get_project_map(
    registry: &ToolRegistry,
    params: GetProjectMapParams,
) -> McpResponse<serde_json::Value> {
    let start = Instant::now();

    let query_engine = match &registry.query_engine {
        Some(q) => q,
        None => return McpResponse::error("查询引擎未初始化"),
    };

    let (all, skipped_docs) = match query_engine.scan_all_symbols() {
        Ok(s) => s,
        Err(e) => return McpResponse::error(&format!("扫描符号失败: {}", e)),
    };

    if all.is_empty() {
        return McpResponse::error("索引中没有符号。请先运行 `codeconnect index` 建立索引。");
    }

    let focus = params
        .focus
        .as_deref()
        .map(|f| f.trim_matches('/').to_string())
        .filter(|f| !f.is_empty());

    let scoped: Vec<&codeconnect_index::tantivy_index::SymbolSearchResult> = match focus.as_deref() {
        Some(f) => {
            // 按路径段匹配，而非裸前缀 —— 否则 focus="crates/index" 会命中 crates/index_old/
            let prefix = format!("{}/", f);
            all.iter()
                .filter(|s| s.file_path == f || s.file_path.starts_with(&prefix))
                .collect()
        }
        None => all.iter().collect(),
    };

    if scoped.is_empty() {
        return McpResponse::error(&format!(
            "focus 路径 '{}' 下没有任何已索引符号。请确认该路径相对于项目根目录（如 crates/index），或去掉 focus 查看全量。",
            params.focus.as_deref().unwrap_or("")
        ));
    }

    // 按预算逐级降级：宁可少给信息并明确说明，也不静默截断
    let budget = params.budget_tokens.max(200);
    let mut selected: Option<(MapLevel, String)> = None;
    for level in [
        MapLevel::Detailed,
        MapLevel::Names,
        MapLevel::Counts,
        MapLevel::Summary,
    ] {
        let candidate = render_project_map(&scoped, &all, level, focus.as_deref(), registry, false);
        if estimate_tokens(&candidate) <= budget {
            selected = Some((level, candidate));
            break;
        }
    }

    let (level, map, over_budget) = match selected {
        Some((l, m)) => (l, m, false),
        // 连最简形式都超预算：仍然返回它，但必须让调用方知道
        None => (
            MapLevel::Summary,
            render_project_map(&scoped, &all, MapLevel::Summary, focus.as_deref(), registry, false),
            true,
        ),
    };

    let estimated = estimate_tokens(&map);
    let degraded = level != MapLevel::Detailed;
    let scoped_files = scoped
        .iter()
        .map(|s| s.file_path.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();

    // 落盘写**全量**地图：不受预算降级影响、不设每文件符号数上限。
    // 响应受预算约束是为了省上下文，但文件是持久记录 ——
    // 需要细节时让 AI 直接 Read/Grep 该文件，而不是把全量塞进响应。
    let mut written_to: Option<String> = None;
    let mut write_error: Option<String> = None;
    let mut file_meta: Option<(usize, usize)> = None; // (字节数, 行数)
    if params.write_file {
        match registry.data_dir.as_ref() {
            Some(data_dir) => {
                let full_map = render_project_map(
                    &scoped,
                    &all,
                    MapLevel::Detailed,
                    focus.as_deref(),
                    registry,
                    true,
                );
                let path = data_dir.join("PROJECT_MAP.md");
                match std::fs::write(&path, &full_map) {
                    Ok(()) => {
                        file_meta = Some((full_map.len(), full_map.lines().count()));
                        written_to = Some(path.display().to_string());
                    }
                    Err(e) => write_error = Some(format!("写入 {} 失败: {}", path.display(), e)),
                }
            }
            None => write_error = Some("未配置数据目录，地图未落盘".to_string()),
        }
    }

    let data = serde_json::json!({
        "map": map,
        "response_level": level.name(),
        "response_degraded": degraded,
        "response_estimated_tokens": estimated,
        "budget_tokens": budget,
        "written_to": written_to,
        "file_bytes": file_meta.map(|(bytes, _)| bytes),
        "file_lines": file_meta.map(|(_, lines)| lines),
        "file_is_complete": file_meta.is_some(),
        "scope": focus.clone().unwrap_or_else(|| "全项目".to_string()),
        "total_symbols": all.len(),
        "scoped_symbols": scoped.len(),
        "scoped_files": scoped_files,
    });

    let total = scoped.len();
    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(data, total, total, elapsed);

    // 降级只影响「本次响应」，落盘文件始终是全量 —— 说明清楚，
    // 否则调用方会以为细节永久丢失了
    let file_note = match (&written_to, &write_error) {
        (Some(path), _) => format!(
            "本次响应仅为摘要；全量地图（含全部符号）已写入 {}，需要细节请直接读取该文件。",
            path
        ),
        (None, Some(err)) => format!("注意：全量地图未能落盘（{}），本次响应即为全部内容。", err),
        (None, None) => "注意：本次未落盘（write_file=false），本次响应即为全部内容。".to_string(),
    };

    if over_budget {
        response = response.with_warning(format!(
            "项目地图已降到最简形式，仍超出 {} token 预算（约 {} token）。请改用 focus 缩小范围，或调大 budget_tokens。{}",
            budget, estimated, file_note
        ));
    } else if degraded {
        response = response.with_warning(format!(
            "为适配 {} token 预算，本次响应已降级为 '{}' 层级（省略了部分细节）。{}",
            budget,
            level.name(),
            file_note
        ));
    }

    if skipped_docs > 0 {
        response = response.with_warning(format!(
            "扫描时有 {} 个索引文档读取失败或已损坏，已被跳过 —— 地图内容与计数可能偏低，建议运行一次 `codeconnect index -f` 重建索引。",
            skipped_docs
        ));
    }

    if let Some(err) = write_error {
        response = response.with_warning(err);
    }

    response
}

/// 渲染项目地图文本
///
/// `full` 为真时用于**落盘**：不受预算降级影响、不设每文件符号数上限，
/// 保留全部符号 —— 该文件是持久记录，供后续 Read/Grep 当作项目索引使用。
/// 为假时用于**响应**：按传入层级渲染，受每文件条数上限约束。
fn render_project_map(
    scoped: &[&codeconnect_index::tantivy_index::SymbolSearchResult],
    all: &[codeconnect_index::tantivy_index::SymbolSearchResult],
    level: MapLevel,
    focus: Option<&str>,
    registry: &ToolRegistry,
    full: bool,
) -> String {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fmt::Write as _;

    // 落盘走最高详细度，且下面把每文件上限放开
    let level = if full { MapLevel::Detailed } else { level };

    let mut out = String::new();

    // ---- 汇总 ----
    let mut lang_stats: BTreeMap<&str, (usize, BTreeSet<&str>)> = BTreeMap::new();
    let mut file_stats: BTreeMap<&str, Vec<&codeconnect_index::tantivy_index::SymbolSearchResult>> =
        BTreeMap::new();
    for s in scoped {
        let entry = lang_stats.entry(s.language.as_str()).or_default();
        entry.0 += 1;
        entry.1.insert(s.file_path.as_str());
        file_stats.entry(s.file_path.as_str()).or_default().push(s);
    }

    let _ = writeln!(out, "# CodeConnect 项目地图");
    let _ = writeln!(out);
    let scope_note = match focus {
        Some(f) => format!("范围: {}（全项目共 {} 符号）", f, all.len()),
        None => "范围: 全项目".to_string(),
    };
    let _ = writeln!(
        out,
        "> {} | 本范围 {} 符号 / {} 文件",
        scope_note,
        scoped.len(),
        file_stats.len()
    );
    match registry.index_built_at_unix {
        Some(built_at) => {
            let age = crate::server::now_unix_secs().saturating_sub(built_at).max(0) as u64;
            let _ = writeln!(
                out,
                "> 索引最后更新于 {} 前 —— 若与实际代码不符，请先调用 reindex",
                crate::server::humanize_duration(age)
            );
        }
        None => {
            let _ = writeln!(out, "> 索引更新时间未知（旧索引），内容可能已过期");
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## 语言分布");
    for (lang, (count, files)) in &lang_stats {
        let _ = writeln!(out, "- {}: {} 符号 / {} 文件", lang, count, files.len());
    }

    // ---- 最简层级：仍须回答「项目由哪些模块构成」----
    if level == MapLevel::Summary {
        let mut dir_totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for (file, syms) in &file_stats {
            let dir = match file.rfind('/') {
                Some(i) => file[..i].to_string(),
                None => ".".to_string(),
            };
            let entry = dir_totals.entry(dir).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += syms.len();
        }
        let mut list: Vec<_> = dir_totals.into_iter().collect();
        list.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then_with(|| a.0.cmp(&b.0)));

        let _ = writeln!(out);
        let _ = writeln!(out, "## 主要模块");
        for (dir, (files, symbols)) in list {
            let _ = writeln!(out, "- {}/  ({} 文件 / {} 符号)", dir, files, symbols);
        }
    }

    // ---- 模块明细 ----
    if level != MapLevel::Summary {
        let mut dirs: BTreeMap<String, Vec<(&str, &Vec<&codeconnect_index::tantivy_index::SymbolSearchResult>)>> =
            BTreeMap::new();
        for (file, syms) in &file_stats {
            let dir = match file.rfind('/') {
                Some(i) => file[..i].to_string(),
                None => ".".to_string(),
            };
            dirs.entry(dir).or_default().push((file, syms));
        }

        // 符号密集的目录优先展示
        let mut dir_list: Vec<_> = dirs.into_iter().collect();
        dir_list.sort_by(|a, b| {
            let ca: usize = a.1.iter().map(|(_, s)| s.len()).sum();
            let cb: usize = b.1.iter().map(|(_, s)| s.len()).sum();
            cb.cmp(&ca).then_with(|| a.0.cmp(&b.0))
        });

        let _ = writeln!(out);
        let _ = writeln!(out, "## 模块明细");

        for (dir, mut files_in_dir) in dir_list {
            let dir_total: usize = files_in_dir.iter().map(|(_, s)| s.len()).sum();
            let _ = writeln!(out);
            let _ = writeln!(
                out,
                "### {}/  —  {} 文件 / {} 符号",
                dir,
                files_in_dir.len(),
                dir_total
            );

            files_in_dir.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));
            for (file, syms) in files_in_dir {
                let _ = writeln!(out, "- {}  ({} 符号)", file, syms.len());

                let mut sorted = syms.clone();
                sorted.sort_by(|a, b| {
                    b.is_exported
                        .cmp(&a.is_exported)
                        .then_with(|| a.name.cmp(&b.name))
                });

                // 计数层级也列出每个文件最关键的几个符号：只给计数的话，
                // 「重建项目认知」拿不到任何具体名字，价值所剩无几
                if level == MapLevel::Counts {
                    for s in sorted.iter().take(COUNT_LEVEL_SYMBOLS_PER_FILE) {
                        let _ = writeln!(out, "    - {}", s.name);
                    }
                    if sorted.len() > COUNT_LEVEL_SYMBOLS_PER_FILE {
                        let _ = writeln!(
                            out,
                            "    - …… 另有 {} 个符号未列出",
                            sorted.len() - COUNT_LEVEL_SYMBOLS_PER_FILE
                        );
                    }
                    continue;
                }

                // 落盘时不设每文件上限，列出该文件全部符号
                let cap = if full {
                    usize::MAX
                } else {
                    match level {
                        MapLevel::Detailed => 40,
                        _ => 25,
                    }
                };
                for s in sorted.iter().take(cap) {
                    if level == MapLevel::Detailed {
                        let sig = truncate_chars(&s.signature, 90);
                        if sig.is_empty() {
                            let _ = writeln!(out, "    - {} [{}] :{}", s.name, s.kind, s.line);
                        } else {
                            let _ = writeln!(
                                out,
                                "    - {} [{}] {} :{}",
                                s.name, s.kind, sig, s.line
                            );
                        }
                    } else {
                        let _ = writeln!(out, "    - {}", s.name);
                    }
                }
                if sorted.len() > cap {
                    let _ = writeln!(out, "    - …… 另有 {} 个符号未列出", sorted.len() - cap);
                }
            }
        }
    }

    // ---- 入口点线索 ----
    let entries: Vec<&&codeconnect_index::tantivy_index::SymbolSearchResult> = scoped
        .iter()
        .filter(|s| {
            s.name == "main"
                || (s.is_exported
                    && (s.file_path.ends_with("lib.rs")
                        || s.file_path.ends_with("index.ts")
                        || s.file_path.ends_with("index.js")))
        })
        .collect();
    if !entries.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## 入口点线索");
        for s in entries.iter().take(30) {
            let _ = writeln!(
                out,
                "- {} [{}] {}:{}",
                s.name, s.kind, s.file_path, s.line
            );
        }
    }

    out
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

    // 完整依赖图的节点与边都可能上万，两个列表都要截断
    let (shown_nodes, total_nodes) = clip(&filtered_nodes, params.limit / 2);
    let (shown_edges, total_edges) = clip(&filtered_edges, params.limit / 2 + 1);

    let result = serde_json::json!({
        "level": params.level,
        "nodes": filtered_nodes[..shown_nodes].iter().map(|n| {
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "kind": format!("{:?}", n.kind),
            })
        }).collect::<Vec<_>>(),
        "edges": filtered_edges[..shown_edges].iter().map(|(src, tgt, edge)| {
            serde_json::json!({
                "source": src.id,
                "target": tgt.id,
                "edge_type": edge.edge_type,
                "count": edge.count,
            })
        }).collect::<Vec<_>>(),
        "total_nodes": total_nodes,
        "total_edges": total_edges,
        "returned_nodes": shown_nodes,
        "returned_edges": shown_edges,
        "truncated": total_nodes > shown_nodes || total_edges > shown_edges,
    });

    let elapsed = start.elapsed().as_millis() as u64;
    let mut response = McpResponse::success(result, total_nodes, shown_nodes, elapsed);
    if let Some(w) = truncation_warning(shown_nodes + shown_edges, total_nodes + total_edges, "依赖图节点与边") {
        response = response.with_warning(w);
    }
    response
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
            detail: "brief".to_string(),
        };
        let response = handle_search_symbol(&registry, params);
        assert_eq!(response.status, codeconnect_core::response::ResponseStatus::Error);
    }

    #[test]
    fn test_handle_get_symbol_no_query_engine() {
        let registry = ToolRegistry::new();
        let params = GetSymbolParams {
            symbol_id: "test_id".to_string(),
            include_source: true,
        };
        let response = handle_get_symbol(&registry, params);
        assert_eq!(response.status, codeconnect_core::response::ResponseStatus::Error);
    }

    // ===== 源码切片 =====

    /// 构造一个仅含 project_root 的注册表，用于测试源码切片
    fn registry_with_root(root: &std::path::Path) -> ToolRegistry {
        ToolRegistry::new().with_project_root(root.to_path_buf())
    }

    fn symbol_at(file_path: &str, line: u64, end_line: u64) -> Symbol {
        Symbol {
            id: "rust::src/demo.rs::function::demo::aaaa".to_string(),
            name: "demo".to_string(),
            kind: codeconnect_core::types::SymbolKind::Function,
            location: codeconnect_core::types::SourceLocation {
                file_path: file_path.to_string(),
                line,
                column: 1,
                end_line,
                end_column: 1,
            },
            signature: None,
            doc_comment: None,
            parent_id: None,
            modifiers: Vec::new(),
            is_exported: false,
            complexity: None,
        }
    }

    #[test]
    fn test_extract_source_returns_exact_line_range() {
        let dir = std::env::temp_dir().join("cc_source_slice_test");
        let _ = std::fs::create_dir_all(dir.join("src"));
        std::fs::write(
            dir.join("src/demo.rs"),
            "line1\nline2\nfn demo() {\n    body\n}\nline6\n",
        )
        .unwrap();

        let registry = registry_with_root(&dir);
        let result = extract_source(&registry, &symbol_at("src/demo.rs", 3, 5));

        assert_eq!(result["available"], true, "应成功取到源码: {}", result);
        assert_eq!(result["code"], "fn demo() {\n    body\n}");
        assert_eq!(result["start_line"], 3);
        assert_eq!(result["end_line"], 5);
        assert_eq!(result["truncated"], false);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_reports_missing_file_not_empty_string() {
        let dir = std::env::temp_dir().join("cc_source_missing_test");
        let _ = std::fs::create_dir_all(&dir);

        let registry = registry_with_root(&dir);
        let result = extract_source(&registry, &symbol_at("src/gone.rs", 1, 2));

        // 关键：不能返回 available=true + 空 code（那会被读成「函数是空的」）
        assert_eq!(result["available"], false, "文件缺失必须如实报告: {}", result);
        assert!(result["reason"].as_str().unwrap().contains("读取源文件失败"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_reports_out_of_range_lines() {
        let dir = std::env::temp_dir().join("cc_source_range_test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("short.rs"), "only_one_line\n").unwrap();

        let registry = registry_with_root(&dir);
        let result = extract_source(&registry, &symbol_at("short.rs", 99, 120));

        assert_eq!(result["available"], false, "行号越界必须如实报告: {}", result);
        assert!(result["reason"].as_str().unwrap().contains("行号越界"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_truncates_long_body() {
        let dir = std::env::temp_dir().join("cc_source_truncate_test");
        let _ = std::fs::create_dir_all(&dir);
        let body: String = (1..=SOURCE_MAX_LINES + 50).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(dir.join("long.rs"), body).unwrap();

        let registry = registry_with_root(&dir);
        let result = extract_source(&registry, &symbol_at("long.rs", 1, (SOURCE_MAX_LINES + 50) as u64));

        assert_eq!(result["available"], true);
        assert_eq!(result["truncated"], true, "超长符号体必须标记截断");
        assert_eq!(result["end_line"], SOURCE_MAX_LINES);
        assert_eq!(result["code"].as_str().unwrap().lines().count(), SOURCE_MAX_LINES);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_handles_end_line_past_eof_without_panic() {
        let dir = std::env::temp_dir().join("cc_source_eof_test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("partial.rs"), "a\nb\nc\n").unwrap();

        let registry = registry_with_root(&dir);
        // 起止都在文件内，但 end_line 超出末行 —— 应裁剪到末行并标记截断
        let result = extract_source(&registry, &symbol_at("partial.rs", 2, 999));

        assert_eq!(result["available"], true, "{}", result);
        assert_eq!(result["start_line"], 2);
        assert_eq!(result["end_line"], 3);
        assert_eq!(result["code"], "b\nc");
        assert_eq!(result["truncated"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_handles_end_line_before_start() {
        let dir = std::env::temp_dir().join("cc_source_reversed_test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("rev.rs"), "a\nb\nc\n").unwrap();

        let registry = registry_with_root(&dir);
        // end_line < line（脏索引数据）—— 必须退化为单行，不能 panic
        let result = extract_source(&registry, &symbol_at("rev.rs", 3, 1));

        assert_eq!(result["available"], true, "{}", result);
        assert_eq!(result["start_line"], 3);
        assert_eq!(result["end_line"], 3);
        assert_eq!(result["code"], "c");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_handles_empty_file() {
        let dir = std::env::temp_dir().join("cc_source_empty_test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("empty.rs"), "").unwrap();

        let registry = registry_with_root(&dir);
        let result = extract_source(&registry, &symbol_at("empty.rs", 1, 5));

        // 空文件属于「行号越界」，必须如实报告而不是回空代码
        assert_eq!(result["available"], false, "{}", result);
        assert!(result["reason"].as_str().unwrap().contains("行号越界"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_estimate_tokens_is_monotonic_and_nonzero() {
        assert!(estimate_tokens("") >= 1);
        assert!(estimate_tokens("abc") <= estimate_tokens("abcdef"));
    }

    #[test]
    fn test_extract_source_handles_zero_line_numbers() {
        let dir = std::env::temp_dir().join("cc_source_zeroline_test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("zero.rs"), "first\nsecond\n").unwrap();

        let registry = registry_with_root(&dir);
        // line=0 且 end_line=0：解析器若将来给出未赋值的行号会走到这里。
        // 此前 end_clamped(0) - start(1) 会 usize 下溢 —— debug panic，
        // release 下回绕成「available:true + 空 code」，即空串伪装成功
        let result = extract_source(&registry, &symbol_at("zero.rs", 0, 0));

        assert_eq!(result["available"], true, "{}", result);
        assert!(
            !result["code"].as_str().unwrap().is_empty(),
            "不得返回空源码伪装成功: {}",
            result
        );
        assert_eq!(result["code"], "first");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_caps_total_characters() {
        let dir = std::env::temp_dir().join("cc_source_charcap_test");
        let _ = std::fs::create_dir_all(&dir);
        // 单行超长（min.js / 生成代码的典型形态）：行数没超，字符数远超上限
        let long_line = "x".repeat(SOURCE_MAX_CHARS * 3);
        std::fs::write(dir.join("min.js"), format!("{}\n", long_line)).unwrap();

        let registry = registry_with_root(&dir);
        let result = extract_source(&registry, &symbol_at("min.js", 1, 1));

        assert_eq!(result["available"], true);
        assert_eq!(result["truncated"], true, "超长单行必须标记截断: {}", result);
        assert_eq!(result["truncated_reason"], "超出字符数上限");
        assert!(
            result["code"].as_str().unwrap().chars().count() <= SOURCE_MAX_CHARS + 1,
            "实际长度 {} 超过上限",
            result["code"].as_str().unwrap().chars().count()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_source_without_project_root() {
        let registry = ToolRegistry::new();
        let result = extract_source(&registry, &symbol_at("src/demo.rs", 1, 2));
        assert_eq!(result["available"], false);
        assert!(result["reason"].as_str().unwrap().contains("未配置项目根目录"));
    }
}
