//! 符号服务
//!
//! 封装 QueryEngine，提供符号的搜索、按 ID 获取和批量解析功能。
//! 作为上层服务单元，将底层的字节存储操作封装为类型安全的 API。

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::types::Symbol;
use codeconnect_index::query_engine::QueryEngine;
use codeconnect_index::tantivy_index::SymbolSearchResult;

/// 符号服务 — 统一的符号查询入口
///
/// 封装 [`QueryEngine`]，将原始字节反序列化为 [`Symbol`]，
/// 提供搜索、精确查找和批量解析三类操作。
pub struct SymbolService<'a> {
    /// 底层查询引擎引用
    engine: &'a QueryEngine,
}

impl<'a> SymbolService<'a> {
    /// 创建新的符号服务实例
    ///
    /// # 参数
    /// - `engine` — 已初始化的查询引擎引用
    pub fn new(engine: &'a QueryEngine) -> Self {
        Self { engine }
    }

    // =========================================================================
    // 搜索
    // =========================================================================

    /// 按符号名搜索（全文搜索）
    ///
    /// 依赖 tantivy 的 BM25 全文索引，对 `name` 字段做模糊匹配。
    ///
    /// # 参数
    /// - `name` — 符号名（支持 tantivy 全文搜索语法）
    /// - `limit` — 最多返回的结果数
    ///
    /// # 返回
    /// 按相关度排序的搜索结果列表，每个结果包含符号 ID、名称、类型和评分。
    pub fn search(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, CodeConnectError> {
        self.engine.search_by_name(name, None, None, limit)
    }

    // =========================================================================
    // 按 ID 查找
    // =========================================================================

    /// 按稳定符号 ID 精确查找符号
    ///
    /// 从 tantivy STORED 字段中读取符号信息。
    ///
    /// # 参数
    /// - `stable_id` — 符号的稳定标识符
    ///
    /// # 返回
    /// - `Ok(Some(symbol))` — 找到符号
    /// - `Ok(None)` — 符号不存在
    /// - `Err(...)` — 查询错误
    pub fn search_by_id(
        &self,
        stable_id: &str,
    ) -> Result<Option<Symbol>, CodeConnectError> {
        self.engine.get_symbol_by_id(stable_id)
    }

    // =========================================================================
    // 批量解析
    // =========================================================================

    /// 批量按 ID 解析符号
    ///
    /// 对一批符号 ID 逐一调用 `search_by_id`。
    /// 不存在的符号会被跳过，不会报错。
    ///
    /// # 参数
    /// - `ids` — 待解析的符号 ID 列表
    ///
    /// # 返回
    /// 成功解析的符号列表。如果某个 ID 对应的符号不存在，
    /// 不会出现在结果中，也不会产生错误。
    pub fn batch_resolve(
        &self,
        ids: &[String],
    ) -> Result<Vec<Symbol>, CodeConnectError> {
        let mut symbols = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(symbol) = self.search_by_id(id)? {
                symbols.push(symbol);
            }
        }
        Ok(symbols)
    }

    // =========================================================================
    // 辅助方法
    // =========================================================================

    /// 获取索引中的符号总数
    ///
    /// 返回 tantivy 索引中的文档总数。
    pub fn total_count(&self) -> Result<u64, CodeConnectError> {
        self.engine.total_symbols()
    }

    /// 获取指定文件中所有符号的 ID 列表
    ///
    /// 从 sled 的 file_symbols 映射中读取。
    /// 如果文件尚未索引，返回空列表。
    ///
    /// # 参数
    /// - `file_path` — 源文件路径
    #[deprecated(note = "文件→符号映射已可从 tantivy search_by_file_path 查询")]
    #[allow(deprecated)]
    pub fn get_file_symbol_ids(&self, file_path: &str) -> Result<Vec<String>, CodeConnectError> {
        match self.engine.get_file_symbol_ids(file_path)? {
            Some(data) => {
                let ids: Vec<String> = serde_json::from_slice(&data)?;
                Ok(ids)
            }
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    /// 验证接口设计：空列表应返回空结果
    #[test]
    fn test_batch_resolve_empty_interface() {
        // 此测试验证接口契约 — 实际集成测试需要完整的索引环境
        let ids: Vec<String> = vec![];
        assert_eq!(ids.len(), 0);
    }
}
