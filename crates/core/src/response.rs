//! 统一 MCP 响应信封
//!
//! 提供标准化的请求-响应封装 [`McpResponse`]，包含请求 ID、
//! 状态码、警告信息、负载数据和元信息。
//!
//! 同时提供 [`PageToken`] 用于分页查询。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// McpResponse
// ============================================================================

/// 统一 MCP 响应信封
///
/// 所有 MCP 协议交互均使用此结构包装，确保前端能统一处理
/// 成功、部分成功和错误状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse<T: Serialize> {
    /// 请求 ID（UUID v4）
    pub request_id: String,

    /// 响应状态
    pub status: ResponseStatus,

    /// 警告信息列表（非致命问题）
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,

    /// 响应数据（成功时有值）
    pub data: Option<T>,

    /// 响应元信息
    pub meta: ResponseMeta,
}

/// 响应状态枚举
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseStatus {
    /// 成功 — 所有请求项均正常返回
    Success,
    /// 部分成功 — 部分结果返回，伴有警告
    Partial,
    /// 错误 — 请求处理失败
    Error,
}

/// 响应元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseMeta {
    /// 请求处理耗时（毫秒）
    pub took_ms: u64,

    /// 总结果数（服务器端匹配总数）
    pub total_results: usize,

    /// 本次返回的结果数
    pub returned_results: usize,

    /// 下一页游标 token（无下一页时为 None）
    pub next_page_token: Option<String>,

    /// 索引陈旧度（毫秒），表示索引最后更新时间距今多久
    pub index_staleness_ms: Option<u64>,
}

impl<T: Serialize> McpResponse<T> {
    /// 创建成功响应
    ///
    /// # 参数
    /// - `data` — 响应负载数据
    /// - `total` — 服务器端匹配总数
    /// - `returned` — 本次实际返回数
    /// - `took_ms` — 处理耗时（毫秒）
    pub fn success(data: T, total: usize, returned: usize, took_ms: u64) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            status: ResponseStatus::Success,
            warnings: vec![],
            data: Some(data),
            meta: ResponseMeta {
                took_ms,
                total_results: total,
                returned_results: returned,
                next_page_token: None,
                index_staleness_ms: None,
            },
        }
    }

    /// 创建错误响应
    ///
    /// # 参数
    /// - `message` — 错误描述，会放入 warnings 字段
    pub fn error(message: &str) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            status: ResponseStatus::Error,
            warnings: vec![message.to_string()],
            data: None,
            meta: ResponseMeta {
                took_ms: 0,
                total_results: 0,
                returned_results: 0,
                next_page_token: None,
                index_staleness_ms: None,
            },
        }
    }

    /// 添加警告信息
    ///
    /// 添加一条警告后自动将状态设置为 Partial。
    ///
    /// # 参数
    /// - `warning` — 警告信息
    pub fn with_warning(mut self, warning: String) -> Self {
        self.warnings.push(warning);
        self.status = ResponseStatus::Partial;
        self
    }

    /// 设置下一页游标 token
    ///
    /// # 参数
    /// - `token` — 分页游标字符串
    pub fn with_page_token(mut self, token: String) -> Self {
        self.meta.next_page_token = Some(token);
        self
    }

    /// 设置索引陈旧度
    ///
    /// # 参数
    /// - `ms` — 索引自上次更新以来的毫秒数
    pub fn with_staleness(mut self, ms: u64) -> Self {
        self.meta.index_staleness_ms = Some(ms);
        self
    }
}

// ============================================================================
// PageToken
// ============================================================================

/// 分页游标
///
/// 用于在多次请求间维持查询上下文，支持大数据集的分页遍历。
/// 通过 base64 编码的 JSON 序列化实现不透明传输。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageToken {
    /// 查询内容的哈希值（用于校验查询一致性）
    pub query_hash: String,

    /// 当前偏移量
    pub offset: usize,
}

impl PageToken {
    /// 创建新的分页游标
    ///
    /// # 参数
    /// - `query_hash` — 查询哈希
    /// - `offset` — 偏移量
    pub fn new(query_hash: String, offset: usize) -> Self {
        Self {
            query_hash,
            offset,
        }
    }

    /// 序列化为传输字符串（base64 JSON）
    pub fn encode(&self) -> Result<String, serde_json::Error> {
        let json = serde_json::to_string(self)?;
        Ok(json)
    }

    /// 从传输字符串反序列化
    pub fn decode(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success_response() {
        let response = McpResponse::success("hello", 1, 1, 42);
        assert_eq!(response.status, ResponseStatus::Success);
        assert_eq!(response.data, Some("hello"));
        assert!(response.warnings.is_empty());
        assert_eq!(response.meta.took_ms, 42);
    }

    #[test]
    fn test_error_response() {
        let response = McpResponse::<()>::error("something went wrong");
        assert_eq!(response.status, ResponseStatus::Error);
        assert!(response.data.is_none());
        assert_eq!(response.warnings, vec!["something went wrong"]);
    }

    #[test]
    fn test_with_warning() {
        let response = McpResponse::success("data", 1, 1, 10)
            .with_warning("stale index".into());
        assert_eq!(response.status, ResponseStatus::Partial);
        assert_eq!(response.warnings, vec!["stale index"]);
    }

    #[test]
    fn test_page_token_roundtrip() {
        let token = PageToken::new("abc123".into(), 50);
        let encoded = token.encode().unwrap();
        let decoded = PageToken::decode(&encoded).unwrap();
        assert_eq!(decoded.query_hash, "abc123");
        assert_eq!(decoded.offset, 50);
    }
}
