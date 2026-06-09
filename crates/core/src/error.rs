//! 分层错误类型定义
//!
//! 使用 thiserror 派生宏定义统一的错误类型 [`CodeConnectError`]，
//! 覆盖 IO、解析、索引、查询、配置、符号查找等各层错误场景。

use std::path::PathBuf;
use thiserror::Error;

/// CodeConnect 统一错误类型
///
/// 按功能层划分变体，每个变体携带足够的上下文信息用于
/// 日志记录和用户友好的错误消息展示。
#[derive(Error, Debug)]
pub enum CodeConnectError {
    /// IO 操作错误（文件读写、网络等）
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    /// 源码解析错误
    #[error("解析错误 {file}: {message}")]
    Parse {
        /// 解析出错的源文件路径
        file: PathBuf,
        /// 错误详情
        message: String,
    },

    /// 索引操作错误
    #[error("索引错误: {0}")]
    Index(String),

    /// 查询操作错误
    #[error("查询错误: {0}")]
    Query(String),

    /// 配置错误（文件格式、缺失字段等）
    #[error("配置错误: {0}")]
    Config(String),

    /// 不支持的语言类型
    #[error("不支持的语言: {0}")]
    UnsupportedLanguage(String),

    /// 符号未找到
    #[error("符号未找到: {0}")]
    SymbolNotFound(String),

    /// JSON 序列化/反序列化错误
    #[error("序列化错误: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// 便捷类型别名
pub type Result<T> = std::result::Result<T, CodeConnectError>;
