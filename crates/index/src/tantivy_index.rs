//! tantivy 全文搜索索引
//!
//! 定义符号文档 Schema，提供索引写入、搜索和版本管理。

use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument};
use codeconnect_core::error::CodeConnectError;

/// 当前 Schema 版本号（用于迁移检测）
/// v2: 新增位置字段 (line, column, end_line, end_column)，doc_comment 改为 STORED
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// 符号文档 Schema 定义
///
/// 封装 tantivy Schema 及其所有字段引用，
/// 作为索引读写操作的统一入口。
pub struct SymbolSchema {
    /// tantivy schema 对象
    pub schema: Schema,
    /// 稳定符号 ID（存储）
    pub stable_id: Field,
    /// 符号名（索引 + 存储）
    pub name: Field,
    /// 符号类型（分面）
    pub kind: Field,
    /// 语言（分面）
    pub language: Field,
    /// 文件路径（索引 + 存储）
    pub file_path: Field,
    /// 签名（索引 + 存储）
    pub signature: Field,
    /// 文档注释（索引 + 存储 — v2 改为 STORED）
    pub doc_comment: Field,
    /// 定义文本（索引）
    pub definition: Field,
    /// 函数体文本（索引）
    pub body_text: Field,
    /// 所属类型/类（索引 + 存储）
    pub parent_type: Field,
    /// 修饰符（分面）
    pub modifiers: Field,
    /// 圈复杂度（存储 + 快速字段）
    pub complexity: Field,
    /// AST 结构哈希（索引 + 存储）
    pub ast_hash: Field,
    /// 是否公开 API（存储 + 快速字段）
    pub is_exported: Field,
    /// 起始行号（存储，v2 新增）
    pub line: Field,
    /// 起始列号（存储，v2 新增）
    pub column: Field,
    /// 结束行号（存储，v2 新增）
    pub end_line: Field,
    /// 结束列号（存储，v2 新增）
    pub end_column: Field,
}

impl SymbolSchema {
    /// 创建 Schema 定义
    ///
    /// 为每个字段配置适当的索引选项：
    /// - `TEXT` 字段：进行分词和索引，适合全文搜索
    /// - `STRING` 字段：精确匹配，适合分面和标识符
    /// - `STORED`：原始值随文档存储，搜索结果中可获取
    /// - `FAST`：列式存储，适合排序和聚合
    pub fn new() -> Self {
        let mut schema_builder = Schema::builder();

        let stable_id = schema_builder.add_text_field("stable_id", STRING | STORED);
        let name = schema_builder.add_text_field("name", TEXT | STORED);
        let kind = schema_builder.add_text_field("kind", STRING | STORED);
        let language = schema_builder.add_text_field("language", STRING | STORED);
        let file_path = schema_builder.add_text_field("file_path", STRING | STORED);
        let signature = schema_builder.add_text_field("signature", TEXT | STORED);
        let doc_comment = schema_builder.add_text_field("doc_comment", TEXT | STORED);
        let definition = schema_builder.add_text_field("definition", TEXT);
        let body_text = schema_builder.add_text_field("body_text", TEXT);
        let parent_type = schema_builder.add_text_field("parent_type", STRING | STORED);
        let modifiers = schema_builder.add_text_field("modifiers", STRING | STORED);
        let complexity = schema_builder.add_u64_field("complexity", STORED | FAST);
        let ast_hash = schema_builder.add_text_field("ast_hash", STRING | STORED);
        let is_exported = schema_builder.add_bool_field("is_exported", STORED | FAST);
        let line = schema_builder.add_u64_field("line", STORED);
        let column = schema_builder.add_u64_field("column", STORED);
        let end_line = schema_builder.add_u64_field("end_line", STORED);
        let end_column = schema_builder.add_u64_field("end_column", STORED);

        let schema = schema_builder.build();

        Self {
            schema,
            stable_id,
            name,
            kind,
            language,
            file_path,
            signature,
            doc_comment,
            definition,
            body_text,
            parent_type,
            modifiers,
            complexity,
            ast_hash,
            is_exported,
            line,
            column,
            end_line,
            end_column,
        }
    }
}

/// tantivy 索引管理器
///
/// 封装索引的创建、写入、提交和搜索操作。
/// 内部维护一个写入器和一个定期重载的读取器，
/// 确保写入后立即可见。
pub struct TantivyIndex {
    /// 索引实例
    index: Index,
    /// 索引写入器（50MB 内存缓冲区）
    writer: IndexWriter,
    /// 索引读取器（Commit 后延迟重载）
    reader: IndexReader,
    /// Schema 引用
    schema: SymbolSchema,
}

impl TantivyIndex {
    /// 创建或打开索引
    ///
    /// 如果 `index_dir` 不存在则创建目录并新建索引；
    /// 如果已存在则直接打开现有索引。
    ///
    /// # 参数
    /// - `index_dir` — 索引目录路径
    pub fn open_or_create(index_dir: &Path) -> Result<Self, CodeConnectError> {
        let schema = SymbolSchema::new();

        let index = if index_dir.exists() {
            Index::open_in_dir(index_dir)
                .map_err(|e| CodeConnectError::Index(format!("无法打开索引: {}", e)))?
        } else {
            std::fs::create_dir_all(index_dir)
                .map_err(|e| CodeConnectError::Index(format!("无法创建索引目录: {}", e)))?;
            Index::create_in_dir(index_dir, schema.schema.clone())
                .map_err(|e| CodeConnectError::Index(format!("无法创建索引: {}", e)))?
        };

        // 写入器：50MB 内存缓冲区
        let writer = index
            .writer(50_000_000)
            .map_err(|e| CodeConnectError::Index(format!("无法创建写入器: {}", e)))?;

        // 读取器：每次 commit 后自动重载
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| CodeConnectError::Index(format!("无法创建读取器: {}", e)))?;

        Ok(Self {
            index,
            writer,
            reader,
            schema,
        })
    }

    /// 添加符号文档到索引
    ///
    /// # 参数
    /// 各参数按 Schema 字段定义一一映射。
    #[allow(clippy::too_many_arguments)]
    pub fn add_symbol(
        &self,
        stable_id: &str,
        name: &str,
        kind: &str,
        language: &str,
        file_path: &str,
        signature: &str,
        doc_comment: &str,
        definition: &str,
        body_text: &str,
        parent_type: &str,
        modifiers: &str,
        complexity: u64,
        ast_hash: &str,
        is_exported: bool,
        line: u64,
        column: u64,
        end_line: u64,
        end_column: u64,
    ) -> Result<(), CodeConnectError> {
        let doc = doc!(
            self.schema.stable_id => stable_id,
            self.schema.name => name,
            self.schema.kind => kind,
            self.schema.language => language,
            self.schema.file_path => file_path,
            self.schema.signature => signature,
            self.schema.doc_comment => doc_comment,
            self.schema.definition => definition,
            self.schema.body_text => body_text,
            self.schema.parent_type => parent_type,
            self.schema.modifiers => modifiers,
            self.schema.complexity => complexity,
            self.schema.ast_hash => ast_hash,
            self.schema.is_exported => is_exported,
            self.schema.line => line,
            self.schema.column => column,
            self.schema.end_line => end_line,
            self.schema.end_column => end_column,
        );

        self.writer
            .add_document(doc)
            .map_err(|e| CodeConnectError::Index(format!("写入文档失败: {}", e)))?;

        Ok(())
    }

    /// 提交所有待写入的变更
    ///
    /// 返回此次提交写入的文档数。
    /// 提交后读取器将自动重载，使新文档可见。
    pub fn commit(&mut self) -> Result<u64, CodeConnectError> {
        self.writer
            .commit()
            .map_err(|e| CodeConnectError::Index(format!("提交失败: {}", e)))
    }

    /// 按名称搜索符号
    ///
    /// 对 `name` 字段进行全文搜索，返回按相关度排序的结果。
    ///
    /// # 参数
    /// - `query_str` — 搜索查询字符串（支持 fts 语法）
    /// - `limit` — 最多返回的结果数
    pub fn search_by_name(
        &self,
        query_str: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, CodeConnectError> {
        self.reader
            .reload()
            .map_err(|e| CodeConnectError::Index(format!("重新加载失败: {}", e)))?;

        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&self.index, vec![self.schema.name]);
        let query = query_parser
            .parse_query(query_str)
            .map_err(|e| CodeConnectError::Query(format!("查询解析失败: {}", e)))?;

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(limit))
            .map_err(|e| CodeConnectError::Query(format!("搜索失败: {}", e)))?;

        let results: Vec<SymbolSearchResult> = top_docs
            .iter()
            .map(|(score, doc_addr)| {
                let doc: TantivyDocument = searcher.doc(*doc_addr).unwrap();
                // 辅助函数：从文档中提取 STORED 文本字段，缺失时返回空字符串
                let get_text = |field: Field| -> String {
                    doc.get_first(field)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                // 辅助函数：从文档中提取 u64 字段
                let get_u64 = |field: Field| -> u64 {
                    doc.get_first(field)
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                };
                // 辅助函数：从文档中提取 bool 字段
                let get_bool = |field: Field| -> bool {
                    doc.get_first(field)
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                };

                SymbolSearchResult {
                    stable_id: get_text(self.schema.stable_id),
                    name: get_text(self.schema.name),
                    kind: get_text(self.schema.kind),
                    language: get_text(self.schema.language),
                    file_path: get_text(self.schema.file_path),
                    signature: get_text(self.schema.signature),
                    doc_comment: get_text(self.schema.doc_comment),
                    parent_type: get_text(self.schema.parent_type),
                    modifiers: get_text(self.schema.modifiers),
                    complexity: get_u64(self.schema.complexity),
                    ast_hash: get_text(self.schema.ast_hash),
                    is_exported: get_bool(self.schema.is_exported),
                    line: get_u64(self.schema.line),
                    column: get_u64(self.schema.column),
                    end_line: get_u64(self.schema.end_line),
                    end_column: get_u64(self.schema.end_column),
                    score: *score,
                }
            })
            .collect();

        Ok(results)
    }

    /// 按稳定 ID 精确搜索符号
    ///
    /// 使用 tantivy term query 对 `stable_id` 字段（STRING 类型，不分词）进行精确匹配。
    /// 返回 None 表示该符号不存在。
    pub fn search_by_id(&self, stable_id: &str) -> Result<Option<SymbolSearchResult>, CodeConnectError> {
        self.reader
            .reload()
            .map_err(|e| CodeConnectError::Index(format!("重新加载失败: {}", e)))?;

        let searcher = self.reader.searcher();

        // 对 STRING 类型的 stable_id 字段使用 Term 查询（精确匹配）
        use tantivy::query::TermQuery;
        use tantivy::Term;
        let term = Term::from_field_text(self.schema.stable_id, stable_id);
        let query = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(1))
            .map_err(|e| CodeConnectError::Query(format!("精确查询失败: {}", e)))?;

        if top_docs.is_empty() {
            return Ok(None);
        }

        let (_score, doc_addr) = &top_docs[0];
        let doc: TantivyDocument = searcher.doc(*doc_addr)
            .map_err(|e| CodeConnectError::Index(format!("读取文档失败: {}", e)))?;

        let get_text = |field: Field| -> String {
            doc.get_first(field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let get_u64 = |field: Field| -> u64 {
            doc.get_first(field)
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
        };
        let get_bool = |field: Field| -> bool {
            doc.get_first(field)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };

        Ok(Some(SymbolSearchResult {
            stable_id: get_text(self.schema.stable_id),
            name: get_text(self.schema.name),
            kind: get_text(self.schema.kind),
            language: get_text(self.schema.language),
            file_path: get_text(self.schema.file_path),
            signature: get_text(self.schema.signature),
            doc_comment: get_text(self.schema.doc_comment),
            parent_type: get_text(self.schema.parent_type),
            modifiers: get_text(self.schema.modifiers),
            complexity: get_u64(self.schema.complexity),
            ast_hash: get_text(self.schema.ast_hash),
            is_exported: get_bool(self.schema.is_exported),
            line: get_u64(self.schema.line),
            column: get_u64(self.schema.column),
            end_line: get_u64(self.schema.end_line),
            end_column: get_u64(self.schema.end_column),
            score: *_score,
        }))
    }

    /// 按文件路径精确搜索符号
    ///
    /// 对 `file_path` 字段（STRING 类型，不分词）进行精确匹配，
    /// 返回该文件下的所有符号。用于替代原来从 sled 读取 file→symbols 映射。
    ///
    /// # 参数
    /// - `file_path` — 文件的相对路径
    pub fn search_by_file_path(
        &self,
        file_path: &str,
    ) -> Result<Vec<SymbolSearchResult>, CodeConnectError> {
        self.reader
            .reload()
            .map_err(|e| CodeConnectError::Index(format!("重新加载失败: {}", e)))?;

        let searcher = self.reader.searcher();

        use tantivy::query::TermQuery;
        use tantivy::Term;
        let term = Term::from_field_text(self.schema.file_path, file_path);
        let query = TermQuery::new(term, tantivy::schema::IndexRecordOption::Basic);

        let top_docs = searcher
            .search(&query, &TopDocs::with_limit(10000))
            .map_err(|e| CodeConnectError::Query(format!("文件路径搜索失败: {}", e)))?;

        let get_text = |doc: &TantivyDocument, field: Field| -> String {
            doc.get_first(field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let get_u64 = |doc: &TantivyDocument, field: Field| -> u64 {
            doc.get_first(field)
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
        };
        let get_bool = |doc: &TantivyDocument, field: Field| -> bool {
            doc.get_first(field)
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };

        let results: Vec<SymbolSearchResult> = top_docs
            .iter()
            .map(|(score, doc_addr)| {
                let doc: TantivyDocument = searcher.doc(*doc_addr).unwrap();
                SymbolSearchResult {
                    stable_id: get_text(&doc, self.schema.stable_id),
                    name: get_text(&doc, self.schema.name),
                    kind: get_text(&doc, self.schema.kind),
                    language: get_text(&doc, self.schema.language),
                    file_path: get_text(&doc, self.schema.file_path),
                    signature: get_text(&doc, self.schema.signature),
                    doc_comment: get_text(&doc, self.schema.doc_comment),
                    parent_type: get_text(&doc, self.schema.parent_type),
                    modifiers: get_text(&doc, self.schema.modifiers),
                    complexity: get_u64(&doc, self.schema.complexity),
                    ast_hash: get_text(&doc, self.schema.ast_hash),
                    is_exported: get_bool(&doc, self.schema.is_exported),
                    line: get_u64(&doc, self.schema.line),
                    column: get_u64(&doc, self.schema.column),
                    end_line: get_u64(&doc, self.schema.end_line),
                    end_column: get_u64(&doc, self.schema.end_column),
                    score: *score,
                }
            })
            .collect();

        Ok(results)
    }

    /// 扫描所有符号的 ID 和名称
    ///
    /// 用于死代码检测等需要遍历所有符号的场景。
    /// 返回 (stable_id, name) 对列表。
    pub fn scan_all_ids(&self) -> Result<Vec<(String, String)>, CodeConnectError> {
        self.reader
            .reload()
            .map_err(|e| CodeConnectError::Index(format!("重新加载失败: {}", e)))?;

        let searcher = self.reader.searcher();
        let mut results = Vec::new();

        for doc_addr in searcher
            .segment_readers()
            .iter()
            .enumerate()
            .flat_map(|(segment_ord, reader)| {
                reader
                    .doc_ids_alive()
                    .map(move |doc_id| tantivy::DocAddress {
                        segment_ord: segment_ord as u32,
                        doc_id,
                    })
            })
        {
            if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_addr) {
                let id = doc
                    .get_first(self.schema.stable_id)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = doc
                    .get_first(self.schema.name)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !id.is_empty() {
                    results.push((id, name));
                }
            }
        }

        Ok(results)
    }

    /// 获取索引中文档总数
    pub fn doc_count(&self) -> Result<u64, CodeConnectError> {
        self.reader
            .reload()
            .map_err(|e| CodeConnectError::Index(format!("重新加载失败: {}", e)))?;
        let searcher = self.reader.searcher();
        Ok(searcher.num_docs())
    }
}

/// 搜索结果
///
/// 包含符号标识信息、相关度评分，以及完整的符号文档字段。
/// 从 tantivy 的 STORED 字段中提取所有数据，调用方无需
/// 再查询 sled 即可获得完整的 Symbol 信息。
#[derive(Debug, Clone)]
pub struct SymbolSearchResult {
    /// 稳定符号 ID
    pub stable_id: String,
    /// 符号名称
    pub name: String,
    /// 符号类型
    pub kind: String,
    /// 编程语言
    pub language: String,
    /// 文件路径
    pub file_path: String,
    /// 函数签名
    pub signature: String,
    /// 文档注释
    pub doc_comment: String,
    /// 所属类型/类
    pub parent_type: String,
    /// 修饰符
    pub modifiers: String,
    /// 圈复杂度
    pub complexity: u64,
    /// AST 结构哈希
    pub ast_hash: String,
    /// 是否公开 API
    pub is_exported: bool,
    /// 起始行号（1-based）
    pub line: u64,
    /// 起始列号（1-based）
    pub column: u64,
    /// 结束行号（1-based）
    pub end_line: u64,
    /// 结束列号（1-based）
    pub end_column: u64,
    /// 相关度评分（BM25）
    pub score: f32,
}
