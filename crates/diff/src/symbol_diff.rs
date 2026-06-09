//! 符号级差异分析
//!
//! 从文件行级 diff 映射到符号范围，识别受变更影响的符号。
//! 支持 Unified Diff 格式解析和符号范围匹配。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 变更行范围
///
/// 表示 diff hunk 中发生变更的行号范围。
/// 行号均为 1-based。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineRange {
    /// 起始行号（1-based）
    pub start: u64,
    /// 结束行号（1-based）
    pub end: u64,
}

/// 受影响的符号条目
///
/// 描述一个被行级 diff 波及的符号。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedSymbol {
    /// 符号稳定 ID
    pub symbol_id: String,
    /// 符号名称
    pub name: String,
    /// 符号所在的文件路径
    pub file_path: String,
    /// 符号在文件中的行号范围
    pub line_range: LineRange,
    /// 与 diff 变更行的重叠程度（0.0-1.0，1.0 表示完全在变更范围内）
    pub overlap_ratio: f64,
}

/// 符号范围映射
///
/// 维护文件路径到符号行号范围的映射，
/// 用于将 diff 变更行匹配到具体符号。
pub struct SymbolRangeMap {
    /// 文件路径 → (符号ID, 符号名称, 行号范围) 列表
    file_to_symbols: HashMap<String, Vec<SymbolRangeEntry>>,
}

/// 符号范围条目（内部用）
#[derive(Debug, Clone)]
struct SymbolRangeEntry {
    symbol_id: String,
    name: String,
    line_start: u64,
    line_end: u64,
}

/// 符号差异分析器
///
/// 将行级 diff 映射到符号范围，识别变更影响的符号。
pub struct SymbolDiff;

impl SymbolDiff {
    /// 从 diff 行范围映射到受影响的符号
    ///
    /// 在给定的符号范围映射中，查找与每一行 diff 变更范围有交集的所有符号。
    /// 对于每个受影响的符号，计算变更行与符号定义行的重叠比率。
    ///
    /// # 参数
    /// - `changed_lines` — 文件到变更行号范围的映射
    /// - `symbol_map` — 文件到符号行号范围的映射
    ///
    /// # 返回
    /// 所有受影响的符号列表，按重叠比率降序排列。
    pub fn map_lines_to_symbols(
        changed_lines: &HashMap<String, Vec<LineRange>>,
        symbol_map: &SymbolRangeMap,
    ) -> Vec<AffectedSymbol> {
        let mut affected = Vec::new();

        for (file_path, ranges) in changed_lines {
            // 查找该文件中的所有符号
            if let Some(symbols) = symbol_map.file_to_symbols.get(file_path) {
                for symbol in symbols {
                    // 计算每个符号与 diff 范围的重叠情况
                    let overlap_result =
                        Self::compute_overlap(ranges, symbol.line_start, symbol.line_end);

                    if overlap_result > 0.0 {
                        affected.push(AffectedSymbol {
                            symbol_id: symbol.symbol_id.clone(),
                            name: symbol.name.clone(),
                            file_path: file_path.clone(),
                            line_range: LineRange {
                                start: symbol.line_start,
                                end: symbol.line_end,
                            },
                            overlap_ratio: overlap_result,
                        });
                    }
                }
            }
        }

        // 按重叠比率降序排列（影响最大的在前）
        affected.sort_by(|a, b| {
            b.overlap_ratio
                .partial_cmp(&a.overlap_ratio)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        affected
    }

    /// 计算变更行范围与符号定义行的重叠比率
    ///
    /// 重叠比率 = 与 diff 有交集的行数 / 符号定义总行数。
    ///
    /// # 返回值
    /// 0.0 到 1.0 之间，其中 1.0 表示符号完全在变更范围内。
    fn compute_overlap(ranges: &[LineRange], symbol_start: u64, symbol_end: u64) -> f64 {
        let symbol_lines = symbol_end - symbol_start + 1;
        if symbol_lines == 0 {
            return 0.0;
        }

        // 检查符号行范围是否与任一 diff hunk 有交集
        let mut has_overlap = false;
        let mut overlap_lines: u64 = 0;

        for range in ranges {
            // 判断两个范围是否相交
            let overlap_start = range.start.max(symbol_start);
            let overlap_end = range.end.min(symbol_end);

            if overlap_start <= overlap_end {
                has_overlap = true;
                overlap_lines += overlap_end - overlap_start + 1;
            }
        }

        if !has_overlap {
            return 0.0;
        }

        // 重叠比率（上限为 1.0）
        (overlap_lines as f64 / symbol_lines as f64).min(1.0)
    }

    /// 解析 Unified Diff 文本，提取文件到变更行范围的映射
    ///
    /// 解析标准 unified diff 格式（如 `git diff` 输出），
    /// 提取每个文件的 hunks 行号范围。
    ///
    /// # 参数
    /// - `diff_text` — unified diff 格式的字符串
    ///
    /// # 返回
    /// `HashMap<文件路径, Vec<变更行范围>>`
    ///
    /// # 注意
    /// 此方法只解析新文件中变更行的范围，适合确定哪些行发生了变更。
    pub fn parse_diff_hunks(diff_text: &str) -> HashMap<String, Vec<LineRange>> {
        let mut result: HashMap<String, Vec<LineRange>> = HashMap::new();
        let mut current_file: Option<String> = None;

        for line in diff_text.lines() {
            // 匹配文件头：`diff --git a/path b/path` 或 `--- a/path`
            if line.starts_with("--- a/") {
                // 提取文件路径（去掉 `--- a/` 前缀）
                current_file = Some(line[6..].to_string());
                continue;
            }

            // 匹配 hunk 头：`@@ -old_start,old_count +new_start,new_count @@`
            if line.starts_with("@@") {
                if let Some(ref file) = current_file {
                    // 解析新文件行号信息
                    if let Some(hunk_range) = Self::parse_hunk_header(line) {
                        result
                            .entry(file.clone())
                            .or_insert_with(Vec::new)
                            .push(hunk_range);
                    }
                }
            }
        }

        result
    }

    /// 解析 hunk 头行
    ///
    /// 格式：`@@ -old_start,old_count +new_start,new_count @@`
    /// 提取新文件的起始行和行数。
    fn parse_hunk_header(header: &str) -> Option<LineRange> {
        // 查找 "+" 后的新文件信息
        let plus_pos = header.rfind('+')?;
        let after_plus = &header[plus_pos + 1..];

        // 提取数字部分（截止到下一个非数字字符或 @@）
        let end_of_num = after_plus
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after_plus.len());
        let num_part = &after_plus[..end_of_num];

        // 格式：start,count 或 start
        if let Some(comma_pos) = num_part.find(',') {
            let start: u64 = num_part[..comma_pos].parse().ok()?;
            let count: u64 = num_part[comma_pos + 1..].parse().ok()?;
            if count > 0 {
                Some(LineRange {
                    start,
                    end: start + count - 1,
                })
            } else {
                // count 为 0 表示空 hunk（如仅有删除），不影响符号匹配
                None
            }
        } else {
            // 只有起始行号（默认 count=1）
            let start: u64 = num_part.parse().ok()?;
            Some(LineRange { start, end: start })
        }
    }
}

// ============================================================================
// SymbolRangeMap 实现
// ============================================================================

impl SymbolRangeMap {
    /// 创建空的符号范围映射
    pub fn new() -> Self {
        Self {
            file_to_symbols: HashMap::new(),
        }
    }

    /// 添加一个符号的行号范围
    ///
    /// # 参数
    /// - `symbol_id` — 符号稳定 ID
    /// - `name` — 符号名称
    /// - `file_path` — 符号所在文件路径
    /// - `line_start` — 符号声明起始行号（1-based）
    /// - `line_end` — 符号声明结束行号（1-based）
    pub fn add_symbol(
        &mut self,
        symbol_id: &str,
        name: &str,
        file_path: &str,
        line_start: u64,
        line_end: u64,
    ) {
        self.file_to_symbols
            .entry(file_path.to_string())
            .or_insert_with(Vec::new)
            .push(SymbolRangeEntry {
                symbol_id: symbol_id.to_string(),
                name: name.to_string(),
                line_start,
                line_end,
            });
    }

    /// 获取映射中的文件数
    pub fn file_count(&self) -> usize {
        self.file_to_symbols.len()
    }

    /// 获取映射中的符号总数
    pub fn symbol_count(&self) -> usize {
        self.file_to_symbols
            .values()
            .map(|v| v.len())
            .sum()
    }
}

impl Default for SymbolRangeMap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // 行号范围重叠计算测试
    // ========================================================================

    #[test]
    fn test_full_overlap() {
        let ranges = vec![LineRange { start: 10, end: 20 }];
        // 符号行号 12-15 完全在变更范围内
        let ratio = SymbolDiff::compute_overlap(&ranges, 12, 15);
        assert!((ratio - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_partial_overlap() {
        let ranges = vec![LineRange { start: 10, end: 15 }];
        // 符号行号 12-20，前 4 行（12-15）与 diff 重叠
        let ratio = SymbolDiff::compute_overlap(&ranges, 12, 20);
        assert!((ratio - 4.0 / 9.0).abs() < 0.001);
    }

    #[test]
    fn test_no_overlap() {
        let ranges = vec![LineRange { start: 10, end: 15 }];
        // 符号行号 20-30，与 diff 范围无交集
        let ratio = SymbolDiff::compute_overlap(&ranges, 20, 30);
        assert_eq!(ratio, 0.0);
    }

    #[test]
    fn test_multiple_ranges() {
        let ranges = vec![
            LineRange { start: 10, end: 15 },
            LineRange { start: 30, end: 35 },
        ];
        // 符号行号 12-32，与两个 diff hunk 都有重叠
        let ratio = SymbolDiff::compute_overlap(&ranges, 12, 32);
        // 符号总行数: 21
        // 重叠行: (12-15)=4 + (30-32)=3 = 7
        assert!((ratio - 7.0 / 21.0).abs() < 0.001);
    }

    #[test]
    fn test_single_line_symbol() {
        let ranges = vec![LineRange { start: 10, end: 10 }];
        let ratio = SymbolDiff::compute_overlap(&ranges, 10, 10);
        assert!((ratio - 1.0).abs() < 0.001);
    }

    // ========================================================================
    // diff hunk 解析测试
    // ========================================================================

    #[test]
    fn test_parse_hunk_header_with_count() {
        let result = SymbolDiff::parse_hunk_header("@@ -1,5 +1,10 @@");
        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, 1);
        assert_eq!(range.end, 10);
    }

    #[test]
    fn test_parse_hunk_header_without_count() {
        let result = SymbolDiff::parse_hunk_header("@@ -5 +10 @@");
        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, 10);
        assert_eq!(range.end, 10); // 单行
    }

    #[test]
    fn test_parse_hunk_header_context() {
        // 带上下文信息的 hunk
        let result = SymbolDiff::parse_hunk_header("@@ -10,7 +10,6 @@ fn main() {");
        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.start, 10);
        assert_eq!(range.end, 15);
    }

    // ========================================================================
    // 完整 diff 解析测试
    // ========================================================================

    #[test]
    fn test_parse_diff_hunks_multi_file() {
        let diff_text = r#"diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,5 @@ fn main() {
 fn main() {
+    println!("hello");
 }

diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,0 +11,5 @@ pub fn add(a: i32, b: i32) -> i32 {
+pub fn add(a: i32, b: i32) -> i32 {
+    a + b
+}
+
"#;

        let result = SymbolDiff::parse_diff_hunks(diff_text);

        // src/main.rs 应该有一个 hunk
        assert!(result.contains_key("src/main.rs"));
        let main_ranges = &result["src/main.rs"];
        assert_eq!(main_ranges.len(), 1);
        assert_eq!(main_ranges[0].start, 1);
        assert_eq!(main_ranges[0].end, 5);

        // src/lib.rs 应该有一个 hunk
        assert!(result.contains_key("src/lib.rs"));
        let lib_ranges = &result["src/lib.rs"];
        assert_eq!(lib_ranges.len(), 1);
        assert_eq!(lib_ranges[0].start, 11);
        assert_eq!(lib_ranges[0].end, 15);
    }

    #[test]
    fn test_parse_diff_empty() {
        let result = SymbolDiff::parse_diff_hunks("");
        assert!(result.is_empty());
    }

    // ========================================================================
    // 符号范围映射测试
    // ========================================================================

    #[test]
    fn test_symbol_range_map_basic() {
        let mut map = SymbolRangeMap::new();
        assert_eq!(map.file_count(), 0);
        assert_eq!(map.symbol_count(), 0);

        map.add_symbol("id_main", "main", "src/main.rs", 1, 5);
        map.add_symbol("id_helper", "helper", "src/main.rs", 10, 15);
        map.add_symbol("id_init", "init", "src/lib.rs", 1, 3);

        assert_eq!(map.file_count(), 2); // main.rs 和 lib.rs
        assert_eq!(map.symbol_count(), 3);
    }

    #[test]
    fn test_map_lines_to_symbols() {
        // 构建符号范围映射
        let mut map = SymbolRangeMap::new();
        map.add_symbol("id_main", "main", "src/main.rs", 1, 10);
        map.add_symbol("id_helper", "helper", "src/main.rs", 20, 25);
        map.add_symbol("id_util", "util", "src/util.rs", 5, 15);

        // 构建变更行映射
        let mut changed_lines = HashMap::new();
        changed_lines.insert(
            "src/main.rs".to_string(),
            vec![
                LineRange { start: 1, end: 5 },    // 与 main 重叠
            ],
        );
        changed_lines.insert(
            "src/util.rs".to_string(),
            vec![
                LineRange { start: 10, end: 12 },  // 与 util 重叠
            ],
        );

        let affected = SymbolDiff::map_lines_to_symbols(&changed_lines, &map);

        // main 和 util 受影响
        assert_eq!(affected.len(), 2);

        let main_affected = affected.iter().find(|s| s.name == "main").unwrap();
        assert!(main_affected.overlap_ratio > 0.0);
        assert_eq!(main_affected.file_path, "src/main.rs");

        let util_affected = affected.iter().find(|s| s.name == "util").unwrap();
        assert!(util_affected.overlap_ratio > 0.0);
        assert_eq!(util_affected.file_path, "src/util.rs");

        // helper 不受影响
        assert!(affected.iter().all(|s| s.name != "helper"));
    }

    #[test]
    fn test_map_lines_to_symbols_no_overlap() {
        let mut map = SymbolRangeMap::new();
        map.add_symbol("id_main", "main", "src/main.rs", 1, 10);

        let mut changed_lines = HashMap::new();
        changed_lines.insert(
            "src/main.rs".to_string(),
            vec![LineRange { start: 50, end: 60 }],
        );

        let affected = SymbolDiff::map_lines_to_symbols(&changed_lines, &map);
        assert!(affected.is_empty());
    }

    #[test]
    fn test_map_lines_to_symbols_sorted() {
        let mut map = SymbolRangeMap::new();
        map.add_symbol("id_a", "func_a", "src/lib.rs", 1, 10);
        map.add_symbol("id_b", "func_b", "src/lib.rs", 20, 25);

        let mut changed_lines = HashMap::new();
        changed_lines.insert(
            "src/lib.rs".to_string(),
            vec![
                LineRange { start: 3, end: 5 },   // func_a 部分重叠
                LineRange { start: 20, end: 25 }, // func_b 完全重叠
            ],
        );

        let affected = SymbolDiff::map_lines_to_symbols(&changed_lines, &map);

        // 按重叠比率降序排列，func_b (1.0) 应在 func_a (0.3) 之前
        assert_eq!(affected.len(), 2);
        assert!(affected[0].overlap_ratio >= affected[1].overlap_ratio);
    }
}
