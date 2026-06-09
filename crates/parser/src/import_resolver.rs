//! 导入解析器
//!
//! 将调用点的被调用者名称解析为稳定的符号 ID。
//! 每种语言实现自己的导入解析策略：
//！ - Rust: use 语句 → mod 树 → Cargo.toml 依赖
//! - TypeScript: import/require → tsconfig paths → package.json
//! - Java: import → classpath → Maven/Gradle source roots
//!
//! 初期实现：返回同文件内符号的简单匹配，后续扩展跨文件解析。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use codeconnect_core::error::CodeConnectError;

/// 解析结果
#[derive(Debug, Clone)]
pub struct ResolutionResult {
    /// 候选目标符号 ID 列表
    pub candidates: Vec<CandidateTarget>,
    /// 解析策略描述
    pub strategy: String,
}

/// 候选目标
#[derive(Debug, Clone)]
pub struct CandidateTarget {
    /// 目标符号的稳定 ID
    pub symbol_id: String,
    /// 置信度 0.0-1.0
    pub confidence: f64,
    /// 解析路径描述（如 "import:std::io::Read → 同 crate 解析"）
    pub resolution_path: String,
}

/// 导入解析器 trait
///
/// 每种语言的解析器实现此 trait，提供跨文件调用目标解析能力。
pub trait ImportResolver: Send + Sync {
    /// 返回此解析器支持的语言名
    fn language(&self) -> &'static str;

    /// 解析调用目标
    ///
    /// * `callee_name` - 被调用者名称（简单名或部分限定名）
    /// * `caller_file` - 调用者文件路径
    /// * `imports` - 调用者文件内所有导入的映射（导入路径 → 已解析符号列表）
    fn resolve(
        &self,
        callee_name: &str,
        caller_file: &Path,
        imports: &HashMap<String, Vec<String>>,
    ) -> Result<ResolutionResult, CodeConnectError>;

    /// 尝试定位外部模块的文件路径
    ///
    /// 返回 `None` 表示无法解析（如系统库/std 库或第三方包）。
    fn locate_module(&self, module_path: &str, from_file: &Path) -> Option<PathBuf>;
}

/// Rust 导入解析器
///
/// 解析策略：
/// 1. 同文件内查找（当前模块内的符号）
/// 2. 同 crate 内查找（通过 mod 树遍历）
/// 3. use 导入的符号（来自 imports 参数）
/// 4. 外部 crate 依赖（标记为 external，不深入索引）
pub struct RustImportResolver;

impl ImportResolver for RustImportResolver {
    fn language(&self) -> &'static str {
        "rust"
    }

    fn resolve(
        &self,
        _callee_name: &str,
        _caller_file: &Path,
        _imports: &HashMap<String, Vec<String>>,
    ) -> Result<ResolutionResult, CodeConnectError> {
        // TODO: 实现完整的 Rust 调用目标解析
        Ok(ResolutionResult {
            candidates: vec![],
            strategy: "rust_stub".to_string(),
        })
    }

    fn locate_module(&self, _module_path: &str, _from_file: &Path) -> Option<PathBuf> {
        // TODO: 实现模块文件定位逻辑
        None
    }
}

/// TypeScript/JavaScript 导入解析器
///
/// 解析策略：
/// 1. 同文件内查找
/// 2. 通过 import/require 语句解析（来自 imports 参数）
/// 3. 同目录下的 index.ts / barrel export
/// 4. node_modules 中的第三方包
pub struct TypeScriptImportResolver;

impl ImportResolver for TypeScriptImportResolver {
    fn language(&self) -> &'static str {
        "typescript"
    }

    fn resolve(
        &self,
        _callee_name: &str,
        _caller_file: &Path,
        _imports: &HashMap<String, Vec<String>>,
    ) -> Result<ResolutionResult, CodeConnectError> {
        // TODO: 实现 ES module / CommonJS import 解析
        Ok(ResolutionResult {
            candidates: vec![],
            strategy: "ts_stub".to_string(),
        })
    }

    fn locate_module(&self, _module_path: &str, _from_file: &Path) -> Option<PathBuf> {
        // TODO: 根据 tsconfig.json paths 和 node_modules 规则定位文件
        None
    }
}

/// Java 导入解析器
///
/// 解析策略：
/// 1. 同文件内查找（当前类/接口的方法）
/// 2. 同包下的其他类（无需 import）
/// 3. import 语句导入的类/静态方法（来自 imports 参数）
/// 4. java.lang.* 自动导入
/// 5. classpath 上的外部 jar（标记为 external）
pub struct JavaImportResolver;

impl ImportResolver for JavaImportResolver {
    fn language(&self) -> &'static str {
        "java"
    }

    fn resolve(
        &self,
        _callee_name: &str,
        _caller_file: &Path,
        _imports: &HashMap<String, Vec<String>>,
    ) -> Result<ResolutionResult, CodeConnectError> {
        // TODO: 实现 Java import 解析
        Ok(ResolutionResult {
            candidates: vec![],
            strategy: "java_stub".to_string(),
        })
    }

    fn locate_module(&self, _module_path: &str, _from_file: &Path) -> Option<PathBuf> {
        // TODO: 按包名目录结构定位源文件（com.example.Foo → com/example/Foo.java）
        None
    }
}

/// C# 导入解析器
///
/// 解析策略：
/// 1. 同文件内查找（当前类的成员方法）
/// 2. using 语句导入的命名空间（同项目内的类型）
/// 3. 项目引用（.csproj 中的 ProjectReference）
/// 4. NuGet 包引用（标记为 external）
pub struct CSharpImportResolver;

impl ImportResolver for CSharpImportResolver {
    fn language(&self) -> &'static str {
        "csharp"
    }

    fn resolve(
        &self,
        _callee_name: &str,
        _caller_file: &Path,
        _imports: &HashMap<String, Vec<String>>,
    ) -> Result<ResolutionResult, CodeConnectError> {
        // TODO: 实现 C# using 指令解析
        Ok(ResolutionResult {
            candidates: vec![],
            strategy: "csharp_stub".to_string(),
        })
    }

    fn locate_module(&self, _module_path: &str, _from_file: &Path) -> Option<PathBuf> {
        // TODO: 根据命名空间和 .csproj 项目引用规则定位源文件
        None
    }
}
