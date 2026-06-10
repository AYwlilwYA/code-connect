//! 解析器工厂
//!
//! 管理所有已注册的语言解析器，根据文件扩展名或语言名查找对应的解析器实例。
//! 使用 `Arc<dyn LanguageParser>` 实现零拷贝共享，避免每次查询都 clone 解析器。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::r#trait::LanguageParser;

/// 解析器注册表
///
/// 维护"扩展名 → 语言名"和"语言名 → 解析器"的双向映射，
/// 支持按文件路径自动检测语言。
pub struct ParserRegistry {
    /// 语言名 → 解析器实例
    parsers: HashMap<String, Arc<dyn LanguageParser>>,
    /// 文件扩展名 → 语言名
    ext_map: HashMap<String, String>,
}

impl ParserRegistry {
    /// 创建空的解析器注册表
    pub fn new() -> Self {
        Self {
            parsers: HashMap::new(),
            ext_map: HashMap::new(),
        }
    }

    /// 注册一个语言解析器
    ///
    /// 自动将解析器支持的所有文件扩展名映射到对应语言。
    pub fn register(&mut self, parser: Arc<dyn LanguageParser>) {
        let lang = parser.language().to_string();
        for ext in parser.file_extensions() {
            self.ext_map.insert(ext.to_string(), lang.clone());
        }
        self.parsers.insert(lang, parser);
    }

    /// 根据文件路径推断语言并查找对应解析器
    ///
    /// 返回 `None` 表示不支持该文件类型。
    pub fn get_for_file(&self, file_path: &Path) -> Option<&Arc<dyn LanguageParser>> {
        let ext = file_path.extension()?.to_str()?;
        let ext_lower = ext.to_lowercase();

        // TS/JS 特别处理 — 复合扩展名映射
        let lang_key = match ext_lower.as_str() {
            "ts" | "tsx" => "typescript",
            "js" | "jsx" | "mjs" | "cjs" => "javascript",
            "rs" => "rust",
            "java" => "java",
            "kt" | "kts" => "kotlin",
            "cs" => "csharp",
            "c" | "h" => "c",
            "cpp" | "hpp" | "cc" | "cxx" | "c++" | "h++" | "hh" | "hxx" => "cpp",
            other => other,
        };

        self.parsers.get(lang_key)
    }

    /// 根据语言名查找解析器
    pub fn get_for_language(&self, language: &str) -> Option<&Arc<dyn LanguageParser>> {
        self.parsers.get(language)
    }

    /// 返回所有已注册的语言名
    pub fn registered_languages(&self) -> Vec<&str> {
        self.parsers.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for ParserRegistry {
    fn default() -> Self {
        Self::new()
    }
}
