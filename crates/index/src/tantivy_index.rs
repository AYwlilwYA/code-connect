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
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

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
    /// 文档注释（索引）
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
        let doc_comment = schema_builder.add_text_field("doc_comment", TEXT);
        let definition = schema_builder.add_text_field("definition", TEXT);
        let body_text = schema_builder.add_text_field("body_text", TEXT);
        let parent_type = schema_builder.add_text_field("parent_type", STRING | STORED);
        let modifiers = schema_builder.add_text_field("modifiers", STRING | STORED);
        let complexity = schema_builder.add_u64_field("complexity", STORED | FAST);
        let ast_hash = schema_builder.add_text_field("ast_hash", STRING | STORED);
        let is_exported = schema_builder.add_bool_field("is_exported", STORED | FAST);

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
                SymbolSearchResult {
                    stable_id: doc
                        .get_first(self.schema.stable_id)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string(),
                    name: doc
                        .get_first(self.schema.name)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string(),
                    kind: doc
                        .get_first(self.schema.kind)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string(),
                    score: *score,
                }
            })
            .collect();

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
/// 包含符号标识信息和相关度评分。
#[derive(Debug, Clone)]
pub struct SymbolSearchResult {
    /// 稳定符号 ID
    pub stable_id: String,
    /// 符号名称
    pub name: String,
    /// 符号类型
    pub kind: String,
    /// 相关度评分（BM25）
    pub score: f32,
}
