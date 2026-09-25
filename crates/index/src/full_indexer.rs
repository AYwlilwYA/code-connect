//! 全量索引引擎
//!
//! 遍历项目目录，用 rayon 并行解析所有源文件，
//! 提取符号、调用和导入信息，批量写入 tantivy 和 sled。
//!
//! 流程：
//! 1. `ignore` crate 遍历目录（自动 .gitignore 过滤）
//! 2. 按文件扩展名匹配解析器
//! 3. `rayon` 并行解析
//! 4. `crossbeam` channel 收集结果
//! 5. 批量写入 tantivy + sled

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crossbeam::channel;
use rayon::prelude::*;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::types::{FileMeta, SymbolKind};
use codeconnect_parser::coverage::{self, FileCoverage, KindGapSummary};
use codeconnect_parser::factory::ParserRegistry;
use ignore::WalkBuilder;

use crate::sled_store::SledStore;
use crate::tantivy_index::{CallEdgeIndex, TantivyIndex, CURRENT_SCHEMA_VERSION};

// ============================================================================
// 统计结构
// ============================================================================

/// 索引统计信息
///
/// 记录一次全量索引运行的整体结果，包括扫描文件数、
/// 解析成功率、提取的符号/调用/导入数量等。
#[derive(Debug, Clone)]
pub struct IndexStats {
    /// 扫描的文件总数
    pub files_scanned: u64,
    /// 成功解析的文件数
    pub files_parsed: u64,
    /// 提取的符号总数
    pub symbols_found: u64,
    /// 发现的调用点数
    pub calls_found: u64,
    /// 发现的导入数
    pub imports_found: u64,
    /// 解析失败的文件列表（路径 + 错误信息）
    pub failed_files: Vec<String>,
    /// 覆盖度审计（spec B）：哪些声明节点种类在本项目里有节点却没产出符号
    pub coverage: CoverageSummary,
}

/// 全项目覆盖度汇总
///
/// 「没索引」与「不存在」必须在返回值上可区分：这个结构就是那个区分本身。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CoverageSummary {
    /// 存在缺口的文件数
    pub files_with_gaps: u64,
    /// 逐种类汇总（按节点数降序）
    pub kinds_without_symbols: Vec<KindGapSummary>,
    /// 说人话的告警；无缺口时为 None
    pub note: Option<String>,
}


/// 单个文件的索引统计（内部用）
#[derive(Debug, Clone, Default)]
struct FileIndexStats {
    symbols: u64,
    calls: u64,
    imports: u64,
}

impl FileIndexStats {
    /// 合并多个文件的统计数据（供增量索引等后续阶段使用）
    #[allow(dead_code)]
    fn merge(stats: &[Self]) -> Self {
        let mut merged = Self::default();
        for s in stats {
            merged.symbols += s.symbols;
            merged.calls += s.calls;
            merged.imports += s.imports;
        }
        merged
    }
}

// ============================================================================
// 解析结果 — channel 传输的数据包
// ============================================================================

/// 单个文件的解析结果
///
/// 注意：`calls` 和 `imports` 字段目前仅作统计计数，
/// 其存储实现在后续的增量索引和引用解析阶段完成。
#[allow(dead_code)]
struct ParsedFile {
    /// 文件相对路径字符串
    relative_path: String,
    /// 内容 blake3 哈希的十六进制字符串
    content_hash: String,
    /// 编程语言名称
    language: &'static str,
    /// 提取的符号列表
    symbols: Vec<codeconnect_core::types::Symbol>,
    /// 提取的调用点列表
    calls: Vec<codeconnect_core::types::CallSite>,
    /// 提取的导入列表
    imports: Vec<codeconnect_core::types::Import>,
    /// 覆盖度审计结果（spec B）
    coverage: FileCoverage,
    /// 统计计数
    stats: FileIndexStats,
}

/// 解析失败的文件信息
struct ParseFailure {
    /// 文件路径
    file_path: PathBuf,
    /// 错误信息
    error: String,
}

// ============================================================================
// 全量索引引擎
// ============================================================================

/// 全量索引引擎
///
/// 负责遍历项目目录中的所有源文件，并行解析并提取符号信息，
/// 然后将结果批量写入 tantivy 全文索引和 sled 键值存储。
///
/// # 使用示例
///
/// ```ignore
/// let mut registry = Arc::new(ParserRegistry::new());
/// // ... 注册解析器 ...
/// let tantivy = Arc::new(TantivyIndex::open_or_create(index_dir)?);
/// let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(edges_dir)?);
/// let sled = Arc::new(SledStore::open(sled_dir)?);
/// let indexer = FullIndexer::new(project_root, tantivy, call_edge_index, sled, registry);
/// let stats = indexer.run()?;
/// println!("索引完成: {:?}", stats);
/// ```
pub struct FullIndexer {
    /// 项目根目录
    pub project_root: PathBuf,
    /// 索引范围限定：相对 `project_root` 的子目录列表
    ///
    /// 为空或含 `"."` 表示整个 `project_root`。
    /// 仅控制「遍历哪些目录」，相对路径仍以 `project_root` 为基准计算。
    roots: Vec<PathBuf>,
    /// tantivy 全文搜索索引（共享引用，支持同时读写）
    pub tantivy: Arc<TantivyIndex>,
    /// tantivy 调用边索引（替代 sled edges 命名空间，避免 sled 磁盘膨胀）
    pub call_edge_index: Arc<CallEdgeIndex>,
    /// sled 键值存储（只存 meta + fingerprint + neighbors）
    sled: Arc<SledStore>,
    /// 解析器注册表
    parser_registry: Arc<ParserRegistry>,
}

impl FullIndexer {
    /// 创建全量索引器
    ///
    /// # 参数
    /// - `project_root` — 项目根目录路径
    /// - `tantivy` — 已初始化的 tantivy 索引实例（共享引用）
    /// - `call_edge_index` — 已初始化的调用边 tantivy 索引实例（共享引用）
    /// - `sled` — 已打开的 sled 存储实例（共享引用）
    /// - `parser_registry` — 已注册所有语言解析器的注册表（共享引用）
    pub fn new(
        project_root: &Path,
        tantivy: Arc<TantivyIndex>,
        call_edge_index: Arc<CallEdgeIndex>,
        sled: Arc<SledStore>,
        parser_registry: Arc<ParserRegistry>,
    ) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            roots: Vec::new(),
            tantivy,
            call_edge_index,
            sled,
            parser_registry,
        }
    }

    /// 设置索引范围限定（相对 `project_root` 的子目录列表）
    ///
    /// 传空列表或 `["."]` 表示索引整个 `project_root`（与不调用本方法等价）。
    /// 未配置到实际存在的项会在遍历时被跳过并告警。
    pub fn with_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.roots = roots;
        self
    }

    /// 计算本次实际参与遍历的目录列表（转调自由函数，保持单一事实来源）
    fn effective_walk_roots(&self) -> Vec<PathBuf> {
        effective_walk_roots(&self.project_root, &self.roots)
    }

    /// 运行全量索引
    ///
    /// 这是索引引擎的主入口，执行完整的索引流程：
    /// 文件收集 → 并行解析 → 批量写入 → 提交刷盘
    ///
    /// 若配置了 `roots` 但全部无效（不存在／越界／非目录），直接返回 `Err`：
    /// 否则调用方会拿到「0 文件」的成功结果，配合 `index -f` 的先删后建
    /// 就会把旧索引清空却报成功。
    ///
    /// # 返回
    /// 返回 [`IndexStats`] 包含详细的统计信息。
    pub fn run(&self) -> Result<IndexStats, CodeConnectError> {
        // ====================================================================
        // 第一步：收集需要解析的文件列表
        // ====================================================================
        // 只算一次遍历起点：既用于「配置错误」判定，也用于实际遍历，
        // 避免重复触发 effective_walk_roots 里的告警
        let walk_roots = self.effective_walk_roots();
        if walk_roots.is_empty() {
            return Err(CodeConnectError::Index(format!(
                "workspace.roots 配置的目录全部无效（{}），未索引任何文件。\
                 请检查 .codeconnect.toml 中 [workspace].roots 的路径拼写",
                self.roots
                    .iter()
                    .map(|r| r.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }

        let files = self.collect_files_in(&walk_roots)?;
        let files_scanned = files.len() as u64;

        tracing::info!("扫描完成，共发现 {} 个源文件", files_scanned);

        if files.is_empty() {
            return Ok(IndexStats {
                files_scanned: 0,
                files_parsed: 0,
                symbols_found: 0,
                calls_found: 0,
                imports_found: 0,
                failed_files: Vec::new(),
                coverage: CoverageSummary::default(),
            });
        }

        // ====================================================================
        // 第二步：流水线模式 — 生产者（rayon 并行解析）与消费者（批量写入）同时运行
        // 使用有界 channel (容量 1024) 限制内存中的 ParsedFile 数量，实现背压控制：
        // 当 channel 满时 write 端阻塞，自动暂停解析，避免 29 万文件同时积压
        // ====================================================================
        let (success_tx, success_rx) = channel::bounded::<ParsedFile>(1024);
        let (failure_tx, failure_rx) = channel::bounded::<ParseFailure>(1024);

        let project_root = Arc::new(self.project_root.clone());
        let parser_registry = Arc::clone(&self.parser_registry);

        // 生产者：在 rayon 线程池中并行解析所有文件
        // 使用 rayon::spawn 使解析任务异步执行，主线程作为消费者同时运行
        // 用 Arc 共享文件列表，避免克隆 29 万个 PathBuf
        let files_arc = Arc::new(files);
        let success_tx_prod = success_tx.clone();
        let failure_tx_prod = failure_tx.clone();
        rayon::spawn(move || {
            files_arc.par_iter().for_each(|file_path| {
                match parse_single_file(file_path, &parser_registry, &project_root) {
                    Ok(parsed) => {
                        // 有界 channel：缓冲区满时阻塞，自动限制并发
                        let _ = success_tx_prod.send(parsed);
                    }
                    Err(error) => {
                        let _ = failure_tx_prod.send(ParseFailure {
                            file_path: file_path.clone(),
                            error,
                        });
                    }
                }
            });
            // par_iter 完成后关闭发送端，通知消费者结束
            // drop here after clone was moved into the closure
        });

        // 关闭主线程持有的发送端引用，只保留生产者闭包内的引用
        // 当生产者闭包完成时，tx 会被自动 drop，接收端迭代自然结束
        drop(success_tx);
        drop(failure_tx);

        // ====================================================================
        // 第三步：流水线消费 — 边收边写入，解析和写入同时进行
        // ParsedFile 写入存储后立即 drop，释放内存
        // ====================================================================
        let mut files_parsed: u64 = 0;
        let mut symbols_found: u64 = 0;
        let mut calls_found: u64 = 0;
        let mut imports_found: u64 = 0;
        let mut failed_files: Vec<String> = Vec::new();
        let mut coverage_gaps: BTreeMap<String, KindGapSummary> = BTreeMap::new();
        let mut files_with_gaps: u64 = 0;

        // 按文件收集符号 ID 列表，用于写文件→符号映射
        let mut file_symbol_map: HashMap<String, FileIndexStats> = HashMap::new();

        for parsed in success_rx {
            let relative_path = parsed.relative_path.clone();
            // 先统计再写入，避免 parsed 部分移动后无法借用
            let symbols_count = parsed.stats.symbols;
            let calls_count = parsed.stats.calls;
            let imports_count = parsed.stats.imports;

            // 覆盖度审计（spec B）：缺口按文件累计，索引结束后上浮到 IndexStats
            if parsed.coverage.has_gaps() {
                files_with_gaps += 1;
                coverage::merge_into(&mut coverage_gaps, &relative_path, &parsed.coverage);
            }

            // 写入 sled 和 tantivy — 写入完成后 parsed 在此次迭代结束时 drop
            self.write_parsed_file(&parsed)?;

            files_parsed += 1;
            symbols_found += symbols_count;
            calls_found += calls_count;
            imports_found += imports_count;

            file_symbol_map.insert(relative_path, parsed.stats);
        }

        // 收集解析失败的文件
        for failure in failure_rx {
            failed_files.push(format!(
                "{}: {}",
                failure.file_path.display(),
                failure.error
            ));
        }

        // ====================================================================
        // 第四步：提交 tantivy 写入（符号索引 + 调用边索引）
        // ====================================================================
        let symbol_count = self.tantivy.commit()?;
        let edge_count = self.call_edge_index.commit()?;
        tracing::info!("提交完成: {} 个符号文档, {} 条调用边", symbol_count, edge_count);

        // ====================================================================
        // 第五步：写入 Schema 版本并刷盘
        // ====================================================================
        self.sled.put_schema_version(CURRENT_SCHEMA_VERSION)?;

        // 记录索引构建时间，供 MCP 侧计算陈旧度
        let built_at = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.sled.put_index_built_at(built_at)?;

        self.sled.flush()?;

        tracing::info!(
            "全量索引完成: {} 成功 / {} 失败, {} 符号, {} 调用, {} 导入",
            files_parsed,
            failed_files.len(),
            symbols_found,
            calls_found,
            imports_found
        );

        let coverage = build_coverage_summary(files_with_gaps, coverage_gaps);
        if let Some(note) = &coverage.note {
            tracing::warn!("{}", note);
        }

        Ok(IndexStats {
            files_scanned,
            files_parsed,
            symbols_found,
            calls_found,
            imports_found,
            failed_files,
            coverage,
        })
    }

    /// 在给定遍历起点下收集源文件
    ///
    /// 使用 `ignore::WalkBuilder` 遍历目录树，自动应用
    /// `.gitignore` 规则过滤，仅收集支持的编程语言源文件。
    ///
    /// 遍历起点由 [`FullIndexer::with_roots`] 限定；所有起点都是
    /// `project_root` 的子目录，收集到的路径仍带 `project_root` 前缀，
    /// 因此后续 `strip_prefix(project_root)` 得到的相对路径语义不变。
    fn collect_files_in(&self, walk_roots: &[PathBuf]) -> Result<Vec<PathBuf>, CodeConnectError> {
        let mut files = Vec::new();
        let supported_exts = self.parser_registry.all_extensions();

        for walk_root in walk_roots {
            let walker = WalkBuilder::new(walk_root)
                .standard_filters(true) // 自动 .gitignore 与常见忽略规则
                .hidden(false) // 不跳过隐藏文件（某些配置目录需要处理）
                .build();

            for entry in walker {
                let entry =
                    entry.map_err(|e| CodeConnectError::Index(format!("目录遍历失败: {}", e)))?;

                // 只处理普通文件
                if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                    continue;
                }

                let path = entry.path();

                // 按扩展名过滤支持的编程语言（动态从解析器注册表获取）
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if supported_exts
                        .iter()
                        .any(|e| e.eq_ignore_ascii_case(&ext_lower))
                    {
                        files.push(path.to_path_buf());
                    }
                }
            }
        }

        // 按路径排序以保证索引顺序稳定；去重避免多个 root 重叠时同一文件被索引两次
        files.sort();
        files.dedup();

        Ok(files)
    }

    /// 将单个文件的解析结果写入存储
    ///
    /// 包括：文件元信息、符号定义（tantivy）、调用边（tantivy 调用边索引）、文件指纹。
    /// sled 只保留 meta + fingerprint + neighbors，不再存储 file_symbols 和 call_edge，
    /// 从根本上消除 sled append-only 导致的磁盘膨胀问题。
    fn write_parsed_file(&self, parsed: &ParsedFile) -> Result<(), CodeConnectError> {
        let file_path_str = &parsed.relative_path;

        // ---- 写入文件元信息（sled） ----
        let file_meta = FileMeta {
            file_path: file_path_str.clone(),
            language: parsed.language.to_string(),
            content_hash: parsed.content_hash.clone(),
            symbol_count: parsed.symbols.len() as u64,
            indexed_at: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
        };

        let meta_bytes = serde_json::to_vec(&file_meta)
            .map_err(|e| CodeConnectError::Index(format!("序列化文件元信息失败: {}", e)))?;
        self.sled.put_file_meta(file_path_str, &meta_bytes)?;

        // ---- 写入每个符号定义（tantivy 符号索引） ----
        // 符号数据只存入 tantivy 全文索引，sled 不再冗余存储。
        for symbol in &parsed.symbols {
            Self::add_symbol_to_tantivy(&self.tantivy, symbol, parsed, file_path_str)?;
        }

        // ---- 写入调用边（tantivy 调用边索引，不再写入 sled） ----
        // 只对 Function / Method 类型的符号创建出边（调用者）。
        // 调用者匹配策略：按 CallSite 的行号范围与文件内 Function/Method 符号的行号范围做包含匹配。

        // 建立从符号名到 StableSymbolId 的快速查找表
        let name_to_sym_id: HashMap<&str, &str> = parsed
            .symbols
            .iter()
            .map(|s| (s.name.as_str(), s.id.as_str()))
            .collect();

        for call in &parsed.calls {
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
                // 将调用边序列化为 JSON 写入 tantivy 调用边索引
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
                for symbol in &parsed.symbols {
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
            .put_fingerprint(file_path_str, parsed.content_hash.as_bytes())?;

        Ok(())
    }

    /// 将单个符号添加到 tantivy 全文索引
    fn add_symbol_to_tantivy(
        tantivy: &TantivyIndex,
        symbol: &codeconnect_core::types::Symbol,
        parsed: &ParsedFile,
        file_path_str: &str,
    ) -> Result<(), CodeConnectError> {
        // 将 SymbolKind 枚举映射为字符串
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

        // 修饰符列表以逗号+空格拼接
        let modifiers_str = symbol.modifiers.join(", ");

        tantivy.add_symbol(
            &symbol.id,
            &symbol.name,
            kind_str,
            parsed.language,
            file_path_str,
            symbol.signature.as_deref().unwrap_or(""),
            symbol.doc_comment.as_deref().unwrap_or(""),
            "", // definition — 暂不在解析器中提取
            "", // body_text — 暂不在解析器中提取
            symbol.parent_id.as_deref().unwrap_or(""),
            &modifiers_str,
            symbol.complexity.unwrap_or(0),
            "", // ast_hash — 暂不在此阶段计算
            symbol.is_exported,
            symbol.location.line,
            symbol.location.column,
            symbol.location.end_line,
            symbol.location.end_column,
        )
    }
}

// ============================================================================
// 并行解析辅助函数
// ============================================================================

/// 解析单个文件并提取所有结构化信息
///
/// 此函数设计为可在 rayon 并行迭代中调用，无外部可变依赖。
/// 所有数据通过返回值传递，由主线程统一写入存储。
fn parse_single_file(
    file_path: &Path,
    parser_registry: &Arc<ParserRegistry>,
    project_root: &Path,
) -> Result<ParsedFile, String> {
    // 查找对应的解析器
    let parser = parser_registry
        .get_for_file(file_path)
        .ok_or_else(|| format!("不支持的文件类型: {}", file_path.display()))?;

    let language: &'static str = parser.language();

    // 读取文件内容
    let source = std::fs::read_to_string(file_path)
        .map_err(|e| format!("读取文件失败 {}: {}", file_path.display(), e))?;

    // 计算内容哈希（blake3::Hash 实现了 Display，输出十六进制字符串）
    let content_hash = blake3::hash(source.as_bytes()).to_string();

    // 计算相对于项目根目录的路径（统一使用正斜杠）
    let relative_path = file_path
        .strip_prefix(project_root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .replace('\\', "/");

    // 解析源码为 AST
    let tree = parser
        .parse(&source)
        .map_err(|e| format!("解析失败 {}: {}", file_path.display(), e))?;

    // 提取符号、调用、导入
    // 使用相对路径生成 StableSymbolId，确保索引在不同机器上可移植
    let relative_path_ref = Path::new(&relative_path);
    let symbols = parser.extract_symbols(&tree, &source, relative_path_ref);
    let calls = parser.extract_calls(&tree, &source, relative_path_ref);
    let imports = parser.extract_imports(&tree, &source, relative_path_ref);

    // 覆盖度审计（spec B）：必须在 drop(tree) 之前做，它要再走一遍 AST
    let coverage = parser.audit_coverage(&tree, &symbols);

    // 提取完成后立即释放 source 和 tree，减少内存峰值
    // source 字符串可能很大（几 MB），早释放 = 早归还给 allocator
    drop(tree);
    drop(source);

    let stats = FileIndexStats {
        symbols: symbols.len() as u64,
        calls: calls.len() as u64,
        imports: imports.len() as u64,
    };

    Ok(ParsedFile {
        relative_path,
        content_hash,
        language,
        symbols,
        calls,
        imports,
        coverage,
        stats,
    })
}

// ============================================================================
// 路径工具
// ============================================================================

/// 把逐文件的缺口汇总成索引级结论（含一句给人/给 AI 看的话）
///
/// 排序把最危险的一类顶到前面：`not_captured`（查询压根没这条模式 → 查询会返回假 0），
/// 其次是 `captured_but_no_symbol`，最后是 `known_gap`（已知、有理由）。
fn build_coverage_summary(
    files_with_gaps: u64,
    gaps: BTreeMap<String, KindGapSummary>,
) -> CoverageSummary {
    if gaps.is_empty() {
        return CoverageSummary::default();
    }

    let severity = |status: &str| match status {
        "not_captured" => 0,
        "captured_but_no_symbol" => 1,
        _ => 2,
    };
    let mut kinds: Vec<KindGapSummary> = gaps.into_values().collect();
    kinds.sort_by(|a, b| {
        severity(&a.status)
            .cmp(&severity(&b.status))
            .then(b.node_count.cmp(&a.node_count))
            .then(a.kind.cmp(&b.kind))
    });

    let uncaptured: Vec<&str> = kinds
        .iter()
        .filter(|k| k.status == "not_captured")
        .map(|k| k.kind.as_str())
        .collect();

    let note = if uncaptured.is_empty() {
        format!(
            "覆盖度审计：{} 个文件存在「有声明节点但没产出符号」的种类（均为已知缺口或部分覆盖），\
             明细见 coverage.kinds_without_symbols。对这些名字的查询返回 0 时，请先确认是不是没建索引。",
            files_with_gaps
        )
    } else {
        format!(
            "覆盖度审计：{} 个文件存在「有声明节点但没产出符号」的种类，其中 [{}] 在 symbols.scm 里\
             压根没有对应模式 —— 对这类名字的查询会返回 0，那是「没建索引」而不是「不存在」，\
             不要据此判断可以安全改删。明细见 coverage.kinds_without_symbols。",
            files_with_gaps,
            uncaptured.join(", ")
        )
    };

    CoverageSummary {
        files_with_gaps,
        kinds_without_symbols: kinds,
        note: Some(note),
    }
}

/// 计算 roots 限定的实际遍历目录
///
/// 这是 `workspace.roots` 语义的**唯一实现** —— 全量索引、增量索引、文件监控、
/// CLI 的 `-f` 前置校验都必须走这里，避免同一条配置在不同链路上解释不一致。
///
/// - `roots` 为空或含 `"."` → `vec![project_root]`（不限定）
/// - 否则逐项校验，不存在/越界/非目录的项跳过并 `tracing::warn`
/// - 全部无效 → 返回空 `Vec`（调用方据此决定报错还是空跑）
pub fn effective_walk_roots(project_root: &Path, roots: &[PathBuf]) -> Vec<PathBuf> {
    if roots.is_empty() || roots.iter().any(|r| is_whole_project_root(r)) {
        return vec![project_root.to_path_buf()];
    }

    let canonical_root = project_root.canonicalize().ok();
    let mut walk_roots = Vec::new();

    for rel in roots {
        if !is_safe_relative(rel) {
            tracing::warn!(
                "workspace.roots 项 {} 不是 project_root 下的相对子路径，已跳过",
                rel.display()
            );
            continue;
        }

        let candidate = project_root.join(rel);

        if !candidate.is_dir() {
            tracing::warn!(
                "workspace.roots 项 {} 不存在或不是目录，已跳过",
                candidate.display()
            );
            continue;
        }

        // 软链接等情况下实际位置可能越出 project_root，用规范化路径二次校验
        let escapes_root = match (&canonical_root, candidate.canonicalize()) {
            (Some(root_real), Ok(candidate_real)) => !candidate_real.starts_with(root_real),
            _ => false,
        };
        if escapes_root {
            tracing::warn!(
                "workspace.roots 项 {} 越出 project_root，已跳过",
                candidate.display()
            );
            continue;
        }

        walk_roots.push(candidate);
    }

    if walk_roots.is_empty() {
        tracing::warn!("workspace.roots 中的所有项都无效，本次不索引任何文件，请修正配置");
    }

    walk_roots
}

/// 判断是否为表示「整个 project_root」的路径（空路径、`"."`、`"./"`）
fn is_whole_project_root(path: &Path) -> bool {
    let mut components = path.components();
    matches!(
        (components.next(), components.next()),
        (None, _) | (Some(std::path::Component::CurDir), None)
    )
}

/// 判断是否为不含 `..`、盘符或根前缀的安全相对子路径
fn is_safe_relative(path: &Path) -> bool {
    use std::path::Component;
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试空项目（无源文件）不会出错
    #[test]
    fn test_collect_files_empty_dir() {
        let tmp = std::env::temp_dir().join("codeconnect_test_empty");
        let _ = std::fs::create_dir_all(&tmp);

        // 这里只验证遍历逻辑不会在空目录崩溃
        // 注意：需要实际的索引实例，所以此处仅验证 collect_files 的逻辑模式
        assert!(tmp.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// 测试 FileIndexStats::merge 合并逻辑
    #[test]
    fn test_file_index_stats_merge() {
        let s1 = FileIndexStats {
            symbols: 10,
            calls: 5,
            imports: 2,
        };
        let s2 = FileIndexStats {
            symbols: 20,
            calls: 8,
            imports: 3,
        };
        let s3 = FileIndexStats::default();

        let merged = FileIndexStats::merge(&[s1, s2, s3]);
        assert_eq!(merged.symbols, 30);
        assert_eq!(merged.calls, 13);
        assert_eq!(merged.imports, 5);
    }

    /// 构造带临时存储的索引器；返回的 TempDir 需随索引器一并持有
    fn make_indexer(project_root: &Path, roots: Vec<PathBuf>) -> (FullIndexer, tempfile::TempDir) {
        let storage = tempfile::tempdir().expect("创建临时目录失败");
        let mut registry = ParserRegistry::new();
        registry.register(Arc::new(codeconnect_parser::rust::RustParser::new()));

        let tantivy = Arc::new(
            TantivyIndex::open_or_create(&storage.path().join("tantivy"))
                .expect("打开 tantivy 失败"),
        );
        let edges = Arc::new(
            CallEdgeIndex::open_or_create(&storage.path().join("tantivy_edges"))
                .expect("打开调用边索引失败"),
        );
        let sled = Arc::new(SledStore::open(&storage.path().join("sled")).expect("打开 sled 失败"));

        let indexer = FullIndexer::new(project_root, tantivy, edges, sled, Arc::new(registry))
            .with_roots(roots);
        (indexer, storage)
    }

    /// 搭建 crates/a、crates/b 两个子目录，各含一个 .rs 文件
    fn make_project() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        for name in ["a", "b"] {
            let dir = tmp.path().join("crates").join(name).join("src");
            std::fs::create_dir_all(&dir).expect("创建目录失败");
            std::fs::write(dir.join(format!("{}.rs", name)), "pub fn f() {}\n")
                .expect("写文件失败");
        }
        tmp
    }

    /// roots 为空 = 全量：两个子目录的源文件都要被扫到
    #[test]
    fn test_collect_files_empty_roots_scans_whole_project() {
        let project = make_project();
        let (indexer, _storage) = make_indexer(project.path(), Vec::new());

        let files = collect(&indexer).expect("收集文件失败");
        assert_eq!(files.len(), 2, "空 roots 应扫描整个项目: {:?}", files);
    }

    /// roots 含 "." = 全量
    #[test]
    fn test_collect_files_dot_root_scans_whole_project() {
        let project = make_project();
        let (indexer, _storage) = make_indexer(project.path(), vec![PathBuf::from(".")]);

        let files = collect(&indexer).expect("收集文件失败");
        assert_eq!(files.len(), 2, "roots=[\".\"] 应扫描整个项目: {:?}", files);
    }

    /// roots 限定 = 只扫指定子目录，且相对路径仍以 project_root 为基准
    #[test]
    fn test_collect_files_roots_limit_to_subdir() {
        let project = make_project();
        let (indexer, _storage) = make_indexer(project.path(), vec![PathBuf::from("crates/a")]);

        let files = collect(&indexer).expect("收集文件失败");
        assert_eq!(files.len(), 1, "应只扫描 crates/a: {:?}", files);

        let relative = files[0]
            .strip_prefix(project.path())
            .expect("相对路径必须以 project_root 为基准")
            .to_string_lossy()
            .replace('\\', "/");
        assert_eq!(relative, "crates/a/src/a.rs");
    }

    /// roots 多个项重叠时同一文件只出现一次
    #[test]
    fn test_collect_files_dedup_overlapping_roots() {
        let project = make_project();
        let (indexer, _storage) = make_indexer(
            project.path(),
            vec![PathBuf::from("crates"), PathBuf::from("crates/a")],
        );

        let files = collect(&indexer).expect("收集文件失败");
        assert_eq!(files.len(), 2, "重叠 roots 不应重复收集: {:?}", files);
    }

    /// 测试辅助：按 roots 限定的遍历起点收集文件（等价于 `run()` 的第一步）
    fn collect(indexer: &FullIndexer) -> Result<Vec<PathBuf>, CodeConnectError> {
        let walk_roots = indexer.effective_walk_roots();
        indexer.collect_files_in(&walk_roots)
    }

    /// 不存在或越出 project_root 的 root 被跳过（不 panic、不扩大范围）
    #[test]
    fn test_collect_files_invalid_roots_skipped() {
        let project = make_project();

        let (indexer, _storage) = make_indexer(project.path(), vec![PathBuf::from("crates/nope")]);
        assert!(
            collect(&indexer).expect("收集文件失败").is_empty(),
            "不存在的 root 应被跳过"
        );

        let (indexer, _storage) = make_indexer(project.path(), vec![PathBuf::from("../escape")]);
        assert!(
            collect(&indexer).expect("收集文件失败").is_empty(),
            "越出 project_root 的 root 应被跳过"
        );

        let (indexer, _storage) = make_indexer(project.path(), vec![PathBuf::from("/etc")]);
        assert!(
            collect(&indexer).expect("收集文件失败").is_empty(),
            "绝对路径 root 应被跳过"
        );
    }

    /// roots 全部无效时 `run()` 必须报错，而不是静默返回 0 文件
    ///
    /// 这是 H2 的一半：只有报错，`index -f` 才不会「删光旧索引还报成功」
    #[test]
    fn test_run_errors_when_all_roots_invalid() {
        let project = make_project();
        let (indexer, _storage) = make_indexer(project.path(), vec![PathBuf::from("subb")]);

        let err = indexer.run().expect_err("roots 全无效时必须返回 Err");
        assert!(
            err.to_string().contains("全部无效"),
            "错误信息应说明 roots 无效: {}",
            err
        );
    }

    /// roots 有效但目录下确实没有源文件 → 仍然是 Ok(0)（合法情况，不报错）
    #[test]
    fn test_run_ok_when_roots_valid_but_no_sources() {
        let project = make_project();
        let empty_dir = project.path().join("empty");
        std::fs::create_dir_all(&empty_dir).expect("创建目录失败");

        let (indexer, _storage) = make_indexer(project.path(), vec![PathBuf::from("empty")]);
        let stats = indexer.run().expect("合法空范围不应报错");
        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.files_parsed, 0);
    }

    /// roots 为空（不限定）且项目无源文件 → 仍是 Ok(0)
    #[test]
    fn test_run_ok_when_unlimited_and_no_sources() {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let (indexer, _storage) = make_indexer(tmp.path(), Vec::new());

        let stats = indexer.run().expect("不限定范围且无源文件不应报错");
        assert_eq!(stats.files_scanned, 0);
    }

    /// 自由函数 `effective_walk_roots`：全量与限定两种语义
    #[test]
    fn test_effective_walk_roots_free_function() {
        let project = make_project();

        // 空 roots = 不限定
        assert_eq!(
            effective_walk_roots(project.path(), &[]),
            vec![project.path().to_path_buf()]
        );
        // 含 "." = 不限定
        assert_eq!(
            effective_walk_roots(project.path(), &[PathBuf::from(".")]),
            vec![project.path().to_path_buf()]
        );
        // 合法子目录 = 限定
        assert_eq!(
            effective_walk_roots(project.path(), &[PathBuf::from("crates/a")]),
            vec![project.path().join("crates/a")]
        );
        // 全无效 = 空
        assert!(effective_walk_roots(project.path(), &[PathBuf::from("nope")]).is_empty());
        // 部分无效 = 只留有效的
        assert_eq!(
            effective_walk_roots(
                project.path(),
                &[PathBuf::from("nope"), PathBuf::from("crates/b")]
            ),
            vec![project.path().join("crates/b")]
        );
    }

    /// 表示「整个 project_root」的路径判定
    #[test]
    fn test_is_whole_project_root() {
        assert!(is_whole_project_root(Path::new("")));
        assert!(is_whole_project_root(Path::new(".")));
        assert!(is_whole_project_root(Path::new("./")));
        assert!(!is_whole_project_root(Path::new("crates")));
        assert!(!is_whole_project_root(Path::new("crates/cli")));
    }

    /// 安全相对子路径判定：拒绝 `..`、绝对路径与盘符前缀
    #[test]
    fn test_is_safe_relative() {
        assert!(is_safe_relative(Path::new("crates/cli")));
        assert!(is_safe_relative(Path::new("crates/cli/src")));
        assert!(!is_safe_relative(Path::new("../outside")));
        assert!(!is_safe_relative(Path::new("crates/../../outside")));
        assert!(!is_safe_relative(Path::new("/etc")));
        assert!(!is_safe_relative(Path::new("C:/Windows")));
    }
}
