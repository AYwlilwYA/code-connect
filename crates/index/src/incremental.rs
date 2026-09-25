//! 增量索引器
//!
//! 响应文件变更事件，对比文件指纹，仅重新解析变更的文件。
//!
//! # 工作流程
//!
//! 1. **接收变更** — 从 [`FileWatcher`](codeconnect_watcher::watcher::FileWatcher) 接收文件变更批次
//! 2. **指纹对比** — 与 sled 中存储的文件指纹比较，跳过内容未变化的文件
//! 3. **重新解析** — 调用解析器重新解析变更的源文件
//! 4. **删除旧数据** — 标记删除变更文件在 tantivy 里的旧符号与旧调用边
//! 5. **写入新索引** — 将新的符号、调用信息写入 tantivy，并刷新 sled 元信息
//!
//! 第 4 步的删除与第 5 步的写入共用同一次 `commit()`（`tantivy` 的删除只在
//! 提交时生效）—— 整批原子替换，不会出现「旧符号残留」或「删了没写」的中间态。
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
use crate::tantivy_index::{CallEdgeIndex, TantivyIndex};
use codeconnect_parser::factory::ParserRegistry;
use codeconnect_watcher::watcher::FileWatcher;
use tokio::sync::mpsc;

/// 增量索引器
///
/// 封装文件监控和增量索引更新逻辑，提供统一的启动入口。
///
/// 持有所有索引和解析所需的共享资源：项目根目录、
/// sled 存储、tantivy 符号索引、tantivy 调用边索引、解析器注册表。
pub struct IncrementalIndexer {
    /// 项目根目录
    project_root: PathBuf,
    /// 索引范围限定：相对 `project_root` 的子目录列表
    ///
    /// 为空或含 `"."` 表示整个 `project_root`。语义与
    /// [`FullIndexer`](crate::full_indexer::FullIndexer) 完全一致，
    /// 否则「全量索引限定范围、增量索引却把范围外文件灌回索引」。
    roots: Vec<PathBuf>,
    /// sled K/V 存储
    sled: Arc<SledStore>,
    /// tantivy 全文搜索索引
    tantivy: Arc<TantivyIndex>,
    /// tantivy 调用边索引（替代 sled edges 命名空间）
    /// 内部 Mutex 保护 IndexWriter，线程安全
    call_edge_index: Arc<CallEdgeIndex>,
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
    /// - `tantivy` — 已初始化的 tantivy 符号索引实例
    /// - `call_edge_index` — 已初始化的 tantivy 调用边索引实例
    /// - `parser_registry` — 已注册所有语言解析器的注册表
    pub fn new(
        project_root: &Path,
        sled: Arc<SledStore>,
        tantivy: Arc<TantivyIndex>,
        call_edge_index: Arc<CallEdgeIndex>,
        parser_registry: Arc<ParserRegistry>,
    ) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            roots: Vec::new(),
            sled,
            tantivy,
            call_edge_index,
            parser_registry,
        }
    }

    /// 设置索引范围限定（相对 `project_root` 的子目录列表）
    ///
    /// 传空列表或 `["."]` 表示监控整个 `project_root`。
    /// 与 [`FullIndexer::with_roots`](crate::full_indexer::FullIndexer::with_roots) 同语义。
    pub fn with_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.roots = roots;
        self
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
    /// 只监控 `workspace.roots` 限定的目录（由
    /// [`IncrementalIndexer::with_roots`] 设置），而不是「监控全项目再过滤事件」，
    /// 否则 serve 一跑就会把范围外的文件重新灌回索引。
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

        // 复用 full_indexer 的 roots 语义作为唯一事实来源，避免出现第二套校验逻辑
        let watch_roots = crate::full_indexer::effective_walk_roots(&self.project_root, &self.roots);
        let watcher = FileWatcher::new(&self.project_root, excludes).with_watch_roots(watch_roots);

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
    /// 3. **重新解析** — 调用解析器重新提取符号、调用和导入（先解析成功再动手删）
    /// 4. **删除旧数据** — 标记删除该文件的 tantivy 符号文档与调用边，清 sled 元信息
    /// 5. **写入新索引** — 新符号与调用边写入 tantivy，刷新 sled 元信息
    /// 6. **提交** — 删除与新增共用同一次 commit，整批原子替换
    ///
    /// 语义是**替换**而非追加：文件改过之后，该文件的符号数等于新内容的符号数，
    /// 不再是新旧之和。
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
        let mut removed_count: u64 = 0;
        // 本批次是否对索引动过手（写入或删除），决定收尾时要不要 commit：
        // tantivy 的删除**在 commit 时才生效**，只标记不提交等于没删。
        let mut mutated = false;

        for file_path in &unique_paths {
            // ---- 计算相对于项目根目录的路径 ----
            // 必须与 full_indexer 一致地归一化为正斜杠：否则同一文件在 Windows 上
            // 会被全量索引写成 `a/b.rs`、被增量索引写成 `a\b.rs`，
            // 索引里出现两份互不相干的记录（查询、去重、计数全部错乱）
            let relative_path = file_path
                .strip_prefix(&self.project_root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .replace('\\', "/");

            // ---- 第二步：读取内容并计算指纹 ----
            let source = match std::fs::read_to_string(file_path) {
                Ok(s) => s,
                Err(e) => {
                    // 文件可能已被删除 — 清理索引中的旧数据
                    tracing::debug!("无法读取文件 {}: {}，从索引中移除", file_path.display(), e);
                    self.remove_file_from_index(&relative_path)?;
                    removed_count += 1;
                    mutated = true;
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

            // ---- 第三步：查找并调用解析器 ----
            // 顺序刻意是「先解析成功、再删旧数据」：反过来（先删后解析）时，
            // 解析失败的文件会被白白删掉 —— 宁可留着旧数据，也好过索引里直接没有
            let parser = match self.parser_registry.get_for_file(file_path) {
                Some(p) => p,
                None => {
                    tracing::debug!("不支持的文件类型: {}", file_path.display());
                    continue;
                }
            };

            let language = parser.language();

            // ---- 第四步：解析源文件 ----
            let tree = match parser.parse(&source) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("解析失败 {}: {}", file_path.display(), e);
                    continue;
                }
            };

            let symbols = parser.extract_symbols(&tree, &source, file_path);
            let calls = parser.extract_calls(&tree, &source, file_path);
            let _imports = parser.extract_imports(&tree, &source, file_path);

            // ---- 第五步：替换式写入新索引数据 ----
            // 必须**先删该文件的旧文档再写新的**：符号文档是纯追加的，
            // 文件改过之后旧符号（改名的函数、删掉的类）不会自己消失，
            // 结果是新旧并存、检索命中数虚高。
            //
            // 删除在这里只做「标记」，与紧随其后的写入共用**同一次 commit**：
            // tantivy 的删除在 commit 时才生效，同一提交窗口里先标删再新增，
            // 提交是原子的 —— 要么读到的全是旧数据、要么全是新数据，没有中间态。
            self.remove_file_from_index(&relative_path)?;
            self.write_file_index(&relative_path, &new_hash, language, &symbols, &calls)?;
            mutated = true;

            tracing::debug!(
                "重索引完成: {} ({} 个符号)",
                relative_path,
                symbols.len()
            );
            reindexed_count += 1;
        }

        // 提交 tantivy 符号索引
        //
        // 此前这里只提交了调用边索引，符号一直悬在 writer 缓冲里不落盘 ——
        // 结果是「文件监控已启动」但改了代码后新符号根本搜不到，
        // 直到某次全量索引的 commit 把它们顺带刷盘时才突然出现（且带着旧路径）。
        //
        // 条件从「有写入」放宽为「动过索引」：只删不写的批次（文件被删掉）
        // 同样必须提交，否则删除停在标记状态、旧符号还在。
        if mutated {
            // commit() 返回 opstamp 而非文档数，要报文档数就提交后实查一次
            match self.tantivy.commit() {
                Ok(_) => match self.tantivy.doc_count() {
                    Ok(count) => tracing::debug!("提交符号索引完成: 索引共 {} 个文档", count),
                    Err(e) => tracing::warn!("提交后统计符号文档数失败: {}", e),
                },
                Err(e) => tracing::error!("提交符号索引失败: {}", e),
            }
        }

        // 提交 tantivy 调用边索引
        if mutated {
            if let Err(e) = self.call_edge_index.commit() {
                tracing::error!("提交调用边索引失败: {}", e);
            }
        }

        // 记录索引更新时间，供 MCP 侧计算陈旧度（增量更新同样使索引变新）
        if mutated {
            let built_at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            if let Err(e) = self.sled.put_index_built_at(built_at) {
                tracing::warn!("写入索引更新时间失败: {}", e);
            }
        }

        tracing::info!(
            "增量索引完成: {} 重索引 / {} 已移除 / {} 跳过",
            reindexed_count,
            removed_count,
            skipped_count
        );

        Ok(())
    }

    /// 从索引中移除文件的所有关联数据
    ///
    /// 包括：tantivy 的符号文档与调用边文档、sled 的文件元信息与文件指纹。
    /// 文件→符号映射不再存 sled，通过 tantivy search_by_file_path 查询。
    ///
    /// ⚠️ tantivy 的两处删除都只是**标记**，在 `commit()` 时才生效 ——
    /// 调用方必须在同一批次结束时提交，否则表现为「删了却还搜得到」。
    /// 本方法刻意不自己 commit：一次批次里多个文件只提交一次，
    /// 且与紧随其后的新增共用同一提交窗口（整批原子替换）。
    fn remove_file_from_index(&self, relative_path: &str) -> Result<(), CodeConnectError> {
        // 删除该文件的符号文档（按 file_path 字段精确删）
        self.tantivy.stage_delete_by_file_path(relative_path)?;

        // 删除该文件的调用边文档（按调用点所在文件删）
        self.call_edge_index.stage_delete_by_file_path(relative_path)?;

        // 删除文件元信息
        let _ = self.sled.remove_file_meta(relative_path);

        // 删除文件指纹
        let _ = self.sled.remove_fingerprint(relative_path);

        Ok(())
    }

    /// 将文件的解析结果写入索引存储
    ///
    /// 包括：文件元信息（sled）、符号定义（tantivy 符号索引）、
    /// 调用边（tantivy 调用边索引）、文件指纹（sled）。
    /// sled 不再存储文件→符号映射（file_symbols），该映射可通过 tantivy 的
    /// search_by_file_path 直接查询，且无需维护两边一致性。
    fn write_file_index(
        &self,
        relative_path: &str,
        content_hash: &str,
        language: &str,
        symbols: &[codeconnect_core::types::Symbol],
        calls: &[codeconnect_core::types::CallSite],
    ) -> Result<(), CodeConnectError> {
        // ---- 写入文件元信息（sled） ----
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

        // ---- 写入每个符号定义（tantivy 符号索引） ----
        // 符号数据只存入 tantivy，sled 不再冗余存储。
        // 文件→符号映射通过 tantivy 的 search_by_file_path 查询。
        for symbol in symbols {
            Self::add_symbol_to_tantivy(&self.tantivy, symbol, language, relative_path)?;
        }

        // ---- 写入调用边（tantivy 调用边索引） ----
        // 建立从符号名到 StableSymbolId 的快速查找表
        let name_to_sym_id: std::collections::HashMap<&str, &str> = symbols
            .iter()
            .map(|s| (s.name.as_str(), s.id.as_str()))
            .collect();

        for call in calls {
            if call.callee_name.is_empty() {
                continue;
            }

            // 被调用方 ID：优先匹配文件内符号的 StableSymbolId，否则直接用 callee_name
            let callee_id = name_to_sym_id
                .get(call.callee_name.as_str())
                .map(|&id| id.to_string())
                .unwrap_or_else(|| call.callee_name.clone());

            // 如果 CallSite 已有 caller_id，直接使用
            if !call.caller_id.is_empty() {
                let edge = codeconnect_core::types::CallEdge {
                    caller_id: call.caller_id.clone(),
                    callee_id: callee_id.clone(),
                    location: call.location.clone(),
                    call_type: call.call_type.clone(),
                    confidence: call.confidence,
                };
                let edge_json = serde_json::to_string(&edge)
                    .map_err(|e| CodeConnectError::Index(format!("序列化调用边失败: {}", e)))?;
                self.call_edge_index.add_call_edge(
                    &call.caller_id,
                    &call.callee_name,
                    &callee_id,
                    &call.location.file_path,
                    call.location.line,
                    call.location.column,
                    &format!("{:?}", call.call_type),
                    call.confidence,
                    &edge_json,
                )?;
            } else {
                // 无 caller_id 时，通过行号范围匹配找到包含此调用的函数
                let call_line = call.location.line;
                for symbol in symbols {
                    if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
                        continue;
                    }
                    if call_line > 0
                        && symbol.location.line > 0
                        && symbol.location.end_line > 0
                        && call_line >= symbol.location.line
                        && call_line <= symbol.location.end_line
                    {
                        let edge = codeconnect_core::types::CallEdge {
                            caller_id: symbol.id.clone(),
                            callee_id: callee_id.clone(),
                            location: call.location.clone(),
                            call_type: call.call_type.clone(),
                            confidence: call.confidence,
                        };
                        let edge_json = serde_json::to_string(&edge)
                            .map_err(|e| CodeConnectError::Index(format!("序列化调用边失败: {}", e)))?;
                        self.call_edge_index.add_call_edge(
                            &symbol.id,
                            &call.callee_name,
                            &callee_id,
                            &call.location.file_path,
                            call.location.line,
                            call.location.column,
                            &format!("{:?}", call.call_type),
                            call.confidence,
                            &edge_json,
                        )?;
                    }
                }
            }
        }

        // ---- 写入文件指纹（sled） ----
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
            SymbolKind::Constant => "constant",
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
            symbol.location.line,
            symbol.location.column,
            symbol.location.end_line,
            symbol.location.end_column,
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
        let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(
            &tmp.path().join("tantivy_edges"),
        ).expect("创建调用边索引失败"));
        let parser_registry = Arc::new(ParserRegistry::new());

        let indexer = IncrementalIndexer::new(
            &tmp.path().join("project"),
            sled,
            tantivy,
            call_edge_index,
            parser_registry,
        );

        assert!(indexer.project_root.ends_with("project"));
    }

    /// roots 默认空 = 监控整个 project_root（向后兼容）
    #[test]
    fn test_incremental_indexer_roots_default_empty() {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let sled = Arc::new(SledStore::open(&tmp.path().join("sled")).expect("打开 sled 失败"));
        let tantivy = Arc::new(
            TantivyIndex::open_or_create(&tmp.path().join("tantivy"))
                .expect("创建 tantivy 失败"),
        );
        let call_edge_index = Arc::new(
            CallEdgeIndex::open_or_create(&tmp.path().join("tantivy_edges"))
                .expect("创建调用边索引失败"),
        );
        let parser_registry = Arc::new(ParserRegistry::new());

        let indexer =
            IncrementalIndexer::new(tmp.path(), sled, tantivy, call_edge_index, parser_registry)
                .with_roots(vec![PathBuf::from("sub")]);

        assert_eq!(indexer.roots, vec![PathBuf::from("sub")]);
    }

    /// roots 全部无效时监控不启动（而不是回退到监控整个项目根）
    #[test]
    fn test_start_watching_invalid_roots_watches_nothing() {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let sled = Arc::new(SledStore::open(&tmp.path().join("sled")).expect("打开 sled 失败"));
        let tantivy = Arc::new(
            TantivyIndex::open_or_create(&tmp.path().join("tantivy"))
                .expect("创建 tantivy 失败"),
        );
        let call_edge_index = Arc::new(
            CallEdgeIndex::open_or_create(&tmp.path().join("tantivy_edges"))
                .expect("创建调用边索引失败"),
        );
        let parser_registry = Arc::new(ParserRegistry::new());

        let indexer =
            IncrementalIndexer::new(tmp.path(), sled, tantivy, call_edge_index, parser_registry)
                .with_roots(vec![PathBuf::from("subb")]);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("创建运行时失败");

        // 监控起点为空 → 立即返回 Ok，不会挂住
        let result = rt.block_on(indexer.start_watching(Vec::new()));
        assert!(result.is_ok(), "无效 roots 应安全退出而非回退全项目监控");
    }

    /// 构造一个注册了 Rust 解析器的增量索引器（存储目录与项目目录分开）
    fn make_indexer(project_root: &Path, storage_root: &Path) -> IncrementalIndexer {
        let mut registry = ParserRegistry::new();
        registry.register(Arc::new(codeconnect_parser::rust::RustParser::new()));

        let sled =
            Arc::new(SledStore::open(&storage_root.join("sled")).expect("打开 sled 失败"));
        let tantivy = Arc::new(
            TantivyIndex::open_or_create(&storage_root.join("tantivy"))
                .expect("创建 tantivy 失败"),
        );
        let call_edge_index = Arc::new(
            CallEdgeIndex::open_or_create(&storage_root.join("tantivy_edges"))
                .expect("创建调用边索引失败"),
        );

        IncrementalIndexer::new(
            project_root,
            sled,
            tantivy,
            call_edge_index,
            Arc::new(registry),
        )
    }

    /// 在临时目录里建一个只含 `src/a.rs` 的项目，返回 (项目根, 该文件路径)
    fn make_project(tmp: &Path, content: &str) -> (PathBuf, PathBuf) {
        let project = tmp.join("project");
        let src_dir = project.join("src");
        std::fs::create_dir_all(&src_dir).expect("创建目录失败");
        let file = src_dir.join("a.rs");
        std::fs::write(&file, content).expect("写文件失败");
        (project, file)
    }

    /// 改动文件后增量重索引：该文件符号数 = **新内容**的符号数（不是新旧之和）
    ///
    /// 这是「增量索引旧符号永远残留」的回归测试。
    #[test]
    fn test_reindex_replaces_old_symbols() {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let (project, file) = make_project(tmp.path(), "pub fn alpha() {}\npub fn beta() {}\n");
        let indexer = make_indexer(&project, &tmp.path().join("storage"));

        indexer
            .reindex_files(&[file.clone()])
            .expect("首次增量索引失败");
        assert_eq!(indexer.tantivy.doc_count().unwrap(), 2, "首次应有 2 个符号");

        // 改内容：beta 改名为 gamma
        std::fs::write(&file, "pub fn alpha() {}\npub fn gamma() {}\n").expect("写文件失败");
        indexer
            .reindex_files(&[file.clone()])
            .expect("二次增量索引失败");

        assert_eq!(
            indexer.tantivy.doc_count().unwrap(),
            2,
            "应是新内容的 2 个符号，而不是新旧之和 4 个"
        );

        let names: Vec<String> = indexer
            .tantivy
            .search_by_file_path("src/a.rs")
            .expect("按路径查询失败")
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(names.contains(&"gamma".to_string()), "新符号应进索引: {:?}", names);
        assert!(
            !names.contains(&"beta".to_string()),
            "旧符号 beta 必须被删掉: {:?}",
            names
        );
    }

    /// 文件被删掉后增量重索引：该文件的符号与 sled 元信息必须一并消失
    #[test]
    fn test_reindex_removes_symbols_of_deleted_file() {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let (project, file) = make_project(tmp.path(), "pub fn alpha() {}\n");
        let indexer = make_indexer(&project, &tmp.path().join("storage"));

        indexer
            .reindex_files(&[file.clone()])
            .expect("首次增量索引失败");
        assert_eq!(indexer.tantivy.doc_count().unwrap(), 1);

        std::fs::remove_file(&file).expect("删除测试文件失败");
        indexer
            .reindex_files(&[file.clone()])
            .expect("文件删除后的增量索引失败");

        assert_eq!(
            indexer.tantivy.doc_count().unwrap(),
            0,
            "文件已删，它的符号必须一并消失"
        );
        assert!(
            indexer.sled.get_file_meta("src/a.rs").unwrap().is_none(),
            "sled 元信息必须一并清理"
        );
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
        let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(
            &tmp.path().join("tantivy_edges"),
        ).expect("创建调用边索引失败"));
        let parser_registry = Arc::new(ParserRegistry::new());

        let indexer = IncrementalIndexer::new(&tmp.path(), sled, tantivy, call_edge_index, parser_registry);

        // 空文件列表不应出错
        let result = indexer.reindex_files(&[]);
        assert!(result.is_ok());
    }
}
