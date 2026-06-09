//! CodeConnect 索引引擎模块
//!
//! 负责代码索引的构建与查询：
//! - [`tantivy_index`] — 基于 tantivy 的全文搜索索引（Schema、Writer、Reader、搜索）
//! - [`sled_store`] — 基于 sled 的键值存储（符号、边、导入、文件指纹）
//! - [`full_indexer`] — 全量索引引擎（目录遍历 → parallel 解析 → 批量写入）
//! - [`incremental`] — 增量索引（文件变更检测 → 差分更新）
//! - [`query_engine`] — 统一查询入口

pub mod full_indexer;
pub mod incremental;
pub mod query_engine;
pub mod sled_store;
pub mod tantivy_index;
