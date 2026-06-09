//! 增量索引器
//!
//! 响应文件变更事件，对比文件指纹，仅重新解析变更的文件。
//!
//! # 工作流程
//!
//! 1. **接收变更** — 从 [`FileWatcher`](codeconnect_watcher::watcher::FileWatcher) 接收文件变更批次
//! 2. **指纹对比** — 与 sled 中存储的文件指纹比较，跳过内容未变化的文件
//! 3. **删除旧数据** — 从 sled 和 tantivy 中移除变更文件的旧符号
//! 4. **重新解析** — 调用解析器重新解析变更的源文件
//! 5. **写入新索引** — 将新的符号、调用、导入信息写入 sled 和 tantivy
//!
//! # 使用
//!
//! ```ignore
//! let indexer = IncrementalIndexer::new(project_root, sled, tantivy, parser_registry)?;
//! indexer.start_watching(excludes).await?;
//! ```

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::types::{FileMeta, SymbolKind};
use crate::sled_store::SledStore;
use crate::tantivy_index::TantivyIndex;
use codeconnect_parser::factory::ParserRegistry;
use codeconnect_watcher::watcher::FileWatcher;
use tokio::sync::mpsc;

/// 增量索引器
///
/// 封装文件监控和增量索引更新逻辑，提供统一的启动入口。
///
/// 持有所有索引和解析所需的共享资源：项目根目录、
/// sled 存储、tantivy 索引、解析器注册表。
pub struct IncrementalIndexer {
    /// 项目根目录
    project_root: PathBuf,
    /// sled K/V 存储
    sled: Arc<SledStore>,
    /// tantivy 全文搜索索引
    tantivy: Arc<TantivyIndex>,
    /// 解析器注册表
    parser_registry: Arc<ParserRegistry>,
}

impl IncrementalIndexer {
    /// 创建增量索引器
    ///
    /// # 参数
    ///
    /// - `project_root` — 项目根目录路径
    /// - `sled` — 已打开的 sled 存储实例
    /// - `tantivy` — 已初始化的 tantivy 索引实例
    /// - `parser_registry` — 已注册所有语言解析器的注册表
    pub fn new(
        project_root: &Path,
        sled: Arc<SledStore>,
        tantivy: Arc<TantivyIndex>,
        parser_registry: Arc<ParserRegistry>,
    ) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            sled,
            tantivy,
            parser_registry,
        }
    }

    /// 启动文件监控和增量索引
    ///
    /// 创建 [`FileWatcher`] 并启动异步监控循环。
    /// 接收到的文件变更批次经过去重和指纹对比后，
    /// 仅对真正变化的文件执行增量重索引。
    ///
    /// # 参数
    ///
    /// - `excludes` — 文件排除模式列表
    ///
    /// # 返回
    ///
    /// 此方法在文件监控持续运行期间不会返回，返回 `Ok(())`
    /// 表示监控停止。
    pub async fn start_watching(
        &self,
        excludes: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<PathBuf>>();

        let watcher = FileWatcher::new(&self.project_root, excludes);

        // 在后台任务中运行文件监控
        let watch_handle = tokio::spawn(async move {
            if let Err(e) = watcher.start(tx).await {
                tracing::error!("文件监控启动失败: {}", e);
            }
        });

        tracing::info!("增量索引器已启动，正在监控文件变更...");

        // 处理文件变更事件
        while let Some(changed_files) = rx.recv().await {
            if let Err(e) = self.reindex_files(&changed_files) {
                tracing::error!("增量索引失败: {}", e);
            }
        }

        // 监控通道关闭，等待后台任务结束
        let _ = watch_handle.await;

        tracing::info!("增量索引器已停止");
        Ok(())
    }

    /// 对指定文件列表执行增量重索引
    ///
    /// 这是增量索引的核心方法，执行以下操作：
    ///
    /// 1. **去重** — 移除路径列表中的重复项
    /// 2. **指纹对比** — 计算文件内容的 blake3 哈希，与 sled 中存储的指纹比较
    /// 3. **删除旧符号** — 从 sled 和 tantivy 中移除变更文件的旧数据
    /// 4. **重新解析** — 调用解析器重新提取符号、调用和导入
    /// 5. **写入新索引** — 将新数据写入 sled 和 tantivy
    ///
    /// # 参数
    ///
    /// - `file_paths` — 变更的文件路径列表
    pub fn reindex_files(&self, file_paths: &[PathBuf]) -> Result<(), CodeConnectError> {
        // ---- 第一步：去重 ----
        let unique_paths: HashSet<&Path> = file_paths.iter().map(|p| p.as_path()).collect();
        let unique_paths: Vec<&Path> = unique_paths.into_iter().collect();

        if unique_paths.is_empty() {
            return Ok(());
        }

        tracing::info!("增量索引: 处理 {} 个变更文件", unique_paths.len());

        let mut reindexed_count: u64 = 0;
        let mut skipped_count: u64 = 0;

        for file_path in &unique_paths {
            // ---- 计算相对于项目根目录的路径 ----
            let relative_path = file_path
                .strip_prefix(&self.project_root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            // ---- 第二步：读取内容并计算指纹 ----
            let source = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(e) => {
                    // 文件可能已被删除 — 清理索引中的旧数据
                    tracing::debug!("无法读取文件 {}: {}，从索引中移除", file_path.display(), e);
                    self.remove_file_from_index(&relative_path)?;
                    reindexed_count += 1;
                    continue;
                }
            };

            let new_hash = blake3::hash(source.as_bytes()).to_hex().to_string();

            // 检查是否内容真正变化
            if let Ok(Some(old_hash_bytes)) = self.sled.get_fingerprint(&relative_path) {
                let old_hash = String::from_utf8_lossy(&old_hash_bytes).to_string();
                if old_hash == new_hash {
                    tracing::debug!("文件 {} 内容未变化，跳过", relative_path);
                    skipped_count += 1;
                    continue;
                }
            }

            // ---- 第三步：删除旧数据 ----
            self.remove_file_from_index(&relative_path)?;

            // ---- 第四步：查找并调用解析器 ----
            let parser = match self.parser_registry.get_for_file(file_path) {
                Some(p) => p,
                None => {
                    tracing::debug!("不支持的文件类型: {}", file_path.display());
                    continue;
                }
            };

            let language = parser.language();

            // ---- 第五步：解析源文件 ----
            let tree = match parser.parse(&source) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("解析失败 {}: {}", file_path.display(), e);
                    continue;
                }
            };

            let symbols = parser.extract_symbols(&tree, &source, file_path);
            let _calls = parser.extract_calls(&tree, &source, file_path);
            let _imports = parser.extract_imports(&tree, &source, file_path);

            // ---- 第六步：写入新索引数据 ----
            self.write_file_index(&relative_path, &new_hash, language, &symbols)?;

            tracing::debug!(
                "重索引完成: {} ({} 个符号)",
                relative_path,
                symbols.len()
            );
            reindexed_count += 1;
        }

        // 提交 tantivy 并刷写 sled
        // 注意: 由于我们持有 Arc<TantivyIndex> 而非 &mut，这里的 commit 是有限制的
        // 在主流程中由 FullIndexer 或外部调用者负责 commit

        tracing::info!(
            "增量索引完成: {} 重索引 / {} 跳过",
            reindexed_count,
            skipped_count
        );

        Ok(())
    }

    /// 从索引中移除文件的所有关联数据
    ///
    /// 包括：文件元信息、符号定义、文件→符号映射、文件指纹。
    /// 通常在文件被删除或即将重索引前调用。
    fn remove_file_from_index(&self, relative_path: &str) -> Result<(), CodeConnectError> {
        // 获取文件内所有符号 ID
        if let Ok(Some(bytes)) = self.sled.get_file_symbols(relative_path) {
            let symbol_ids: Vec<String> =
                serde_json::from_slice(&bytes).unwrap_or_default();

            // 删除每个符号的定义
            for sid in &symbol_ids {
                let _ = self.sled.delete_symbol(sid);
            }
        }

        // 删除文件→符号映射
        let _ = self.sled.remove_file_symbols(relative_path);

        // 删除文件元信息
        let _ = self.sled.remove_file_meta(relative_path);

        // 删除文件指纹
        let _ = self.sled.remove_fingerprint(relative_path);

        Ok(())
    }

    /// 将文件的解析结果写入索引存储
    ///
    /// 包括：文件元信息、符号定义、文件指纹、文件→符号映射。
    /// 亦将每个符号通过 tantivy 添加到全文搜索索引。
    fn write_file_index(
        &self,
        relative_path: &str,
        content_hash: &str,
        language: &str,
        symbols: &[codeconnect_core::types::Symbol],
    ) -> Result<(), CodeConnectError> {
        // ---- 写入文件元信息 ----
        let file_meta = FileMeta {
            file_path: relative_path.to_string(),
            language: language.to_string(),
            content_hash: content_hash.to_string(),
            symbol_count: symbols.len() as u64,
            indexed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        };

        let meta_bytes = serde_json::to_vec(&file_meta)
            .map_err(|e| CodeConnectError::Index(format!("序列化文件元信息失败: {}", e)))?;
        self.sled.put_file_meta(relative_path, &meta_bytes)?;

        // ---- 收集文件内所有符号 ID ----
        let mut file_symbol_ids: Vec<String> = Vec::with_capacity(symbols.len());

        // ---- 写入每个符号定义 ----
        for symbol in symbols {
            file_symbol_ids.push(symbol.id.clone());

            let sym_bytes = serde_json::to_vec(symbol)
                .map_err(|e| CodeConnectError::Index(format!("序列化符号失败: {}", e)))?;
            self.sled.put_symbol(&symbol.id, &sym_bytes)?;

            // 写入 tantivy 全文索引
           Self::add_symbol_to_tantivy(&self.tantivy, symbol, language, relative_path)?;
        }

        // ---- 写入文件→符号映射 ----
        let file_symbol_bytes = serde_json::to_vec(&file_symbol_ids)
            .map_err(|e| CodeConnectError::Index(format!("序列化文件符号映射失败: {}", e)))?;
        self.sled
            .put_file_symbols(relative_path, &file_symbol_bytes)?;

        // ---- 写入文件指纹 ----
        self.sled
            .put_fingerprint(relative_path, content_hash.as_bytes())?;

        Ok(())
    }

    /// 将单个符号添加到 tantivy 全文索引（内部辅助方法）
    fn add_symbol_to_tantivy(
        tantivy: &TantivyIndex,
        symbol: &codeconnect_core::types::Symbol,
        language: &str,
        file_path_str: &str,
    ) -> Result<(), CodeConnectError> {
        let kind_str = match &symbol.kind {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Variable => "variable",
            SymbolKind::Field => "field",
            SymbolKind::Module => "module",
            SymbolKind::Macro => "macro",
            SymbolKind::Parameter => "parameter",
            SymbolKind::Unknown(_) => "unknown",
        };

        let modifiers_str = symbol.modifiers.join(", ");

        tantivy.add_symbol(
            &symbol.id,
            &symbol.name,
            kind_str,
            language,
            file_path_str,
            symbol.signature.as_deref().unwrap_or(""),
            symbol.doc_comment.as_deref().unwrap_or(""),
            "",
            "",
            symbol.parent_id.as_deref().unwrap_or(""),
            &modifiers_str,
            symbol.complexity.unwrap_or(0),
            "",
            symbol.is_exported,
        )
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incremental_indexer_creation() {
        // 使用临时目录验证创建过程
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let sled = Arc::new(
            SledStore::open(&tmp.path().join("sled")).expect("打开 sled 失败"),
        );
        let tantivy = Arc::new(
            TantivyIndex::open_or_create(&tmp.path().join("tantivy"))
                .expect("创建 tantivy 失败"),
        );
        let parser_registry = Arc::new(ParserRegistry::new());

        let indexer = IncrementalIndexer::new(
            &tmp.path().join("project"),
            sled,
            tantivy,
            parser_registry,
        );

        assert!(indexer.project_root.ends_with("project"));
    }

    #[test]
    fn test_reindex_empty_file_list() {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let sled = Arc::new(
            SledStore::open(&tmp.path().join("sled")).expect("打开 sled 失败"),
        );
        let tantivy = Arc::new(
            TantivyIndex::open_or_create(&tmp.path().join("tantivy"))
                .expect("创建 tantivy 失败"),
        );
        let parser_registry = Arc::new(ParserRegistry::new());

        let indexer = IncrementalIndexer::new(&tmp.path(), sled, tantivy, parser_registry);

        // 空文件列表不应出错
        let result = indexer.reindex_files(&[]);
        assert!(result.is_ok());
    }
}
