//! MCP 服务器创建与启动
//!
//! 基于 rmcp 实现 MCP 服务器，使用 stdio 传输方式。
//!
//! # 架构
//!
//! 使用 `CodeConnectServer` 结构体，通过以下三个宏协同工作：
//! - `#[tool_router]` — 在工具方法 impl 块上生成 `tool_router()` 函数
//! - `#[tool]` — 标记每个工具方法，声明名称、描述和参数 schema
//! - `#[tool_handler]` — 在 `ServerHandler` impl 块上自动生成 `call_tool` 和 `list_tools`
//!
//! 启动流程：
//! 1. 创建 `CodeConnectServer` 实例（持有索引和 tool_router）
//! 2. 通过 `serve_server` 启动 stdio MCP 服务

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{
    CallToolResult, Implementation, InitializeResult, ProtocolVersion, RawContent,
    RawTextContent, ServerCapabilities,
};
use rmcp::service::serve_server;

use crate::schemas::*;
use crate::tools::{self, ToolRegistry};

// 导入宏，使得编译时可以找到
// （rmcp 通过 proc-macro 自动导入，这里不需要显式 use）

/// CodeConnect MCP 服务器
///
/// 实现 `ServerHandler` trait，提供所有 MCP 工具注册和 handler 逻辑。
/// 持有对索引、存储和服务的共享引用，以及 rmcp 的 tool_router。
#[derive(Clone)]
pub struct CodeConnectServer {
    /// 工具注册表
    registry: Arc<ToolRegistry>,
    /// 服务器信息
    server_info: InitializeResult,
    /// rmcp 工具路由器（由 #[tool_router] 生成）
    tool_router: ToolRouter<Self>,
}

impl CodeConnectServer {
    /// 创建新的 MCP 服务器实例
    ///
    /// # 参数
    ///
    /// - `registry` — 工具注册表，包含 sled、tantivy 等服务后端
    pub fn new(registry: ToolRegistry) -> Self {
        let server_info = InitializeResult {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "codeconnect".into(),
                title: Some("CodeConnect — 多语言代码分析 MCP 服务器".into()),
                version: env!("CARGO_PKG_VERSION").into(),
                description: Some(
                    "提供符号搜索、调用分析、影响分析、死代码检测、代码质量指标等代码智能分析能力"
                        .into(),
                ),
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "使用工具名称查询具体的分析能力。常用: search_symbol, trace_callers, analyze_impact"
                    .into(),
            ),
        };
        Self {
            registry: Arc::new(registry),
            server_info,
            tool_router: Self::tool_router(),
        }
    }

    /// 启动 MCP stdio 服务器
    ///
    /// 使用 tokio::io::stdin/stdout 作为传输通道。
    pub async fn start_stdio(self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let transport = (tokio::io::stdin(), tokio::io::stdout());
        serve_server(self, transport).await?;
        Ok(())
    }
}

// ============================================================================
// MCP 工具方法 — 使用 #[tool_router] + #[tool] 宏
// ============================================================================

#[rmcp::tool_router]
impl CodeConnectServer {
    /// 符号搜索 — 按名称、类型和语言搜索代码符号
    #[rmcp::tool(
        description = "按名称、类型和语言搜索代码符号，返回匹配的符号及其位置、签名、文档等完整信息"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn search_symbol(
        &self,
        Parameters(params): Parameters<SearchSymbolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_search_symbol(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 获取符号详情 — 按符号 ID 获取完整信息
    #[rmcp::tool(
        description = "按符号 ID 获取符号的完整信息，包括位置、签名、文档注释、修饰符等"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn get_symbol(
        &self,
        Parameters(params): Parameters<GetSymbolParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_get_symbol(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 追溯调用者 — 反向遍历调用链找出所有上游调用者
    #[rmcp::tool(
        description = "反向遍历调用图，找出目标符号的所有上游调用者及其完整调用链"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn trace_callers(
        &self,
        Parameters(params): Parameters<TraceCallersParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_trace_callers(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 追溯被调用者 — 正向遍历调用链找出所有下游被调用者
    #[rmcp::tool(description = "正向遍历调用图，找出目标符号调用的所有下游符号")]
    #[allow(clippy::needless_pass_by_value)]
    async fn trace_callees(
        &self,
        Parameters(params): Parameters<TraceCalleesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_trace_callees(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 变更影响分析 — 基于调用图评估修改符号的影响范围
    #[rmcp::tool(
        description = "基于 BFS 调用链传播算法，评估修改指定符号后的潜在影响范围，按严重度分类输出影响报告"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn analyze_impact(
        &self,
        Parameters(params): Parameters<AnalyzeImpactParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_analyze_impact(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 获取调用子图 — 获取指定符号周围的局部调用图
    #[rmcp::tool(
        description = "以指定符号为中心，提取上游调用者和下游被调用者组成的局部调用关系子图"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn get_call_graph(
        &self,
        Parameters(params): Parameters<GetCallGraphParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_get_call_graph(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 获取代码质量指标 — 圈复杂度、扇入扇出、继承深度
    #[rmcp::tool(
        description = "获取指定符号或文件的代码质量指标：圈复杂度、扇入、扇出、继承深度等"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn get_metrics(
        &self,
        Parameters(params): Parameters<GetMetricsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_get_metrics(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 死代码检测 — 检测从入口点不可达的代码
    #[rmcp::tool(
        description = "从入口点出发遍历调用图，标记未被引用到的死代码符号，附带置信度评估"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn detect_dead_code(
        &self,
        Parameters(params): Parameters<DetectDeadCodeParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_detect_dead_code(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 架构规则验证 — 验证自定义架构规则
    #[rmcp::tool(description = "验证指定的架构规则，检测层间依赖违规和层次隔离破坏")]
    #[allow(clippy::needless_pass_by_value)]
    async fn check_arch_rules(
        &self,
        Parameters(params): Parameters<CheckArchRulesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_check_arch_rules(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 语义搜索 — 基于自然语言描述搜索符号
    #[rmcp::tool(
        description = "基于自然语言描述在符号名称、签名和文档注释中进行语义匹配搜索"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn semantic_search(
        &self,
        Parameters(params): Parameters<SemanticSearchParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_semantic_search(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 查找引用 — 找出所有引用指定符号的位置
    #[rmcp::tool(
        description = "查找代码中所有引用指定符号的位置，包括调用引用和数据依赖引用"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn find_references(
        &self,
        Parameters(params): Parameters<FindReferencesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_find_references(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 重新索引 — 触发增量或全量索引重建
    #[rmcp::tool(
        description = "触发索引重建：可指定文件列表进行增量更新，或不指定进行全量重建"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn reindex(
        &self,
        Parameters(params): Parameters<ReindexParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_reindex(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 获取索引状态 — 查看索引的整体状态和统计信息
    #[rmcp::tool(
        description = "查看索引的整体状态：已索引文档数、存储条目数、Schema版本等信息"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn get_index_status(
        &self,
        Parameters(params): Parameters<GetIndexStatusParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_get_index_status(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 列出已索引文件 — 按语言过滤分页列出已索引的文件
    #[rmcp::tool(
        description = "按语言过滤、分页列出当前项目中已索引的所有源文件及其元信息"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn list_files(
        &self,
        Parameters(params): Parameters<ListFilesParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_list_files(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 类型继承链 — 获取类型的祖先链和子类链
    #[rmcp::tool(
        description = "获取指定类型的完整继承链：父类（祖先）和子类（后代）双向查询"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn get_type_hierarchy(
        &self,
        Parameters(params): Parameters<GetTypeHierarchyParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_get_type_hierarchy(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 文件符号列表 — 获取指定文件中的所有符号
    #[rmcp::tool(
        description = "获取指定源文件中提取的所有符号及其位置、类型和签名信息"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn get_file_symbols(
        &self,
        Parameters(params): Parameters<GetFileSymbolsParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_get_file_symbols(&self.registry, params);
        response_to_call_tool_result(response)
    }

    /// 获取依赖图 — 获取文件/模块/符号级别的依赖关系图
    #[rmcp::tool(
        description = "获取指定层级的项目依赖关系图：文件级别、模块级别或符号级别"
    )]
    #[allow(clippy::needless_pass_by_value)]
    async fn get_dependency_graph(
        &self,
        Parameters(params): Parameters<GetDependencyGraphParams>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        let response = tools::handle_get_dependency_graph(&self.registry, params);
        response_to_call_tool_result(response)
    }
}

// ============================================================================
// ServerHandler 实现 — 使用 #[tool_handler] 自动生成 call_tool/list_tools
// ============================================================================

#[rmcp::tool_handler]
impl ServerHandler for CodeConnectServer {
    /// 返回服务器能力信息
    fn get_info(&self) -> InitializeResult {
        self.server_info.clone()
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 将 McpResponse 转换为 rmcp 的 CallToolResult
///
/// 成功和部分成功的响应序列化为 JSON 文本内容，
/// 错误响应序列化为错误信息文本。
fn response_to_call_tool_result<T: serde::Serialize>(
    response: codeconnect_core::response::McpResponse<T>,
) -> Result<CallToolResult, rmcp::ErrorData> {
    match response.status {
        codeconnect_core::response::ResponseStatus::Success
        | codeconnect_core::response::ResponseStatus::Partial => {
            let json_text = serde_json::to_string_pretty(&response)
                .map_err(|e| rmcp::ErrorData::internal_error(format!("序列化响应失败: {}", e), None))?;

            let content = RawContent::Text(RawTextContent {
                text: json_text,
                meta: None,
            });

            // 将 RawContent 包装为带标注的内容
            let annotated = rmcp::model::Annotated {
                raw: content,
                annotations: None,
            };

            Ok(CallToolResult {
                content: vec![annotated],
                is_error: None,
                meta: None,
                structured_content: None,
            })
        }
        codeconnect_core::response::ResponseStatus::Error => {
            let error_msg = response.warnings.join("; ");
            Err(rmcp::ErrorData::internal_error(error_msg, None))
        }
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_creation() {
        let registry = ToolRegistry::new();
        let server = CodeConnectServer::new(registry);
        // 验证 server 可以创建
        let info = ServerHandler::get_info(&server);
        assert_eq!(info.server_info.name, "codeconnect");
    }

    #[test]
    fn test_response_to_call_tool_result_success() {
        let resp: codeconnect_core::response::McpResponse<String> =
            codeconnect_core::response::McpResponse::success("test data".to_string(), 1, 1, 10);
        let result = response_to_call_tool_result(resp);
        assert!(result.is_ok());
        let call_result = result.unwrap();
        assert!(!call_result.content.is_empty());
    }

    #[test]
    fn test_response_to_call_tool_result_error() {
        let resp: codeconnect_core::response::McpResponse<()> =
            codeconnect_core::response::McpResponse::error("something broke");
        let result = response_to_call_tool_result(resp);
        assert!(result.is_err());
    }
}
