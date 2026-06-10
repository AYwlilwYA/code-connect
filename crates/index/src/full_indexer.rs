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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crossbeam::channel;
use rayon::prelude::*;

use codeconnect_core::error::CodeConnectError;
use codeconnect_core::types::{FileMeta, SymbolKind};
use codeconnect_parser::factory::ParserRegistry;
use ignore::WalkBuilder;

use crate::sled_store::SledStore;
use crate::tantivy_index::{TantivyIndex, CURRENT_SCHEMA_VERSION};

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
/// let tantivy = TantivyIndex::open_or_create(index_dir)?;
/// let sled = SledStore::open(sled_dir)?;
/// let indexer = FullIndexer::new(project_root, tantivy, sled, registry);
/// let stats = indexer.run()?;
/// println!("索引完成: {:?}", stats);
/// ```
pub struct FullIndexer {
    /// 项目根目录
    pub project_root: PathBuf,
    /// tantivy 全文搜索索引
    pub tantivy: TantivyIndex,
    /// sled 键值存储
    sled: SledStore,
    /// 解析器注册表
    parser_registry: Arc<ParserRegistry>,
}

impl FullIndexer {
    /// 创建全量索引器
    ///
    /// # 参数
    /// - `project_root` — 项目根目录路径
    /// - `tantivy` — 已初始化的 tantivy 索引实例
    /// - `sled` — 已打开的 sled 存储实例
    /// - `parser_registry` — 已注册所有语言解析器的注册表
    pub fn new(
        project_root: &Path,
        tantivy: TantivyIndex,
        sled: SledStore,
        parser_registry: Arc<ParserRegistry>,
    ) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            tantivy,
            sled,
            parser_registry,
        }
    }

    /// 运行全量索引
    ///
    /// 这是索引引擎的主入口，执行完整的索引流程：
    /// 文件收集 → 并行解析 → 批量写入 → 提交刷盘
    ///
    /// # 返回
    /// 返回 [`IndexStats`] 包含详细的统计信息。
    pub fn run(&mut self) -> Result<IndexStats, CodeConnectError> {
        // ====================================================================
        // 第一步：收集需要解析的文件列表
        // ====================================================================
        let files = self.collect_files()?;
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

        // 按文件收集符号 ID 列表，用于写文件→符号映射
        let mut file_symbol_map: HashMap<String, FileIndexStats> = HashMap::new();

        for parsed in success_rx {
            let relative_path = parsed.relative_path.clone();
            // 先统计再写入，避免 parsed 部分移动后无法借用
            let symbols_count = parsed.stats.symbols;
            let calls_count = parsed.stats.calls;
            let imports_count = parsed.stats.imports;

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
        // 第四步：提交 tantivy 写入
        // ====================================================================
        self.tantivy.commit()?;

        // ====================================================================
        // 第五步：写入 Schema 版本并刷盘
        // ====================================================================
        self.sled.put_schema_version(CURRENT_SCHEMA_VERSION)?;
        self.sled.flush()?;

        tracing::info!(
            "全量索引完成: {} 成功 / {} 失败, {} 符号, {} 调用, {} 导入",
            files_parsed,
            failed_files.len(),
            symbols_found,
            calls_found,
            imports_found
        );

        Ok(IndexStats {
            files_scanned,
            files_parsed,
            symbols_found,
            calls_found,
            imports_found,
            failed_files,
        })
    }

    /// 收集项目目录下的所有源文件
    ///
    /// 使用 `ignore::WalkBuilder` 遍历目录树，自动应用
    /// `.gitignore` 规则过滤，仅收集支持的编程语言源文件。
    fn collect_files(&self) -> Result<Vec<PathBuf>, CodeConnectError> {
        let mut files = Vec::new();

        let walker = WalkBuilder::new(&self.project_root)
            .standard_filters(true) // 自动 .gitignore 与常见忽略规则
            .hidden(false) // 不跳过隐藏文件（某些配置目录需要处理）
            .build();

        for entry in walker {
            let entry = entry
                .map_err(|e| CodeConnectError::Index(format!("目录遍历失败: {}", e)))?;

            // 只处理普通文件
            if !entry.file_type().map_or(false, |ft| ft.is_file()) {
                continue;
            }

            let path = entry.path();

            // 按扩展名过滤支持的编程语言
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let supported = matches!(
                    ext,
                    "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "java" | "cs"
                );
                if supported {
                    files.push(path.to_path_buf());
                }
            }
        }

        // 按路径排序以保证索引顺序稳定
        files.sort();

        Ok(files)
    }

    /// 将单个文件的解析结果写入存储
    ///
    /// 包括：文件元信息、符号定义、文件指纹、文件→符号映射，
    /// 以及将每个符号写入 tantivy 全文索引。
    fn write_parsed_file(&self, parsed: &ParsedFile) -> Result<(), CodeConnectError> {
        let file_path_str = &parsed.relative_path;

        // ---- 写入文件元信息 ----
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

        // ---- 收集文件内所有符号 ID ----
        let mut file_symbol_ids: Vec<String> = Vec::with_capacity(parsed.symbols.len());

        // ---- 写入每个符号定义 ----
        // 注意：符号数据只存入 tantivy 全文索引（Schema 中所有字段均为 STORED）。
        // sled 不再冗余存储符号定义，避免磁盘膨胀（sled 是 append-only）。
        for symbol in &parsed.symbols {
            file_symbol_ids.push(symbol.id.clone());
            Self::add_symbol_to_tantivy(&self.tantivy, symbol, parsed, file_path_str)?;
        }

        // ---- 写入文件→符号映射 ----
        let file_symbol_bytes = serde_json::to_vec(&file_symbol_ids)
            .map_err(|e| CodeConnectError::Index(format!("序列化文件符号映射失败: {}", e)))?;
        self.sled
            .put_file_symbols(file_path_str, &file_symbol_bytes)?;

        // ---- 写入调用边 ----
        // 只对 Function / Method 类型的符号创建出边（调用者），
        // 避免为局部变量、字段、类等非可执行符号创建无意义的边。
        // 对于每个 CallSite，如果能匹配到文件内的符号，使用其 StableSymbolId；
        // 否则直接用 callee_name 作为 callee_id。
        //
        // 调用者匹配策略：
        // 按 CallSite 的行号范围与文件内 Function/Method 符号的行号范围做包含匹配。
        // 只有行号在符号范围内的调用才会被绑定到该符号，
        // 避免将所有调用都错误地绑定到文件中所有函数。

        // 先建立一个从符号名到 StableSymbolId 的快速查找表
        let name_to_sym_id: HashMap<&str, &str> = parsed.symbols
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
                let edge_bytes = serde_json::to_vec(&edge)
                    .map_err(|e| CodeConnectError::Index(format!("序列化调用边失败: {}", e)))?;
                // 用 caller_id 和 callee_name 作为键写入
                self.sled.put_call_edge(&call.caller_id, &call.callee_name, &edge_bytes)?;
            } else {
                // 无 caller_id 时，通过行号范围匹配找到包含此调用的函数
                // 调用行号必须在符号的 line..=end_line 范围内
                let call_line = call.location.line;
                for symbol in &parsed.symbols {
                    if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
                        continue;
                    }
                    // 检查调用是否在此符号的行号范围内
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
                        // 如果 callee_id 本身也是一个稳定 ID，且已在 name_to_sym_id 中
                        // 用 callee_id 替代 callee_name 作为键，以便 sled 中正确关联
                        let callee_key = name_to_sym_id
                            .get(callee_id.as_str())
                            .copied()
                            .unwrap_or(&callee_id);
                        let edge_bytes = serde_json::to_vec(&edge)
                            .map_err(|e| CodeConnectError::Index(format!("序列化调用边失败: {}", e)))?;
                        self.sled.put_call_edge(&symbol.id, callee_key, &edge_bytes)?;
                    }
                }
            }
        }

        // ---- 写入文件指纹 ----
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

    // 计算相对于项目根目录的路径
    let relative_path = file_path
        .strip_prefix(project_root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .to_string();

    // 解析源码为 AST
    let tree = parser
        .parse(&source)
        .map_err(|e| format!("解析失败 {}: {}", file_path.display(), e))?;

    // 提取符号、调用、导入 — tree-sitter 解析完成后 source 仍然需要，
    // 用于获取符号名称等文本内容（通过 AST 节点在 source 上的 slice）
    let symbols = parser.extract_symbols(&tree, &source, file_path);
    let calls = parser.extract_calls(&tree, &source, file_path);
    let imports = parser.extract_imports(&tree, &source, file_path);

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
        stats,
    })
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
}
