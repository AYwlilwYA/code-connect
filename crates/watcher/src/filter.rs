//! 文件变更过滤器
//!
//! 提供 .gitignore 规则过滤和文件扩展名过滤，
//! 确保文件监控事件仅处理相关的源代码文件。

use std::path::Path;

/// 支持的源文件扩展名列表
///
/// 覆盖 CodeConnect 支持的所有编程语言：
/// - Rust: `.rs`
/// - TypeScript: `.ts`, `.tsx`
/// - JavaScript: `.js`, `.jsx`, `.mjs`, `.cjs`
/// - Java: `.java`
/// - C#: `.cs`
/// - Kotlin: `.kt`, `.kts`
/// - Python: `.py`
/// - Go: `.go`
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs",
    "java", "cs", "kt", "kts", "py", "go",
];

/// 判断文件是否应当被监控
///
/// # 检查逻辑
///
/// 1. **扩展名检查** — 文件扩展名必须在 [`SUPPORTED_EXTENSIONS`] 列表中
/// 2. **排除规则检查** — 使用 `ignore::gitignore::Gitignore` 匹配排除模式，
///    匹配到排除模式的文件会被过滤掉
///
/// # 参数
///
/// - `path` — 待检查的文件路径
/// - `excludes` — 排除模式列表（glob 格式，如 `**/node_modules/**`）
///
/// # 返回
///
/// 返回 `true` 表示该文件应当被监控，`false` 表示应当跳过。
pub fn should_watch(path: &Path, excludes: &[String]) -> bool {
    // ---- 第一步：检查扩展名 ----
    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => ext,
        None => return false, // 无扩展名的文件直接跳过
    };

    if !SUPPORTED_EXTENSIONS.contains(&ext) {
        return false;
    }

    // ---- 第二步：检查排除规则 ----
    // 使用 ignore crate 的 Gitignore 匹配器
    // 将 exclude 模式列表构造为 Gitignore 对象
    if excludes.is_empty() {
        return true;
    }

    // 构造临时 Gitignore 匹配器
    // GitignoreBuilder 允许从字符串添加模式
    let mut builder = ignore::gitignore::GitignoreBuilder::new("/");
    for exclude in excludes {
        // ignore crate 会自动忽略以 ! 开头的非否定模式时的行为差异，
        // 这里排除模式只支持否定（不支持 re-include）
        let _ = builder.add_line(None, exclude);
    }

    let gitignore = match builder.build() {
        Ok(gi) => gi,
        Err(_) => return true, // 构建失败时保守地允许通过
    };

    // matched 返回 Match::None 表示不匹配（应该被监控）
    // 返回 Match::Ignore 表示匹配到排除规则（应该被过滤）
    !gitignore.matched(path, path.is_dir()).is_ignore()
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_watch_supported_extensions() {
        // 支持的扩展名应返回 true
        assert!(should_watch(Path::new("src/main.rs"), &[]));
        assert!(should_watch(Path::new("app.ts"), &[]));
        assert!(should_watch(Path::new("component.tsx"), &[]));
        assert!(should_watch(Path::new("lib.js"), &[]));
        assert!(should_watch(Path::new("module.mjs"), &[]));
        assert!(should_watch(Path::new("Foo.java"), &[]));
        assert!(should_watch(Path::new("Bar.cs"), &[]));
        assert!(should_watch(Path::new("foo.py"), &[]));
        assert!(should_watch(Path::new("main.go"), &[]));
    }

    #[test]
    fn test_should_watch_unsupported_extensions() {
        // 不支持的扩展名应返回 false
        assert!(!should_watch(Path::new("README.md"), &[]));
        assert!(!should_watch(Path::new("config.toml"), &[]));
        assert!(!should_watch(Path::new("Makefile"), &[]));
        assert!(!should_watch(Path::new("image.png"), &[]));
    }

    #[test]
    fn test_should_watch_no_extension() {
        // 无扩展名的文件直接返回 false
        assert!(!should_watch(Path::new("Dockerfile"), &[]));
        assert!(!should_watch(Path::new("LICENSE"), &[]));
    }

    #[test]
    fn test_should_watch_with_excludes() {
        // node_modules 内的 .ts 文件应被过滤
        let excludes = vec!["**/node_modules/**".to_string()];
        assert!(!should_watch(
            Path::new("node_modules/react/index.js"),
            &excludes
        ));

        // target 目录内的 .rs 文件应被过滤
        let excludes = vec!["**/target/**".to_string()];
        assert!(!should_watch(Path::new("target/debug/build.rs"), &excludes));

        // 普通源文件应不被过滤
        assert!(should_watch(Path::new("src/main.rs"), &excludes));
    }

    #[test]
    fn test_should_watch_multiple_excludes() {
        let excludes = vec![
            "**/node_modules/**".to_string(),
            "**/target/**".to_string(),
            "**/dist/**".to_string(),
        ];

        assert!(!should_watch(Path::new("node_modules/pkg/index.js"), &excludes));
        assert!(!should_watch(Path::new("target/debug/lib.rs"), &excludes));
        assert!(!should_watch(Path::new("dist/app.js"), &excludes));
        assert!(should_watch(Path::new("src/lib.rs"), &excludes));
    }

    #[test]
    fn test_should_watch_empty_excludes() {
        // 空排除列表，只要是支持的扩展名都应该通过
        assert!(should_watch(Path::new("any/path/main.rs"), &[]));
        assert!(!should_watch(Path::new("any/path/readme.md"), &[]));
    }
}
