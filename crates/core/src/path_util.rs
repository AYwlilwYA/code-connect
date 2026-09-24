//! 路径展示相关的通用工具
//!
//! 关键词：verbatim 前缀 \\?\ UNC 路径剥离 Windows canonicalize

use std::path::{Path, PathBuf};

/// 剥离 Windows `\\?\` verbatim 前缀
///
/// `std::fs::canonicalize` 在 Windows 上返回 `\\?\F:\x` 形式的路径：
/// Win32 API 能吃，但 bash / MSYS 等工具读不了，写进日志或 MCP 响应里也很难看，
/// 而 MCP 响应中的路径是会被 AI 直接拿去 Read 的。
///
/// - `\\?\F:\x` → `F:\x`
/// - `\\?\UNC\server\share\x` → `\\server\share\x`
/// - 其它路径原样返回
///
/// 只还原前缀，**不**把反斜杠换成正斜杠（内部路径用反斜杠是正常的）；
/// 需要正斜杠的对外场景（如写进 MCP 配置的 args）请自行替换。
pub fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    // 非 UTF-8 路径不做处理，避免 to_string_lossy 造成有损改写
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };

    // UNC 必须先判：`\\?\UNC\server\share` 里 `\\?\` 之后是 `UNC\`，
    // 按普通前缀剥离会得到错误的 `UNC\server\share`
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }

    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_drive_prefix() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\F:\a\b.rs")),
            PathBuf::from(r"F:\a\b.rs")
        );
    }

    #[test]
    fn test_strip_unc_prefix() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
    }

    #[test]
    fn test_plain_path_unchanged() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"F:\a\b")),
            PathBuf::from(r"F:\a\b")
        );
        // UNC 普通形式不应被误伤
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\server\share\x")),
            PathBuf::from(r"\\server\share\x")
        );
    }
}
