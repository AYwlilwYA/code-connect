//! 已索引文件集内的文本扫描（文本真值回显）
//!
//! 符号索引必然覆盖不全 —— 枚举量、限定名调用、宏、间接引用等都可能不在索引里，
//! 而「索引里没有」与「代码里没有」在返回值上**完全同形**：都是 0，且不带任何警告。
//! 调用方（AI）据此下「不存在」「没有调用方」的结论，是已发生三次事故的共同根因。
//!
//! 本模块在**已索引文件集内**做纯文本扫描，把文本真值摆到符号结果旁边，
//! 让「稀疏/0 命中」不再是静默的。范围严格限定在 sled `meta:` 命名空间登记过的
//! 文件（即索引已知的文件集），**绝不遍历整个项目或磁盘** —— 全盘 grep 会打满磁盘 I/O。
//!
//! 匹配语义就是 grep：**大小写敏感的原始子串匹配，命中按「行」计**。
//! 刻意不做词边界收窄 —— 词边界会漏掉「真值」（例如 `mMyEnumValue` 这类派生使用），
//! 而漏掉真值正是本模块要消灭的失败模式；多报的位置调用方看得到原文，可以自行判断。

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use rayon::prelude::*;

use codeconnect_core::types::FileMeta;

use crate::sled_store::SledStore;

/// 单行原文片段的字符上限（超出时以匹配点为中心开窗裁剪）
pub const SNIPPET_MAX_CHARS: usize = 200;

/// 明细行数上限：超过则只给前若干条，但**必须**同时给出真实总数
pub const TEXT_DETAIL_LIMIT: usize = 50;

/// 符号命中「充足」的阈值：达到该值时只回一行汇总，不展开明细
///
/// 高频符号（如 `main`）展开会回上千行、吃爆上下文，
/// 而真正危险的是 0 命中与稀少命中 —— 阈值把它们与常见符号区分开。
pub const TEXT_SUMMARY_THRESHOLD: usize = 5;

/// 文本扫描的时间预算（毫秒）
///
/// MCP 工具必须能在可预期时间内返回，不能让单次搜索被文本扫描拖住。
/// 超预算时如实报告「未扫完」，绝不把不完整的 0 当成 0 呈现。
pub const SCAN_BUDGET_MS: u128 = 2000;

/// 单个文件的扫描体积上限（字节），超过则跳过并计数告警
pub const MAX_SCAN_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// 一条文本命中
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TextHit {
    /// 相对项目根目录的文件路径（与符号索引里的路径同口径）
    pub file_path: String,
    /// 行号（从 1 开始）
    pub line: usize,
    /// 该行原文片段（已裁剪到 [`SNIPPET_MAX_CHARS`]）
    pub snippet: String,
}

/// 文本扫描报告
#[derive(Debug, Clone)]
pub struct TextScanReport {
    /// 调用方原始的查询串（用于措辞，原样保留）
    pub query: String,
    /// 实际用于文本匹配的字符串（可能是 `query` 的末段标识符，见 [`text_needle`]）
    pub needle: String,
    /// 已索引文件集的大小
    pub files_total: usize,
    /// 实际读完并扫描的文件数
    pub files_scanned: usize,
    /// 因体积超限被跳过的文件数
    pub files_skipped_large: usize,
    /// 读取失败的文件数（多半是索引里有、磁盘上已被删）
    pub files_unreadable: usize,
    /// 含命中的文件数
    pub files_with_hits: usize,
    /// 命中总处数（按行计，**未被明细上限截断的真实总数**）
    pub total_hits: usize,
    /// 命中的前若干条明细（至多 `max_hits` 条）
    pub hits: Vec<TextHit>,
    /// 是否在时间预算内扫完了全部文件
    pub scan_complete: bool,
    /// 扫描耗时（毫秒）
    pub elapsed_ms: u64,
}

/// 触发文本真值回显的场景 —— 决定关键文案的措辞
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTruthContext {
    /// 符号索引 0 命中（`search_symbol`）
    SymbolNotIndexed,
    /// 调用图里没有调用方的 0 命中（`find_references`）
    NoCallers,
}

impl TextScanReport {
    /// 文本里是否有命中
    pub fn has_hits(&self) -> bool {
        self.total_hits > 0
    }

    /// 明细行（已按 [`TEXT_DETAIL_LIMIT`] 裁剪过）
    pub fn detail_lines(&self) -> Vec<String> {
        self.hits
            .iter()
            .map(|h| format!("  {}:{}  {}", h.file_path, h.line, h.snippet))
            .collect()
    }

    /// 一行汇总 —— 符号命中充足时使用
    pub fn summary_line(&self) -> String {
        format!(
            "文本另有 {} 个文件 {} 处出现（含注释/字符串，仅作核对参考）",
            self.files_with_hits, self.total_hits
        )
    }

    /// 覆盖率告警：扫描没扫全时必须显式说出来
    ///
    /// 不报这些，就等于把「没扫到」伪装成「没有」—— 正是本模块要消灭的失败模式。
    pub fn coverage_warning(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if !self.scan_complete {
            parts.push(format!(
                "仅扫描了 {}/{} 个已索引文件，命中数为不完全统计",
                self.files_scanned, self.files_total
            ));
        }
        if self.files_skipped_large > 0 {
            parts.push(format!(
                "跳过 {} 个超过 {}MB 的文件",
                self.files_skipped_large,
                MAX_SCAN_FILE_BYTES / (1024 * 1024)
            ));
        }
        if self.files_unreadable > 0 {
            parts.push(format!(
                "{} 个已索引文件读取失败（可能已被删除，建议 reindex）",
                self.files_unreadable
            ));
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!(
            "文本扫描未覆盖全部：{}。这些缺口里可能有命中，不能据此判断「不存在」。",
            parts.join("；")
        ))
    }

    /// 构造文本真值提示块
    ///
    /// - 符号命中充足（≥ [`TEXT_SUMMARY_THRESHOLD`]）且文本有命中 → 只回一行汇总
    /// - 符号命中稀少或为 0 → 展开 `文件:行` + 原文片段（含 §2.3 的关键文案）
    /// - 文本也没有命中 → 如实说明扫描范围，让「确实没有」与「没扫到」可区分
    ///
    /// 返回 `None` 表示无话可说（符号命中充足且文本无命中），不制造噪音。
    pub fn text_truth_notice(
        &self,
        context: TextTruthContext,
        symbol_hits: usize,
    ) -> Option<String> {
        let q = &self.query;
        let abundant = symbol_hits >= TEXT_SUMMARY_THRESHOLD;

        if !self.has_hits() {
            if abundant {
                return None;
            }
            // 0 命中场景下，扫描是否完整直接决定这条 0 能不能被采信。
            // 这里报的是**实际检索串**：说「没找到 X」而其实搜的是别的串，本身就是误导。
            return Some(if self.scan_complete {
                format!(
                    "文本检索在 {} 个已索引文件的原文里也没有找到 `{}`。{}注意：未索引的文件不在扫描范围内。",
                    self.files_scanned,
                    self.needle,
                    self.needle_note()
                )
            } else {
                format!(
                    "⚠ 文本扫描未完成（预算内只扫了 {}/{} 个已索引文件），**不能**据此判断 `{}` 不存在。",
                    self.files_scanned, self.files_total, q
                )
            });
        }

        if abundant {
            return Some(self.summary_line());
        }

        // 命中为 0 与命中稀少是两种情形，措辞必须分开 ——
        // 明明上面刚列出了符号命中，这里却说「未找到」，就成了自相矛盾的假话。
        let headline = match (context, symbol_hits) {
            (TextTruthContext::SymbolNotIndexed, 0) => format!(
                "符号索引中未找到 `{}`。但文本检索在 {} 个文件、{} 处找到了它 —— 这不代表 {} 不存在，\
                 更可能是它属于索引未覆盖的符号类别。下面列出文本命中位置；改动前请以这些位置为准。",
                q, self.files_with_hits, self.total_hits, q
            ),
            (TextTruthContext::NoCallers, 0) => format!(
                "调用图中没有 `{}` 的调用方。但文本检索在 {} 个文件、{} 处找到了它 —— 这不代表它没有调用方，\
                 更可能是调用边未覆盖这类引用（限定名调用、宏、间接调用等）。\
                 下面列出文本命中位置；改动前请以这些位置为准。",
                q, self.files_with_hits, self.total_hits
            ),
            (TextTruthContext::SymbolNotIndexed, hits) => format!(
                "符号索引只命中 {} 个，文本检索另有 {} 个文件、{} 处出现 `{}` —— \
                 稀疏命中时符号索引很可能没覆盖全，下面列出文本命中位置，改动前请一并核对。",
                hits, self.files_with_hits, self.total_hits, q
            ),
            (TextTruthContext::NoCallers, hits) => format!(
                "调用图只找到 {} 个调用方，文本检索另有 {} 个文件、{} 处出现 `{}` —— \
                 调用边覆盖不全时这个数字会偏小，下面列出文本命中位置，改动前请一并核对。",
                hits, self.files_with_hits, self.total_hits, q
            ),
        };

        let mut block = String::with_capacity(headline.len() + self.hits.len() * 96);
        block.push_str(&headline);
        let note = self.needle_note();
        if !note.is_empty() {
            block.push('\n');
            block.push_str(note.trim_end());
        }
        block.push_str("\n文本命中明细:");
        for line in self.detail_lines() {
            block.push('\n');
            block.push_str(&line);
        }
        Some(block)
    }

    /// 实际检索串与原查询串不同时的说明（相同则返回空串）
    ///
    /// 不能默默换了串去搜 —— 那会让调用方对「搜的到底是什么」产生错觉。
    fn needle_note(&self) -> String {
        if self.needle == self.query {
            String::new()
        } else {
            format!(
                "（文本检索串取 `{}`：由 `{}` 的末段标识符得到）",
                self.needle, self.query
            )
        }
    }
}

/// 从「符号引用写法」里提取用于文本检索的字面串
///
/// 调用方给的往往是**符号引用**而非文本串：`Class::method()`、`obj.foo()` 这类
/// 限定名/带括号的写法在源码里未必有对应字面量，直接拿去 grep 会得到「文本里也没有」
/// 的**假阴性** —— 而这恰恰是本模块要消灭的静默失败（事故 2 的限定名场景）。
/// 因此带限定/调用标记时退到「末段标识符」；纯标识符与自由文本一律原样使用。
pub fn text_needle(input: &str) -> String {
    let trimmed = input.trim();
    let qualified = trimmed.contains("::")
        || trimmed.contains("->")
        || trimmed.contains('.')
        || trimmed.contains('(');
    if !qualified {
        return trimmed.to_string();
    }

    // 先砍掉参数列表，再逐级取最后一个限定段
    let head = trimmed.split('(').next().unwrap_or(trimmed);
    let head = head.rsplit("->").next().unwrap_or(head);
    let head = head.rsplit("::").next().unwrap_or(head);
    let head = head.rsplit('.').next().unwrap_or(head);

    // 只保留标识符字符，避免把泛型参数、模板实参等一起带进来
    let segment: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();

    if segment.is_empty() {
        trimmed.to_string()
    } else {
        segment
    }
}

/// 从 sled 的 `meta:` 命名空间读出全部已索引文件（相对项目根目录的路径）
///
/// 这是文本扫描范围的**唯一来源**：索引已知什么，就只扫什么。
pub fn indexed_file_paths(sled: &SledStore) -> Vec<String> {
    let start = Instant::now();
    let mut paths: Vec<String> = Vec::new();
    for item in sled.scan_prefix(b"meta:") {
        let Ok((_key, value)) = item else {
            continue;
        };
        match serde_json::from_slice::<FileMeta>(&value) {
            Ok(meta) => paths.push(meta.file_path),
            Err(e) => tracing::warn!("文本扫描：解析文件元信息失败，跳过一项: {}", e),
        }
    }
    paths.sort();
    paths.dedup();
    tracing::debug!(
        "文本扫描: 已索引文件列表 {} 项，加载耗时 {}ms",
        paths.len(),
        start.elapsed().as_millis()
    );
    paths
}

/// 在「索引已知的文件集」内扫描文本 —— [`indexed_file_paths`] + [`scan_indexed_files`] 的组合入口
///
/// MCP 与 CLI 都走这一个入口，保证两条路径的范围、语义、文案完全一致。
///
/// 匹配分两步：先按**原查询串**扫（字面 grep，不会漏掉原串确实出现的情形）；
/// 原串一处都没命中且 [`text_needle`] 能推出末段标识符时，再用末段扫一遍。
/// 只能「多找到」、不会「少找到」—— 少找到正是本模块要消灭的失败模式。
///
/// 返回 `None` 表示索引侧信息缺失（已索引文件列表为空），
/// 调用方应据此当作「无文本信息」，而**不是**伪造一个「文本里也没有」。
pub fn scan_indexed_corpus(
    project_root: &Path,
    indexed_files: &[String],
    query: &str,
    max_hits: usize,
) -> Option<TextScanReport> {
    if indexed_files.is_empty() {
        return None;
    }

    let mut report = scan_indexed_files(project_root, indexed_files, query, max_hits);

    let derived = text_needle(query);
    if report.total_hits == 0 && derived != report.needle {
        report = scan_indexed_files(project_root, indexed_files, &derived, max_hits);
    }

    report.query = query.to_string();
    Some(report)
}

/// 单个文件的扫描结果
struct FileScan {
    scanned: bool,
    skipped_large: bool,
    unreadable: bool,
    hit_count: usize,
    hits: Vec<TextHit>,
}

impl FileScan {
    fn skipped() -> Self {
        Self {
            scanned: false,
            skipped_large: false,
            unreadable: false,
            hit_count: 0,
            hits: Vec::new(),
        }
    }
}

/// 在已索引文件集内扫描文本
///
/// # 参数
///
/// - `project_root` — 项目根目录（相对路径以它为基准还原）
/// - `relative_paths` — 已索引文件集（相对路径），见 [`indexed_file_paths`]
/// - `needle` — 查询串，按原始子串匹配（grep 语义，大小写敏感）
/// - `max_hits` — 明细条数上限；超出部分只计数不保留
///
/// `needle` 为空或全空白时直接返回空报告（否则每一行都会命中）。
pub fn scan_indexed_files(
    project_root: &Path,
    relative_paths: &[String],
    needle: &str,
    max_hits: usize,
) -> TextScanReport {
    let start = Instant::now();
    let empty = |needle: &str, total: usize| TextScanReport {
        query: needle.to_string(),
        needle: needle.to_string(),
        files_total: total,
        files_scanned: 0,
        files_skipped_large: 0,
        files_unreadable: 0,
        files_with_hits: 0,
        total_hits: 0,
        hits: Vec::new(),
        scan_complete: true,
        elapsed_ms: 0,
    };

    if needle.trim().is_empty() {
        return empty(needle, relative_paths.len());
    }

    let expired = AtomicBool::new(false);
    let scanned = AtomicUsize::new(0);

    // 并行扫描：单次 grep 全量源码是毫秒级工作，串行在万级文件的项目上会明显拖慢响应
    let scans: Vec<FileScan> = relative_paths
        .par_iter()
        .map(|rel| {
            if expired.load(Ordering::Relaxed) {
                return FileScan::skipped();
            }

            let scan = scan_one_file(project_root, rel, needle, max_hits);
            if scan.scanned {
                scanned.fetch_add(1, Ordering::Relaxed);
            }
            if start.elapsed().as_millis() > SCAN_BUDGET_MS {
                expired.store(true, Ordering::Relaxed);
            }
            scan
        })
        .collect();

    let mut hits: Vec<TextHit> = Vec::new();
    let mut total_hits = 0usize;
    let mut files_with_hits = 0usize;
    let mut files_skipped_large = 0usize;
    let mut files_unreadable = 0usize;
    let mut files_scanned = 0usize;

    // 按 relative_paths 的原始顺序合并，保证输出稳定可复现
    for scan in scans {
        files_scanned += usize::from(scan.scanned);
        files_skipped_large += usize::from(scan.skipped_large);
        files_unreadable += usize::from(scan.unreadable);
        if scan.hit_count > 0 {
            files_with_hits += 1;
        }
        total_hits += scan.hit_count;
        if hits.len() < max_hits {
            let room = max_hits - hits.len();
            hits.extend(scan.hits.into_iter().take(room));
        }
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    // 调试日志：文本扫描耗时是响应延迟的主要新增项，排查慢响应时直接看这一行
    tracing::debug!(
        "文本扫描: needle={:?} 扫描 {}/{} 个已索引文件，命中 {} 处（{} 个文件），耗时 {}ms，完整={}",
        needle,
        files_scanned,
        relative_paths.len(),
        total_hits,
        files_with_hits,
        elapsed_ms,
        files_scanned + files_skipped_large + files_unreadable == relative_paths.len()
    );

    TextScanReport {
        query: needle.to_string(),
        needle: needle.to_string(),
        files_total: relative_paths.len(),
        files_scanned,
        files_skipped_large,
        files_unreadable,
        files_with_hits,
        total_hits,
        hits,
        scan_complete: files_scanned + files_skipped_large + files_unreadable
            == relative_paths.len(),
        elapsed_ms,
    }
}

/// 扫描单个文件，返回该文件的命中与状态
fn scan_one_file(
    project_root: &Path,
    relative_path: &str,
    needle: &str,
    max_hits: usize,
) -> FileScan {
    let full_path = project_root.join(relative_path);

    match std::fs::metadata(&full_path) {
        Ok(meta) if meta.len() > MAX_SCAN_FILE_BYTES => {
            return FileScan {
                scanned: false,
                skipped_large: true,
                unreadable: false,
                hit_count: 0,
                hits: Vec::new(),
            };
        }
        Ok(_) => {}
        Err(_) => {
            return FileScan {
                scanned: false,
                skipped_large: false,
                unreadable: true,
                hit_count: 0,
                hits: Vec::new(),
            };
        }
    }

    // 非 UTF-8 文件用有损转换而不是跳过：跳过会变成新的静默缺口
    let bytes = match std::fs::read(&full_path) {
        Ok(b) => b,
        Err(_) => {
            return FileScan {
                scanned: false,
                skipped_large: false,
                unreadable: true,
                hit_count: 0,
                hits: Vec::new(),
            };
        }
    };
    let source = String::from_utf8_lossy(&bytes);

    let needle_chars = needle.chars().count();
    let mut hits: Vec<TextHit> = Vec::new();
    let mut hit_count = 0usize;

    for (idx, raw_line) in source.lines().enumerate() {
        let trimmed = raw_line.trim();
        let Some(byte_idx) = trimmed.find(needle) else {
            continue;
        };
        hit_count += 1;
        if hits.len() >= max_hits {
            continue;
        }
        let char_idx = trimmed[..byte_idx].chars().count();
        hits.push(TextHit {
            file_path: relative_path.to_string(),
            line: idx + 1,
            snippet: snippet_around(trimmed, char_idx, needle_chars, SNIPPET_MAX_CHARS),
        });
    }

    FileScan {
        scanned: true,
        skipped_large: false,
        unreadable: false,
        hit_count,
        hits,
    }
}

/// 取匹配点周围至多 `max_chars` 个字符的原文片段
///
/// 以匹配点为中心开窗（而不是从行首截断）—— 长行从头截会把命中点本身切掉。
/// 靠边界的匹配点贴边取，省略号也计入字符预算。
fn snippet_around(
    line: &str,
    match_char_idx: usize,
    needle_chars: usize,
    max_chars: usize,
) -> String {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= max_chars {
        return line.to_string();
    }

    // 给首尾省略号各留一格
    let budget = max_chars.saturating_sub(2).max(1);
    let needle_chars = needle_chars.max(1).min(budget);
    let half = (budget - needle_chars) / 2;

    let mut start = match_char_idx.saturating_sub(half);
    if start + budget > chars.len() {
        start = chars.len() - budget;
    }
    let end = start + budget;

    let mut out = String::with_capacity(max_chars + 3);
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with(
        query: &str,
        files_with_hits: usize,
        total_hits: usize,
        hits: Vec<TextHit>,
    ) -> TextScanReport {
        TextScanReport {
            query: query.to_string(),
            needle: query.to_string(),
            files_total: 10,
            files_scanned: 10,
            files_skipped_large: 0,
            files_unreadable: 0,
            files_with_hits,
            total_hits,
            hits,
            scan_complete: true,
            elapsed_ms: 3,
        }
    }

    fn hit(path: &str, line: usize, snippet: &str) -> TextHit {
        TextHit {
            file_path: path.to_string(),
            line,
            snippet: snippet.to_string(),
        }
    }

    #[test]
    fn test_empty_needle_returns_no_hits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        let report = scan_indexed_files(dir.path(), &["a.rs".to_string()], "  ", 50);
        assert_eq!(report.total_hits, 0);
        assert!(report.scan_complete);
    }

    #[test]
    fn test_scan_only_given_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("in.rs"), "let SENTINEL = 1;\n").unwrap();
        // 索引外的文件：不在 relative_paths 里，绝不能被扫到
        std::fs::write(dir.path().join("out.rs"), "let SENTINEL = 2;\n").unwrap();

        let report = scan_indexed_files(dir.path(), &["in.rs".to_string()], "SENTINEL", 50);
        assert_eq!(report.total_hits, 1);
        assert_eq!(report.files_with_hits, 1);
        assert_eq!(report.hits[0].file_path, "in.rs");
        assert_eq!(report.hits[0].line, 1);
    }

    #[test]
    fn test_total_hits_counts_beyond_detail_limit() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (0..120).map(|_| "NEEDLE\n").collect();
        std::fs::write(dir.path().join("a.rs"), body).unwrap();

        let report = scan_indexed_files(dir.path(), &["a.rs".to_string()], "NEEDLE", 50);
        assert_eq!(report.total_hits, 120, "必须给出真实总数");
        assert_eq!(report.hits.len(), 50, "明细只保留上限条数");
    }

    #[test]
    fn test_missing_file_counted_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let report = scan_indexed_files(dir.path(), &["gone.rs".to_string()], "x", 50);
        assert_eq!(report.files_unreadable, 1);
        assert!(report.scan_complete);
        assert!(report.coverage_warning().is_some());
    }

    #[test]
    fn test_notice_expands_for_zero_symbol_hits() {
        let report = report_with(
            "MY_ENUM_VALUE",
            2,
            15,
            vec![hit("src/a.rs", 7, "case MY_ENUM_VALUE:")],
        );
        let notice = report
            .text_truth_notice(TextTruthContext::SymbolNotIndexed, 0)
            .expect("0 命中且有文本命中时必须给文案");
        assert!(notice.contains("符号索引中未找到 `MY_ENUM_VALUE`"));
        assert!(notice.contains("文本检索在 2 个文件、15 处找到了它"));
        assert!(notice.contains("这不代表 MY_ENUM_VALUE 不存在"));
        assert!(notice.contains("改动前请以这些位置为准"));
        assert!(notice.contains("src/a.rs:7"));
    }

    #[test]
    fn test_notice_sparse_symbol_hits_does_not_claim_not_found() {
        // 符号命中稀少（但非 0）时，上面刚列过符号命中，
        // 文案再写「未找到」就是自相矛盾
        let report = report_with("compute", 1, 4, vec![hit("src/a.rs", 6, "pub fn compute()")]);
        let notice = report
            .text_truth_notice(TextTruthContext::SymbolNotIndexed, 1)
            .unwrap();
        assert!(
            !notice.contains("符号索引中未找到"),
            "稀疏命中不得谎称未找到:\n{}",
            notice
        );
        assert!(notice.contains("符号索引只命中 1 个"), "{}", notice);
        assert!(notice.contains("改动前请一并核对"), "{}", notice);
    }

    #[test]
    fn test_notice_summarizes_when_symbol_hits_abundant() {
        let report = report_with("main", 30, 400, vec![hit("src/a.rs", 1, "fn main() {")]);
        let notice = report
            .text_truth_notice(TextTruthContext::SymbolNotIndexed, 20)
            .expect("有文本命中就该有一行汇总");
        assert_eq!(notice, "文本另有 30 个文件 400 处出现（含注释/字符串，仅作核对参考）");
        assert!(!notice.contains("src/a.rs:1"), "充足命中时不得展开明细");
    }

    #[test]
    fn test_notice_absent_when_abundant_and_no_text_hits() {
        let report = report_with("main", 0, 0, vec![]);
        assert!(report
            .text_truth_notice(TextTruthContext::SymbolNotIndexed, 20)
            .is_none());
    }

    #[test]
    fn test_notice_reports_incomplete_scan_instead_of_zero() {
        let mut report = report_with("ghost", 0, 0, vec![]);
        report.scan_complete = false;
        report.files_scanned = 3;
        let notice = report
            .text_truth_notice(TextTruthContext::SymbolNotIndexed, 0)
            .unwrap();
        assert!(notice.contains("文本扫描未完成"));
        assert!(notice.contains("不能"));
    }

    #[test]
    fn test_no_callers_context_wording() {
        let report = report_with("Add", 3, 7, vec![hit("src/b.cpp", 9, "Add(x, y);")]);
        let notice = report
            .text_truth_notice(TextTruthContext::NoCallers, 0)
            .unwrap();
        assert!(notice.contains("调用图中没有 `Add` 的调用方"));
        assert!(notice.contains("这不代表它没有调用方"));
    }

    #[test]
    fn test_snippet_keeps_window_around_match() {
        let line = format!("{}NEEDLE{}", "a".repeat(300), "b".repeat(300));
        let snippet = snippet_around(&line, 300, 6, SNIPPET_MAX_CHARS);
        assert!(snippet.contains("NEEDLE"), "命中点不能被裁掉: {}", snippet);
        assert!(snippet.chars().count() <= SNIPPET_MAX_CHARS);
    }

    #[test]
    fn test_snippet_short_line_untouched() {
        assert_eq!(snippet_around("let x = 1;", 4, 1, 200), "let x = 1;");
    }

    #[test]
    fn test_text_needle_plain_identifier_unchanged() {
        assert_eq!(text_needle("MY_ENUM_VALUE"), "MY_ENUM_VALUE");
        assert_eq!(text_needle("  main  "), "main");
    }

    #[test]
    fn test_text_needle_derives_last_segment() {
        // 事故 2 的输入形态：限定名在源码里未必有对应字面量
        assert_eq!(text_needle("Class::method()"), "method");
        assert_eq!(text_needle("ns::Sub::Type"), "Type");
        assert_eq!(text_needle("obj.foo()"), "foo");
        assert_eq!(text_needle("ptr->run(a, b)"), "run");
    }

    #[test]
    fn test_text_needle_keeps_free_text() {
        // 自由文本（含空格、无限定标记）不得被改写
        assert_eq!(text_needle("parse config"), "parse config");
    }

    #[test]
    fn test_corpus_falls_back_to_derived_needle() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "let x = Zzqqxxtargetzzqqxx;\n").unwrap();

        let sled_dir = dir.path().join("sled");
        let sled = crate::sled_store::SledStore::open(&sled_dir).unwrap();
        let meta = codeconnect_core::types::FileMeta {
            file_path: "a.rs".to_string(),
            language: "rust".to_string(),
            content_hash: String::new(),
            symbol_count: 0,
            indexed_at: 0,
        };
        sled.put_file_meta("a.rs", &serde_json::to_vec(&meta).unwrap())
            .unwrap();

        // 原串 `Zzqqxxtargetzzqqxx()` 在源码里不存在，必须退到末段标识符才找得到
        let files = indexed_file_paths(&sled);
        let report =
            scan_indexed_corpus(dir.path(), &files, "Zzqqxxtargetzzqqxx()", 50).unwrap();
        assert_eq!(report.total_hits, 1, "退到末段标识符后应命中");
        assert_eq!(report.needle, "Zzqqxxtargetzzqqxx");
        assert_eq!(report.query, "Zzqqxxtargetzzqqxx()");

        let notice = report
            .text_truth_notice(TextTruthContext::SymbolNotIndexed, 0)
            .unwrap();
        assert!(
            notice.contains("这不代表 Zzqqxxtargetzzqqxx() 不存在"),
            "文案仍应报原始查询串:\n{}",
            notice
        );
        assert!(
            notice.contains("文本检索串取 `Zzqqxxtargetzzqqxx`"),
            "换串必须说清楚:\n{}",
            notice
        );
    }
}
