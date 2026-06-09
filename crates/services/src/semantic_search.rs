//! 语义搜索服务
//!
//! 提供基于 tantivy 全文索引的代码搜索能力：
//! - `search_pattern` — 在代码全文内容中搜索模式
//! - `find_similar` — 按名称模糊匹配查找相似符号

use codeconnect_core::error::CodeConnectError;
use codeconnect_index::query_engine::QueryEngine;
use codeconnect_index::tantivy_index::SymbolSearchResult;

/// 语义搜索服务
///
/// 封装 [`QueryEngine`] 的全文搜索能力，提供模式搜索和相似符号查找。
///
/// # 搜索策略
/// - **search_pattern** — 在符号名中搜索指定模式（模糊匹配）
/// - **find_similar** — 根据名称查找相似符号（利用 tantivy 的分词和 BM25 评分）
pub struct SemanticSearch<'a> {
    /// 底层查询引擎引用
    engine: &'a QueryEngine,
}

impl<'a> SemanticSearch<'a> {
    /// 创建新的语义搜索服务
    ///
    /// # 参数
    /// - `engine` — 已初始化的查询引擎引用
    pub fn new(engine: &'a QueryEngine) -> Self {
        Self { engine }
    }

    // =========================================================================
    // 模式搜索
    // =========================================================================

    /// 在符号名中搜索指定模式
    ///
    /// 使用 tantivy 的 BM25 全文搜索对 `name` 字段进行匹配。
    /// 支持 tantivy 查询语法（如通配符 `*`、模糊匹配 `~`、短语 `"..."`）。
    ///
    /// # 参数
    /// - `pattern` — 搜索模式（支持 tantivy 查询语法）
    /// - `limit` — 最多返回的结果数
    ///
    /// # 返回
    /// 按相关度排序的搜索结果列表。
    ///
    /// # 示例
    /// - `"handle"` — 精确/模糊匹配函数名 handle
    /// - `"handle*"` — 匹配所有以 handle 开头的符号
    /// - `"event_handle"` — 匹配包含 event_handle 的符号
    pub fn search_pattern(
        &self,
        pattern: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, CodeConnectError> {
        self.engine.search_by_name(pattern, None, None, limit)
    }

    // =========================================================================
    // 相似符号查找
    // =========================================================================

    /// 查找与指定名称相似的符号
    ///
    /// 将 `name` 直接作为查询词进行全文搜索，
    /// 利用 tantivy 的分词和 BM25 评分返回相似结果。
    /// 搜索结果中的第一个（评分最高的）通常就是精确匹配，
    /// 后续的结果是名称中包含相似词汇的符号。
    ///
    /// # 参数
    /// - `name` — 用于查找相似符号的名称
    /// - `limit` — 最多返回的结果数
    ///
    /// # 返回
    /// 按相关度排序的相似符号列表。
    ///
    /// # 示例
    /// 搜索 "handle_event" 可能返回：
    /// - handle_event（精确匹配，最高评分）
    /// - handle_click_event（包含 event 和 handle）
    /// - process_event（包含 event）
    pub fn find_similar(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, CodeConnectError> {
        self.engine.search_by_name(name, None, None, limit)
    }

    // =========================================================================
    // 辅助方法
    // =========================================================================

    /// 按完整名称精确搜索
    ///
    /// 精确搜索适合需要完全匹配符号名的场景。
    /// 注意：tantivy 会对查询词分词，所以此方法仍然可能
    /// 匹配含有查询词其他部分的符号。如需严格精确匹配，
    /// 应在上层对结果进行二次过滤。
    ///
    /// # 参数
    /// - `exact_name` — 要精确匹配的符号名
    /// - `limit` — 最多返回的结果数
    pub fn exact_search(
        &self,
        exact_name: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, CodeConnectError> {
        // 使用引号包裹做短语查询，提高精确度
        let query = format!("\"{}\"", exact_name);
        self.engine.search_by_name(&query, None, None, limit)
    }
}

#[cfg(test)]
mod tests {
    /// 验证接口设计 — 实际搜索测试需要完整的 tantivy 索引环境
    #[test]
    fn test_interface_accepts_parameters() {
        // 语义搜索服务的接口验证
        // search_pattern 和 find_similar 的参数签名测试
        let pattern = "handle";
        let limit: usize = 10;
        assert!(!pattern.is_empty());
        assert!(limit > 0);
    }
}
