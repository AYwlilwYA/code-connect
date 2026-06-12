//! sled K/V 存储
//!
//! 提供符号、边、导入关系的持久化存储。
//! 命名空间通过键前缀实现：
//!
//! | 前缀 | 用途 |
//! |------|------|
//! | `meta:` | 文件元信息 |
//! | `symbols:` | 符号定义 |
//! | `edges:` | 调用边 |
//! | `refs:` | 引用边 |
//! | `imports:` | 导入信息 |
//! | `file_symbols:` | 文件→符号映射 |
//! | `neighbors:` | 邻接表 |
//! | `index_meta:` | 索引元信息（指纹、版本） |

use std::path::Path;
use sled::Db;
use codeconnect_core::error::CodeConnectError;

/// 键前缀常量
const PREFIX_META: &str = "meta:";
const PREFIX_SYMBOLS: &str = "symbols:";
const PREFIX_EDGES: &str = "edges:";
const PREFIX_REFS: &str = "refs:";
const PREFIX_IMPORTS: &str = "imports:";
const PREFIX_FILE_SYMBOLS: &str = "file_symbols:";
const PREFIX_NEIGHBORS: &str = "neighbors:";
const PREFIX_INDEX_META: &str = "index_meta:";

/// sled 存储管理器
///
/// 基于 sled 嵌入式数据库提供持久化 K/V 存储。
/// 所有数据以字节形式存储，上层负责序列化/反序列化。
pub struct SledStore {
    db: Db,
}

impl SledStore {
    /// 打开或创建 sled 数据库
    ///
    /// 警告：`sled::open` 在目录不存在时内部调用 `fs::create_dir_all`，
    /// 会递归创建父目录（包括 `.codeconnect/`）。因此只应在 index 命令中使用，
    /// serve / analyze / search / status 等只读场景应使用 `open_only`。
    ///
    /// # 参数
    /// - `path` — 数据库目录路径，目录不存在时会自动创建
    pub fn open(path: &Path) -> Result<Self, CodeConnectError> {
        let db = sled::open(path)
            .map_err(|e| CodeConnectError::Index(format!("无法打开 sled: {}", e)))?;
        Ok(Self { db })
    }

    /// 仅打开已有 sled 数据库（目录不存在时不自动创建）
    ///
    /// 在调用 `sled::open` 之前检查目录是否存在，
    /// 避免 sled 内部 `fs::create_dir_all` 自动创建
    /// `.codeconnect/` 父目录。适用于 serve 等只读场景。
    pub fn open_only(path: &Path) -> Result<Self, CodeConnectError> {
        if !path.exists() {
            return Err(CodeConnectError::Index(format!(
                "sled 数据目录不存在: {}",
                path.display()
            )));
        }
        // 此时目录已存在，sled::open 不会再触发 fs::create_dir_all
        Self::open(path)
    }

    // ===== 文件元信息 =====

    /// 存储文件元信息
    ///
    /// `data` 应为 `FileMeta` 的序列化（JSON/bincode）字节。
    pub fn put_file_meta(&self, file_path: &str, data: &[u8]) -> Result<(), CodeConnectError> {
        let key = format!("{}{}", PREFIX_META, file_path);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入文件元信息失败: {}", e)))?;
        Ok(())
    }

    /// 读取文件元信息
    ///
    /// 返回 `None` 表示该文件尚无元信息记录。
    pub fn get_file_meta(&self, file_path: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        let key = format!("{}{}", PREFIX_META, file_path);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取文件元信息失败: {}", e)))?
            .map(|v| v.to_vec()))
    }

    /// 删除文件元信息
    pub fn remove_file_meta(&self, file_path: &str) -> Result<(), CodeConnectError> {
        let key = format!("{}{}", PREFIX_META, file_path);
        self.db
            .remove(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("删除文件元信息失败: {}", e)))?;
        Ok(())
    }

    // ===== 符号定义 =====

    /// 存储符号定义
    ///
    /// `data` 应为 `Symbol` 的序列化字节。
    #[deprecated(note = "符号定义已只存 tantivy，sled 不再冗余存储")]
    pub fn put_symbol(&self, stable_id: &str, data: &[u8]) -> Result<(), CodeConnectError> {
        let key = format!("{}{}", PREFIX_SYMBOLS, stable_id);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入符号失败: {}", e)))?;
        Ok(())
    }

    /// 读取符号定义
    ///
    /// 返回 `None` 表示该符号不存在。
    #[deprecated(note = "符号定义已只存 tantivy，请使用 TantivyIndex::search_by_id")]
    pub fn get_symbol(&self, stable_id: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        let key = format!("{}{}", PREFIX_SYMBOLS, stable_id);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取符号失败: {}", e)))?
            .map(|v| v.to_vec()))
    }

    /// 删除符号定义
    #[deprecated(note = "符号定义已只存 tantivy，sled 不再管理符号生命周期")]
    pub fn delete_symbol(&self, stable_id: &str) -> Result<(), CodeConnectError> {
        let key = format!("{}{}", PREFIX_SYMBOLS, stable_id);
        self.db
            .remove(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("删除符号失败: {}", e)))?;
        Ok(())
    }

    /// 批量存储符号（使用原子批处理）
    #[deprecated(note = "符号定义已只存 tantivy，sled 不再批量管理符号")]
    pub fn put_symbols_batch(
        &self,
        entries: &[(&str, &[u8])],
    ) -> Result<(), CodeConnectError> {
        let mut batch = sled::Batch::default();
        for (stable_id, data) in entries {
            let key = format!("{}{}", PREFIX_SYMBOLS, stable_id);
            batch.insert(key.as_bytes(), *data);
        }
        self.db
            .apply_batch(batch)
            .map_err(|e| CodeConnectError::Index(format!("批量写入符号失败: {}", e)))?;
        Ok(())
    }

    // ===== 调用边 =====

    /// 存储调用边
    ///
    /// `data` 应为 `CallEdge` 的序列化字节。
    #[deprecated(note = "调用边已迁入 tantivy 调用边索引，请使用 CallEdgeIndex::add_call_edge")]
    pub fn put_call_edge(
        &self,
        caller_id: &str,
        callee_id: &str,
        data: &[u8],
    ) -> Result<(), CodeConnectError> {
        let key = format!("{}{}::{}", PREFIX_EDGES, caller_id, callee_id);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入调用边失败: {}", e)))?;
        Ok(())
    }

    /// 读取调用边
    #[deprecated(note = "调用边已迁入 tantivy，请使用 CallEdgeIndex")]
    pub fn get_call_edge(
        &self,
        caller_id: &str,
        callee_id: &str,
    ) -> Result<Option<Vec<u8>>, CodeConnectError> {
        let key = format!("{}{}::{}", PREFIX_EDGES, caller_id, callee_id);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取调用边失败: {}", e)))?
            .map(|v| v.to_vec()))
    }

    /// 删除调用边
    #[deprecated(note = "调用边已迁入 tantivy，sled 不再管理调用边生命周期")]
    pub fn remove_call_edge(
        &self,
        caller_id: &str,
        callee_id: &str,
    ) -> Result<(), CodeConnectError> {
        let key = format!("{}{}::{}", PREFIX_EDGES, caller_id, callee_id);
        self.db
            .remove(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("删除调用边失败: {}", e)))?;
        Ok(())
    }

    /// 获取某个调用者的所有出边
    ///
    /// 遍历以 `edges:{caller_id}::` 为前缀的所有键。
    #[deprecated(note = "调用边已迁入 tantivy，请使用 CallEdgeIndex::search_edges_by_caller")]
    pub fn get_call_edges_from(
        &self,
        caller_id: &str,
    ) -> Result<Vec<(String, Vec<u8>)>, CodeConnectError> {
        let prefix = format!("{}{}::", PREFIX_EDGES, caller_id);
        let mut results = Vec::new();
        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (key, value) =
                item.map_err(|e| CodeConnectError::Index(format!("扫描调用边失败: {}", e)))?;
            results.push((String::from_utf8_lossy(&key).to_string(), value.to_vec()));
        }
        Ok(results)
    }

    // ===== 引用边 =====

    /// 存储引用边
    ///
    /// `data` 应为 `RefEdge` 的序列化字节。
    pub fn put_ref_edge(
        &self,
        ref_id: &str,
        target_id: &str,
        data: &[u8],
    ) -> Result<(), CodeConnectError> {
        let key = format!("{}{}::{}", PREFIX_REFS, ref_id, target_id);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入引用边失败: {}", e)))?;
        Ok(())
    }

    /// 读取引用边
    pub fn get_ref_edge(
        &self,
        ref_id: &str,
        target_id: &str,
    ) -> Result<Option<Vec<u8>>, CodeConnectError> {
        let key = format!("{}{}::{}", PREFIX_REFS, ref_id, target_id);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取引用边失败: {}", e)))?
            .map(|v| v.to_vec()))
    }

    // ===== 导入关系 =====

    /// 存储导入信息
    ///
    /// `data` 应为 `Import` 的序列化字节。
    pub fn put_import(
        &self,
        file_path: &str,
        import_path: &str,
        data: &[u8],
    ) -> Result<(), CodeConnectError> {
        let key = format!("{}{}::{}", PREFIX_IMPORTS, file_path, import_path);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入导入信息失败: {}", e)))?;
        Ok(())
    }

    /// 获取某个文件的所有导入
    pub fn get_imports_for_file(
        &self,
        file_path: &str,
    ) -> Result<Vec<Vec<u8>>, CodeConnectError> {
        let prefix = format!("{}{}::", PREFIX_IMPORTS, file_path);
        let mut results = Vec::new();
        for item in self.db.scan_prefix(prefix.as_bytes()) {
            let (_, value) =
                item.map_err(|e| CodeConnectError::Index(format!("扫描导入失败: {}", e)))?;
            results.push(value.to_vec());
        }
        Ok(results)
    }

    // ===== 文件→符号映射 =====

    /// 存储文件→符号列表的映射
    ///
    /// `data` 应为符号 ID 列表的序列化字节（如 JSON 数组）。
    #[deprecated(note = "文件→符号映射可通过 tantivy search_by_file_path 查询，不再冗余存储")]
    pub fn put_file_symbols(
        &self,
        file_path: &str,
        data: &[u8],
    ) -> Result<(), CodeConnectError> {
        let key = format!("{}{}", PREFIX_FILE_SYMBOLS, file_path);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入文件符号映射失败: {}", e)))?;
        Ok(())
    }

    /// 读取文件→符号列表的映射
    #[deprecated(note = "文件→符号映射可通过 tantivy search_by_file_path 查询")]
    pub fn get_file_symbols(&self, file_path: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        let key = format!("{}{}", PREFIX_FILE_SYMBOLS, file_path);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取文件符号映射失败: {}", e)))?
            .map(|v| v.to_vec()))
    }

    /// 删除文件→符号映射
    #[deprecated(note = "文件→符号映射已不再存 sled，无需删除")]
    pub fn remove_file_symbols(&self, file_path: &str) -> Result<(), CodeConnectError> {
        let key = format!("{}{}", PREFIX_FILE_SYMBOLS, file_path);
        self.db
            .remove(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("删除文件符号映射失败: {}", e)))?;
        Ok(())
    }

    // ===== 邻接表 =====

    /// 存储符号的邻接表
    ///
    /// `data` 应为相邻符号 ID 列表的序列化字节。
    pub fn put_neighbors(
        &self,
        symbol_id: &str,
        data: &[u8],
    ) -> Result<(), CodeConnectError> {
        let key = format!("{}{}", PREFIX_NEIGHBORS, symbol_id);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入邻接表失败: {}", e)))?;
        Ok(())
    }

    /// 读取符号的邻接表
    pub fn get_neighbors(&self, symbol_id: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        let key = format!("{}{}", PREFIX_NEIGHBORS, symbol_id);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取邻接表失败: {}", e)))?
            .map(|v| v.to_vec()))
    }

    // ===== 文件指纹 =====

    /// 存储文件指纹（内容哈希）
    ///
    /// 用于增量索引时检测文件是否变更。
    pub fn put_fingerprint(
        &self,
        file_path: &str,
        data: &[u8],
    ) -> Result<(), CodeConnectError> {
        let key = format!("{}fingerprint:{}", PREFIX_INDEX_META, file_path);
        self.db
            .insert(key.as_bytes(), data)
            .map_err(|e| CodeConnectError::Index(format!("写入文件指纹失败: {}", e)))?;
        Ok(())
    }

    /// 读取文件指纹
    pub fn get_fingerprint(&self, file_path: &str) -> Result<Option<Vec<u8>>, CodeConnectError> {
        let key = format!("{}fingerprint:{}", PREFIX_INDEX_META, file_path);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取文件指纹失败: {}", e)))?
            .map(|v| v.to_vec()))
    }

    /// 删除文件指纹
    pub fn remove_fingerprint(&self, file_path: &str) -> Result<(), CodeConnectError> {
        let key = format!("{}fingerprint:{}", PREFIX_INDEX_META, file_path);
        self.db
            .remove(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("删除文件指纹失败: {}", e)))?;
        Ok(())
    }

    // ===== Schema 版本 =====

    /// 存储 Schema 版本号
    ///
    /// 用于索引迁移时检测是否需要重建索引。
    pub fn put_schema_version(&self, version: u32) -> Result<(), CodeConnectError> {
        let key = format!("{}schema_version", PREFIX_INDEX_META);
        let data = version.to_le_bytes();
        self.db
            .insert(key.as_bytes(), &data[..])
            .map_err(|e| CodeConnectError::Index(format!("写入版本失败: {}", e)))?;
        Ok(())
    }

    /// 读取 Schema 版本号
    ///
    /// 返回 `None` 表示版本信息尚未写入（旧版本索引）。
    pub fn get_schema_version(&self) -> Result<Option<u32>, CodeConnectError> {
        let key = format!("{}schema_version", PREFIX_INDEX_META);
        Ok(self
            .db
            .get(key.as_bytes())
            .map_err(|e| CodeConnectError::Index(format!("读取版本失败: {}", e)))?
            .map(|v| {
                let mut arr = [0u8; 4];
                let len = 4.min(v.len());
                arr[..len].copy_from_slice(&v[..len]);
                u32::from_le_bytes(arr)
            }))
    }

    // ===== 批量操作 =====

    /// 批处理：原子地执行多个键值对操作
    ///
    /// 接受插入和删除操作列表，在单次事务中提交。
    pub fn apply_batch(
        &self,
        inserts: &[(&[u8], &[u8])],
        removals: &[&[u8]],
    ) -> Result<(), CodeConnectError> {
        let mut batch = sled::Batch::default();
        for (key, value) in inserts {
            batch.insert(*key, *value);
        }
        for key in removals {
            batch.remove(*key);
        }
        self.db
            .apply_batch(batch)
            .map_err(|e| CodeConnectError::Index(format!("批处理失败: {}", e)))?;
        Ok(())
    }

    /// 扫描指定前缀的所有键值对
    pub fn scan_prefix(
        &self,
        prefix: &[u8],
    ) -> impl Iterator<Item = Result<(Vec<u8>, Vec<u8>), CodeConnectError>> + '_ {
        self.db.scan_prefix(prefix).map(|item| {
            item.map(|(k, v)| (k.to_vec(), v.to_vec()))
                .map_err(|e| CodeConnectError::Index(format!("扫描失败: {}", e)))
        })
    }

    // ===== 生命周期 =====

    /// 刷写所有待写入数据到磁盘
    pub fn flush(&self) -> Result<(), CodeConnectError> {
        self.db
            .flush()
            .map_err(|e| CodeConnectError::Index(format!("刷写失败: {}", e)))?;
        Ok(())
    }

    /// 获取数据库中的近似条目数
    pub fn size(&self) -> usize {
        self.db.len()
    }

    /// 检查数据库中是否包含指定键
    pub fn contains_key(&self, key: &[u8]) -> Result<bool, CodeConnectError> {
        self.db
            .contains_key(key)
            .map_err(|e| CodeConnectError::Index(format!("检查键失败: {}", e)))
    }
}
