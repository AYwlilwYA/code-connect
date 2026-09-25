//! JSON Schema 参数定义
//!
//! 基于 schemars 为 MCP 工具参数生成 JSON Schema，
//! 用于 rmcp 的类型化注册。
//!
//! 每个工具都有对应的请求参数结构体，派生 `schemars::JsonSchema`
//! 和 `serde::Deserialize`，以便 rmcp 自动生成并验证参数。

use schemars::JsonSchema;
use serde::Deserialize;

// ============================================================================
// search_symbol — 符号搜索
// ============================================================================

/// 符号搜索请求参数
///
/// 支持名称搜索（精确或模糊匹配）、类型过滤和语言过滤。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchSymbolParams {
    /// 搜索查询字符串（支持精确名称或模糊匹配）
    pub query: String,
    /// 符号类型过滤（如 function、class、method、enum、constant 等）
    ///
    /// `constant` 指枚举量（C/C++ enumerator、Rust enum variant、
    /// Java enum constant、C# enum member、TS enum 成员）。
    #[serde(default)]
    pub kind: Option<String>,
    /// 编程语言过滤（如 rust、typescript、java 等）
    #[serde(default)]
    pub language: Option<String>,
    /// 最大返回结果数，默认 20
    #[serde(default = "default_limit_20")]
    pub limit: usize,
    /// 返回详细程度：`brief`（默认，只回定位必需字段）/ `full`（回完整符号信息含文档注释）
    ///
    /// 搜索用于定位，详情应由 get_symbol 按需获取，
    /// 因此默认 brief 以避免单次调用吃掉大量上下文。
    #[serde(default = "default_detail_brief")]
    pub detail: String,
    /// 是否在已索引文件内做文本检索并回显文本真值，默认 true
    ///
    /// 符号索引覆盖不全时（枚举量、限定名调用、宏等），
    /// 「索引里没有」与「代码里没有」在返回值上同形 —— 关掉它会失去这层保护。
    /// 仅在对响应体积/耗时敏感时关掉。
    #[serde(default = "default_true")]
    pub text_truth: bool,
}

fn default_limit_20() -> usize {
    20
}

fn default_detail_brief() -> String {
    "brief".to_string()
}

// ============================================================================
// get_symbol — 获取符号详情
// ============================================================================

/// 获取符号详情请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetSymbolParams {
    /// 稳定符号 ID，或符号名称（内部自动解析）
    pub symbol_id: String,
    /// 是否随符号一并返回源码片段，默认 true
    ///
    /// 索引中已记录符号的行区间，可直接切片返回，
    /// 避免为「看一眼这个函数怎么写的」而 read 整个文件。
    #[serde(default = "default_true")]
    pub include_source: bool,
}

// ============================================================================
// trace_callers — 追溯调用者
// ============================================================================

/// 追溯调用者请求参数
///
/// 反向遍历调用图，找出所有调用（或间接触发）目标符号的上游符号。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TraceCallersParams {
    /// 目标符号 ID
    pub symbol_id: String,
    /// 最大追溯深度，默认 3
    #[serde(default = "default_depth_3")]
    pub max_depth: usize,
    /// 是否包含间接调用者，默认 true
    #[serde(default = "default_true")]
    pub include_indirect: bool,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

fn default_depth_3() -> usize {
    3
}

fn default_result_limit() -> usize {
    50
}

fn default_true() -> bool {
    true
}

// ============================================================================
// trace_callees — 追溯被调用者
// ============================================================================

/// 追溯被调用者请求参数
///
/// 正向遍历调用图，找出目标符号调用了哪些下游符号。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TraceCalleesParams {
    /// 源符号 ID
    pub symbol_id: String,
    /// 最大追溯深度，默认 3
    #[serde(default = "default_depth_3")]
    pub max_depth: usize,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

// ============================================================================
// analyze_impact — 变更影响分析
// ============================================================================

/// 变更影响分析请求参数
///
/// 基于调用图 BFS 遍历评估修改指定符号的潜在影响范围。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AnalyzeImpactParams {
    /// 变更的符号 ID 列表
    pub symbol_ids: Vec<String>,
    /// BFS 遍历最大深度，默认 5
    #[serde(default = "default_impact_depth")]
    pub max_depth: usize,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

fn default_impact_depth() -> usize {
    5
}

// ============================================================================
// get_call_graph — 获取调用子图
// ============================================================================

/// 获取调用子图请求参数
///
/// 以指定符号为中心，提取其上游调用者和下游被调用者的局部调用图。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetCallGraphParams {
    /// 中心符号 ID
    pub symbol_id: String,
    /// 调用者方向深度，默认 2
    #[serde(default = "default_graph_depth_2")]
    pub caller_depth: usize,
    /// 被调用者方向深度，默认 2
    #[serde(default = "default_graph_depth_2")]
    pub callee_depth: usize,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

fn default_graph_depth_2() -> usize {
    2
}

// ============================================================================
// get_metrics — 获取代码质量指标
// ============================================================================

/// 获取代码质量指标请求参数
///
/// 可针对单个符号或整个文件计算圈复杂度、出入度等指标。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetMetricsParams {
    /// 目标符号 ID（可选，不提供则获取文件级/项目级指标）
    #[serde(default)]
    pub symbol_id: Option<String>,
    /// 目标文件路径（可选）
    #[serde(default)]
    pub file_path: Option<String>,
    /// 指标类型过滤（如 complexity、fan_in、fan_out）
    #[serde(default)]
    pub metric_types: Option<Vec<String>>,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

// ============================================================================
// detect_dead_code — 死代码检测
// ============================================================================

/// 死代码检测请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DetectDeadCodeParams {
    /// 入口点符号名称列表（如 main、pub 函数）
    #[serde(default)]
    pub entry_points: Option<Vec<String>>,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

// ============================================================================
// check_arch_rules — 架构规则验证
// ============================================================================

/// 架构规则验证请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CheckArchRulesParams {
    /// 指定要检查的规则名称列表（可选，默认检查全部）
    #[serde(default)]
    pub rule_names: Option<Vec<String>>,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

// ============================================================================
// semantic_search — 语义搜索
// ============================================================================

/// 语义搜索请求参数
///
/// 基于自然语言描述在符号名称、签名和文档注释中进行搜索。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SemanticSearchParams {
    /// 自然语言搜索描述
    pub description: String,
    /// 最大返回结果数，默认 10
    #[serde(default = "default_limit_10")]
    pub limit: usize,
    /// 编程语言过滤（可选）
    #[serde(default)]
    pub language: Option<String>,
}

fn default_limit_10() -> usize {
    10
}

// ============================================================================
// find_references — 查找引用
// ============================================================================

/// 查找引用请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FindReferencesParams {
    /// 目标符号 ID
    pub symbol_id: String,
    /// 最大返回结果数，默认 50
    #[serde(default = "default_limit_50")]
    pub limit: usize,
    /// 是否在已索引文件内做文本检索并回显文本真值，默认 true
    ///
    /// 调用边覆盖不全时（限定名调用、宏、间接调用等），
    /// 「没有调用方」与「调用边没建」在返回值上同形，
    /// 据此判定「改签名很安全」已造成过真实事故 —— 关掉它会失去这层保护。
    #[serde(default = "default_true")]
    pub text_truth: bool,
}

fn default_limit_50() -> usize {
    50
}

// ============================================================================
// reindex — 重新索引
// ============================================================================

/// 重新索引请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReindexParams {
    /// 指定要重新索引的文件路径列表（可选，不提供则全量重建）
    #[serde(default)]
    pub file_paths: Option<Vec<String>>,
    /// 是否全量重建索引（默认 false，即增量更新）
    #[serde(default)]
    pub full: bool,
}

// ============================================================================
// get_index_status — 获取索引状态
// ============================================================================

/// 获取索引状态请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetIndexStatusParams {
    /// 是否显示详细信息（各语言统计、磁盘占用等）
    #[serde(default)]
    pub verbose: bool,
}

// ============================================================================
// list_files — 列出已索引文件
// ============================================================================

/// 列出已索引文件请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListFilesParams {
    /// 编程语言过滤（可选，如 rust、java、typescript）
    #[serde(default)]
    pub language: Option<String>,
    /// 最大返回数，默认 100
    #[serde(default = "default_limit_100")]
    pub limit: usize,
    /// 分页偏移量
    #[serde(default)]
    pub offset: usize,
}

fn default_limit_100() -> usize {
    100
}

// ============================================================================
// get_type_hierarchy — 类型继承链
// ============================================================================

/// 获取类型继承链请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetTypeHierarchyParams {
    /// 类型符号 ID
    pub symbol_id: String,
    /// 查询方向：ancestors（父类链）| descendants（子类链）| both（双向）
    #[serde(default = "default_direction")]
    pub direction: String,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

fn default_direction() -> String {
    "both".to_string()
}

// ============================================================================
// get_file_symbols — 获取文件内符号列表
// ============================================================================

/// 获取文件内所有符号请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetFileSymbolsParams {
    /// 文件路径（相对于项目根目录）
    pub file_path: String,
    /// 是否随每个符号返回源码片段，默认 false
    ///
    /// 一个文件可能包含大量符号，默认关闭以免一次调用吃掉大量上下文；
    /// 需要逐段查看时再显式开启。
    #[serde(default)]
    pub include_source: bool,
}

// ============================================================================
// get_project_map — 项目语义快照
// ============================================================================

/// 项目地图请求参数
///
/// 用于在上下文丢失（如对话压缩）后，一次性重建对项目的整体认知，
/// 替代逐个文件重读。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetProjectMapParams {
    /// 本次**响应**的目标 token 预算，默认 6000
    ///
    /// 超出预算时会自动逐级降级（省略签名 → 只留符号名 → 只留统计），
    /// 并在响应中明确标注已降级，不会静默截断。
    /// **只约束响应**：落盘的 PROJECT_MAP.md 始终是全量，
    /// 需要细节时直接读取该文件即可。
    #[serde(default = "default_budget_tokens")]
    pub budget_tokens: usize,
    /// 只关注某个子目录（相对项目根目录的路径前缀，如 `crates/index`）
    #[serde(default)]
    pub focus: Option<String>,
    /// 是否把地图写入 `<数据目录>/PROJECT_MAP.md`，默认 true
    ///
    /// 落盘是为了让它能被项目 CLAUDE.md 引用，
    /// 从而像 MEMORY.md 一样在每次会话自动加载 —— 这是唯一能让
    /// 语义自动挺过对话压缩的机制。
    #[serde(default = "default_true")]
    pub write_file: bool,
}

fn default_budget_tokens() -> usize {
    6000
}

// ============================================================================
// get_dependency_graph — 获取依赖图
// ============================================================================

/// 获取依赖图请求参数
///
/// 返回文件级别或模块级别的依赖关系图。
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetDependencyGraphParams {
    /// 依赖图层级：file | module | symbol
    #[serde(default = "default_dep_level")]
    pub level: String,
    /// 指定文件路径（可选，不提供则返回完整图）
    #[serde(default)]
    pub file_path: Option<String>,
    /// 最大返回结果数，默认 50（硬上限 200）
    ///
    /// 超出时会截断并在响应中给出真实总数与告警，不静默丢弃。
    #[serde(default = "default_result_limit")]
    pub limit: usize,
}

fn default_dep_level() -> String {
    "file".to_string()
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_symbol_params_defaults() {
        let params = SearchSymbolParams {
            query: "main".to_string(),
            kind: None,
            language: None,
            limit: default_limit_20(),
            detail: "brief".to_string(),
            text_truth: default_true(),
        };
        assert_eq!(params.query, "main");
        assert_eq!(params.limit, 20);
        assert!(params.kind.is_none());
        assert!(params.text_truth, "文本真值回显默认开启");
    }

    #[test]
    fn test_trace_callers_params_defaults() {
        let params = TraceCallersParams {
            symbol_id: "test_id".to_string(),
            max_depth: default_depth_3(),
            include_indirect: true,
            limit: default_result_limit(),
        };
        assert_eq!(params.max_depth, 3);
        assert!(params.include_indirect);
        assert_eq!(params.limit, 50);
    }

    #[test]
    fn test_analyze_impact_defaults() {
        let params = AnalyzeImpactParams {
            symbol_ids: vec!["s1".to_string()],
            max_depth: 5,
            limit: default_result_limit(),
        };
        assert_eq!(params.max_depth, 5);
        assert_eq!(params.symbol_ids.len(), 1);
        assert_eq!(params.limit, 50);
    }
}
