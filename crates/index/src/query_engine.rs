//! 统一查询引擎
//!
//! 将上层查询请求路由到适当的存储后端（tantivy / sled），
//! 并组合结果。支持全文搜索、精确查找、分页。
//!
//! 注意：符号定义数据只存储在 tantivy 的 STORED 字段中，
//! sled 不再冗余存储符号定义（sled 的 append-only 架构会导致磁盘膨胀）。

use crate::sled_store::SledStore;
use crate::tantivy_index::{SymbolSearchResult, TantivyIndex};
use codeconnect_core::error::CodeConnectError;
use codeconnect_core::types::{
    Symbol, SymbolKind, SourceLocation,
};

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

    /// 按稳定 ID 获取符号的完整信息（从 tantivy STORED 字段反序列化）
    ///
    /// 返回 `None` 表示该 ID 对应的符号不存在。
    ///
    /// # 参数
    /// - `stable_id` — 符号的稳定标识符
    pub fn get_symbol_by_id(&self, stable_id: &str) -> Result<Option<Symbol>, CodeConnectError> {
        self.tantivy.search_by_id(stable_id).map(|opt| {
            opt.map(|result| symbol_search_result_to_symbol(&result))
        })
    }

    /// 按稳定 ID 获取符号的原始 JSON 字节（兼容旧接口）
    ///
    /// 从 tantivy STORED 字段读取 SymbolSearchResult 后重新序列化为 JSON 字节。
    /// 返回 `None` 表示该 ID 对应的符号不存在。
    ///
    /// # 参数
    /// - `stable_id` — 符号的稳定标识符
    pub fn get_symbol_bytes_by_id(&self, stable_id: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        match self.tantivy.search_by_id(stable_id)? {
            Some(result) => {
                let symbol = symbol_search_result_to_symbol(&result);
                let bytes = serde_json::to_vec(&symbol)
                    .map_err(|e| CodeConnectError::Serialization(e))?;
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }

    /// 搜索指定文件路径中的所有符号
    ///
    /// 对 tantivy 的 `file_path` 字段进行精确匹配，返回该文件下的所有符号。
    ///
    /// # 参数
    /// - `file_path` — 源文件的相对路径
    /// 搜索指定文件路径中的所有符号
    ///
    /// 对 tantivy 的 `file_path` 字段进行精确匹配，返回该文件下的所有符号。
    ///
    /// # 参数
    /// - `file_path` — 源文件的相对路径
    pub fn get_file_symbols_tantivy(&self, file_path: &str) -> Result<Vec<Symbol>, CodeConnectError> {
        self.tantivy.search_by_file_path(file_path).map(|results| {
            results.iter().map(|r| symbol_search_result_to_symbol(r)).collect()
        })
    }

    /// 获取文件内所有符号的 ID 列表（从 sled 读文件→符号映射）
    ///
    /// # 参数
    /// - `file_path` — 源文件的绝对路径
    ///
    /// # 返回值
    /// - `Some(Vec<u8>)` 表示该文件有已索引的符号
    /// - `None` 表示该文件尚未索引或无符号
    #[deprecated(note = "文件→符号映射已可从 tantivy search_by_file_path 查询")]
    #[allow(deprecated)]
    pub fn get_file_symbol_ids(&self, file_path: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
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

    /// 扫描所有符号的 ID 和名称
    ///
    /// 用于死代码检测等需要遍历所有符号的场景。
    pub fn scan_all_ids(&self) -> Result<Vec<(String, String)>, CodeConnectError> {
        self.tantivy.scan_all_ids()
    }
}

/// 将 tantivy 搜索结果转换为完整的 Symbol 结构
///
/// 从 Schema 的 STORED 字段中提取所有符号信息并构造 Symbol。
pub fn symbol_search_result_to_symbol(result: &SymbolSearchResult) -> Symbol {
    // 将 kind 字符串解析为 SymbolKind 枚举
    let kind = match result.kind.as_str() {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "class" => SymbolKind::Class,
        "interface" => SymbolKind::Interface,
        "struct" => SymbolKind::Struct,
        "enum" => SymbolKind::Enum,
        "trait" => SymbolKind::Trait,
        "type_alias" => SymbolKind::TypeAlias,
        "variable" => SymbolKind::Variable,
        "field" => SymbolKind::Field,
        "parameter" => SymbolKind::Parameter,
        "module" => SymbolKind::Module,
        "macro" => SymbolKind::Macro,
        other => SymbolKind::Unknown(other.to_string()),
    };

    // 解析修饰符字符串（以 ", " 分隔）
    let modifiers: Vec<String> = result
        .modifiers
        .split(", ")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    Symbol {
        id: result.stable_id.clone(),
        name: result.name.clone(),
        kind,
        location: SourceLocation {
            file_path: result.file_path.clone(),
            line: result.line,
            column: result.column,
            end_line: result.end_line,
            end_column: result.end_column,
        },
        signature: if result.signature.is_empty() {
            None
        } else {
            Some(result.signature.clone())
        },
        doc_comment: if result.doc_comment.is_empty() {
            None
        } else {
            Some(result.doc_comment.clone())
        },
        parent_id: if result.parent_type.is_empty() {
            None
        } else {
            Some(result.parent_type.clone())
        },
        modifiers,
        is_exported: result.is_exported,
        complexity: if result.complexity > 0 {
            Some(result.complexity)
        } else {
            None
        },
    }
}
