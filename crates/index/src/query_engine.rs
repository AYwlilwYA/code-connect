//! 统一查询引擎
//!
//! 将上层查询请求路由到适当的存储后端（tantivy / sled），
//! 并组合结果。支持全文搜索、精确查找、分页。

use crate::sled_store::SledStore;
use crate::tantivy_index::{SymbolSearchResult, TantivyIndex};
use codeconnect_core::error::CodeConnectError;

/// 查询引擎 — 组合 tantivy 全文索引和 sled K/V 存储
///
/// 上层服务通过此结构体进行所有查询操作，
/// 无需关心底层存储实现的细节。
pub struct QueryEngine {
    /// 全文搜索索引（基于 tantivy BM25）
    pub tantivy: TantivyIndex,
    /// 键值持久化存储（基于 sled）
    pub sled: SledStore,
}

impl QueryEngine {
    /// 创建新的查询引擎（使用已存在的索引和存储实例）
    ///
    /// # 参数
    /// - `tantivy` — 已初始化的 tantivy 全文搜索索引
    /// - `sled` — 已打开的 sled K/V 数据库
    pub fn new(tantivy: TantivyIndex, sled: SledStore) -> Self {
        Self { tantivy, sled }
    }

    /// 按符号名搜索（全文 + 精确匹配）
    ///
    /// 当前通过 tantivy 的 `name` 字段进行全文搜索。
    /// 后续可扩展为组合 `language` 和 `kind` 过滤的分面查询。
    ///
    /// # 参数
    /// - `name` — 符号名（支持部分匹配和 fts 语法）
    /// - `_language` — 可选的编程语言过滤（暂未实现）
    /// - `_kind` — 可选的符号种类过滤（暂未实现）
    /// - `limit` — 最大返回结果数
    pub fn search_by_name(
        &self,
        name: &str,
        _language: Option<&str>,
        _kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, CodeConnectError> {
        self.tantivy.search_by_name(name, limit)
    }

    /// 按稳定 ID 获取符号的完整序列化数据
    ///
    /// # 参数
    /// - `stable_id` — 符号的稳定标识符
    ///
    /// # 返回值
    /// - `Some(Vec<u8>)` 表示符号存在，返回序列化字节（由调用方反序列化）
    /// - `None` 表示该 ID 对应的符号不存在
    pub fn get_symbol_by_id(&self, stable_id: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        self.sled.get_symbol(stable_id)
    }

    /// 获取文件内所有符号的 ID 列表（序列化字节）
    ///
    /// # 参数
    /// - `file_path` — 源文件的绝对路径
    ///
    /// # 返回值
    /// - `Some(Vec<u8>)` 表示该文件有已索引的符号
    /// - `None` 表示该文件尚未索引或无符号
    pub fn get_file_symbols(&self, file_path: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        self.sled.get_file_symbols(file_path)
    }

    /// 获取文件的元信息（如最后索引时间、指纹等）
    ///
    /// # 参数
    /// - `file_path` — 源文件的绝对路径
    pub fn get_file_meta(&self, file_path: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        self.sled.get_file_meta(file_path)
    }

    /// 获取索引中的符号总数
    ///
    /// 返回 tantivy 索引中的文档数（每个文档对应一个符号）。
    pub fn total_symbols(&self) -> Result<u64, CodeConnectError> {
        self.tantivy.doc_count()
    }
}
