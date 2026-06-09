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
    /// 符号类型过滤（如 function、class、method 等）
    #[serde(default)]
    pub kind: Option<String>,
    /// 编程语言过滤（如 rust、typescript、java 等）
    #[serde(default)]
    pub language: Option<String>,
    /// 最大返回结果数，默认 20
    #[serde(default = "default_limit_20")]
    pub limit: usize,
}

fn default_limit_20() -> usize {
    20
}

// ============================================================================
// get_symbol — 获取符号详情
// ============================================================================

/// 获取符号详情请求参数
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetSymbolParams {
    /// 稳定符号 ID
    pub symbol_id: String,
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
}

fn default_depth_3() -> usize {
    3
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
        };
        assert_eq!(params.query, "main");
        assert_eq!(params.limit, 20);
        assert!(params.kind.is_none());
    }

    #[test]
    fn test_trace_callers_params_defaults() {
        let params = TraceCallersParams {
            symbol_id: "test_id".to_string(),
            max_depth: default_depth_3(),
            include_indirect: true,
        };
        assert_eq!(params.max_depth, 3);
        assert!(params.include_indirect);
    }

    #[test]
    fn test_analyze_impact_defaults() {
        let params = AnalyzeImpactParams {
            symbol_ids: vec!["s1".to_string()],
            max_depth: 5,
        };
        assert_eq!(params.max_depth, 5);
        assert_eq!(params.symbol_ids.len(), 1);
    }
}
