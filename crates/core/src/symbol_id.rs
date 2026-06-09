//! 稳定符号 ID 系统
//!
//! 基于 blake3 指纹的稳定符号标识符，格式为：
//! `language::relative_path::kind::name::fingerprint`
//!
//! 其中 fingerprint 为 `name + path + language + kind` 串联后
//! blake3 哈希的前 8 字符（前 4 字节）。

use serde::{Deserialize, Serialize};

/// 稳定符号 ID
///
/// 由语言、路径、符号类型、名称和 blake3 指纹组成。
/// 指纹确保即使符号重命名或移动后也能通过原始属性查找到。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StableSymbolId {
    /// 编程语言（rust, typescript, java 等）
    pub language: String,
    /// 相对于项目根目录的文件路径
    pub relative_path: String,
    /// 符号类型
    pub kind: String,
    /// 符号名称
    pub name: String,
    /// blake3 指纹（8 字符十六进制）
    pub fingerprint: String,
}

impl StableSymbolId {
    /// 创建新的稳定符号 ID
    ///
    /// 自动计算 blake3 指纹。
    pub fn new(language: &str, relative_path: &str, kind: &str, name: &str) -> Self {
        let fingerprint = Self::compute_fingerprint(language, relative_path, kind, name);
        Self {
            language: language.to_string(),
            relative_path: relative_path.to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            fingerprint,
        }
    }

    /// 计算 blake3 指纹
    ///
    /// 输入格式：`name::path::language::kind`
    /// 输出：前 4 字节的十六进制表示（8 字符）
    fn compute_fingerprint(language: &str, path: &str, kind: &str, name: &str) -> String {
        let input = format!("{}::{}::{}::{}", name, path, language, kind);
        let hash = blake3::hash(input.as_bytes());
        // 取前 4 字节 = 8 个十六进制字符
        hex::encode(&hash.as_bytes()[..4])
    }

    /// 序列化为存储格式字符串
    ///
    /// 格式：`language::relative_path::kind::name::fingerprint`
    pub fn to_storage_string(&self) -> String {
        format!(
            "{}::{}::{}::{}::{}",
            self.language, self.relative_path, self.kind, self.name, self.fingerprint
        )
    }

    /// 从字符串解析稳定符号 ID
    ///
    /// 期望格式：`language::relative_path::kind::name::fingerprint`
    /// 5 个以 `::` 分隔的字段。
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split("::").collect();
        if parts.len() != 5 {
            return Err(format!("无效的符号 ID 格式，期望 5 个字段实际得到 {} 个: {}", parts.len(), s));
        }
        Ok(Self {
            language: parts[0].into(),
            relative_path: parts[1].into(),
            kind: parts[2].into(),
            name: parts[3].into(),
            fingerprint: parts[4].into(),
        })
    }
}

// ============================================================================
// 标准 trait 实现
// ============================================================================

impl std::fmt::Display for StableSymbolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_storage_string())
    }
}

impl std::str::FromStr for StableSymbolId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_create_stable_id() {
        let id = StableSymbolId::new("rust", "src/main.rs", "function", "main");
        assert_eq!(id.language, "rust");
        assert_eq!(id.relative_path, "src/main.rs");
        assert_eq!(id.kind, "function");
        assert_eq!(id.name, "main");
        assert_eq!(id.fingerprint.len(), 8);
    }

    #[test]
    fn test_fingerprint_is_stable() {
        let id1 = StableSymbolId::new("rust", "src/main.rs", "function", "main");
        let id2 = StableSymbolId::new("rust", "src/main.rs", "function", "main");
        assert_eq!(id1.fingerprint, id2.fingerprint);
    }

    #[test]
    fn test_fingerprint_differs() {
        let id1 = StableSymbolId::new("rust", "src/main.rs", "function", "main");
        let id2 = StableSymbolId::new("rust", "src/main.rs", "function", "other");
        assert_ne!(id1.fingerprint, id2.fingerprint);
    }

    #[test]
    fn test_to_string_and_parse() {
        let original = StableSymbolId::new("typescript", "src/components/App.tsx", "class", "App");
        let serialized = original.to_string();
        let parsed = StableSymbolId::from_str(&serialized).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_parse_invalid_format() {
        let result = StableSymbolId::from_str("too::few::fields");
        assert!(result.is_err());
    }
}
