//! 配置文件解析模块
//!
//! 支持从当前目录向上查找 `.codeconnect.toml` 项目配置，
//! 并与 `~/.codeconnect/config.toml` 全局配置合并。
//!
//! 配置涵盖：工作区设置、语言支持、索引策略、搜索参数、
//! 复杂度阈值、死代码检测规则和图校验规则。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ============================================================================
// CodeConnect 主配置
// ============================================================================

/// CodeConnect 主配置结构
///
/// 所有字段都使用 `#[serde(default)]` 以支持部分覆盖，
/// 合并时缺失字段使用默认值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeConnectConfig {
    /// 工作区配置
    #[serde(default)]
    pub workspace: WorkspaceConfig,

    /// 语言支持配置
    #[serde(default)]
    pub languages: LanguagesConfig,

    /// 索引配置
    #[serde(default)]
    pub index: IndexConfig,

    /// 搜索配置
    #[serde(default)]
    pub search: SearchConfig,

    /// 复杂度阈值配置
    #[serde(default)]
    pub complexity: ComplexityConfig,

    /// 死代码检测规则
    #[serde(default)]
    pub dead_code: Vec<DeadCodeConfig>,

    /// 图校验规则
    #[serde(default)]
    pub rules: Vec<RuleConfig>,
}

impl Default for CodeConnectConfig {
    fn default() -> Self {
        Self {
            workspace: WorkspaceConfig::default(),
            languages: LanguagesConfig::default(),
            index: IndexConfig::default(),
            search: SearchConfig::default(),
            complexity: ComplexityConfig::default(),
            dead_code: Vec::new(),
            rules: Vec::new(),
        }
    }
}

// ============================================================================
// 工作区配置
// ============================================================================

/// 工作区配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// 项目根目录列表（支持 monorepo）
    #[serde(default)]
    pub roots: Vec<PathBuf>,

    /// 排除的目录模式（glob 格式）
    #[serde(default)]
    pub excludes: Vec<String>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            roots: vec![PathBuf::from(".")],
            excludes: vec![
                "**/node_modules/**".into(),
                "**/target/**".into(),
                "**/build/**".into(),
                "**/dist/**".into(),
                "**/.git/**".into(),
                "**/vendor/**".into(),
            ],
        }
    }
}

// ============================================================================
// 语言配置
// ============================================================================

/// 各语言开关配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguagesConfig {
    /// Rust 语言支持
    #[serde(default = "default_true")]
    pub rust: bool,

    /// TypeScript 语言支持
    #[serde(default = "default_true")]
    pub typescript: bool,

    /// JavaScript 语言支持
    #[serde(default = "default_true")]
    pub javascript: bool,

    /// Java 语言支持
    #[serde(default)]
    pub java: bool,

    /// Kotlin 语言支持
    #[serde(default)]
    pub kotlin: bool,

    /// C# 语言支持
    #[serde(default)]
    pub csharp: bool,
}

fn default_true() -> bool {
    true
}

impl Default for LanguagesConfig {
    fn default() -> Self {
        Self {
            rust: true,
            typescript: true,
            javascript: true,
            java: false,
            kotlin: false,
            csharp: false,
        }
    }
}

// ============================================================================
// 索引配置
// ============================================================================

/// 索引配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexConfig {
    /// 索引数据存储目录（相对于项目根目录），默认 `.codeconnect/`
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// 是否启用增量索引
    #[serde(default = "default_true")]
    pub incremental: bool,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from(".codeconnect")
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            incremental: true,
        }
    }
}

// ============================================================================
// 搜索配置
// ============================================================================

/// 搜索配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    /// 单次搜索最大返回结果数
    #[serde(default = "default_max_results")]
    pub max_results: usize,
}

fn default_max_results() -> usize {
    100
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_results: default_max_results(),
        }
    }
}

// ============================================================================
// 复杂度配置
// ============================================================================

/// 圈复杂度阈值配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityConfig {
    /// 警告阈值（超过此值产生警告）
    #[serde(default = "default_warning_threshold")]
    pub warning_threshold: u64,

    /// 错误阈值（超过此值视为代码质量问题）
    #[serde(default = "default_error_threshold")]
    pub error_threshold: u64,
}

fn default_warning_threshold() -> u64 {
    15
}

fn default_error_threshold() -> u64 {
    30
}

impl Default for ComplexityConfig {
    fn default() -> Self {
        Self {
            warning_threshold: default_warning_threshold(),
            error_threshold: default_error_threshold(),
        }
    }
}

// ============================================================================
// 死代码检测配置
// ============================================================================

/// 死代码检测配置
///
/// 定义从给定的入口点出发无法到达的代码为"死代码"。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeConfig {
    /// 入口点符号名称列表
    pub entry_points: Vec<String>,
}

// ============================================================================
// 图校验规则
// ============================================================================

/// 架构层校验规则
///
/// 定义哪些层之间允许相互依赖，用于检测架构违规。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    /// 规则名称
    pub name: String,

    /// 规则描述
    pub description: String,

    /// 参与规则的层名称列表
    pub layers: Vec<String>,

    /// 允许的依赖方向（"layerA -> layerB" 格式）
    pub allowed: Vec<String>,
}

// ============================================================================
// 配置加载函数
// ============================================================================

/// 加载配置
///
/// 从当前目录开始向上查找 `.codeconnect.toml`，找到后读取。
/// 同时尝试读取 `~/.codeconnect/config.toml` 作为全局配置基准，
/// 项目配置会覆盖全局配置。
///
/// 如果找不到任何配置文件，返回默认配置。
pub fn load_config() -> CodeConnectConfig {
    let mut config = load_global_config();

    // 从当前目录向上查找项目配置
    if let Some(project_config) = find_and_load_project_config() {
        merge_configs(&mut config, project_config);
    }

    config
}

/// 加载全局配置文件 `~/.codeconnect/config.toml`
fn load_global_config() -> CodeConnectConfig {
    let global_path = dirs_home().join(".codeconnect").join("config.toml");

    match std::fs::read_to_string(&global_path) {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => CodeConnectConfig::default(),
    }
}

/// 从当前目录向上查找并加载 `.codeconnect.toml`
fn find_and_load_project_config() -> Option<CodeConnectConfig> {
    let current_dir = std::env::current_dir().ok()?;
    let mut dir = current_dir.as_path();

    loop {
        let config_path = dir.join(".codeconnect.toml");
        if config_path.is_file() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str::<CodeConnectConfig>(&content) {
                    return Some(config);
                }
            }
        }

        // 向上一级目录
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }

    None
}

/// 将项目配置合并到基准配置中（项目配置覆盖全局配置）
fn merge_configs(base: &mut CodeConnectConfig, overlay: CodeConnectConfig) {
    // 工作区配置：合并 roots 和 excludes
    if !overlay.workspace.roots.is_empty() {
        base.workspace.roots = overlay.workspace.roots;
    }
    if !overlay.workspace.excludes.is_empty() {
        base.workspace.excludes = overlay.workspace.excludes;
    }

    // 语言配置：逐字段覆盖
    base.languages.rust = overlay.languages.rust;
    base.languages.typescript = overlay.languages.typescript;
    base.languages.javascript = overlay.languages.javascript;
    base.languages.java = overlay.languages.java;
    base.languages.kotlin = overlay.languages.kotlin;
    base.languages.csharp = overlay.languages.csharp;

    // 索引配置
    if overlay.index.data_dir != default_data_dir() {
        base.index.data_dir = overlay.index.data_dir;
    }
    base.index.incremental = overlay.index.incremental;

    // 搜索配置
    base.search.max_results = overlay.search.max_results;

    // 复杂度配置
    base.complexity.warning_threshold = overlay.complexity.warning_threshold;
    base.complexity.error_threshold = overlay.complexity.error_threshold;

    // 死代码规则：如果项目配置了，完全替换
    if !overlay.dead_code.is_empty() {
        base.dead_code = overlay.dead_code;
    }

    // 校验规则：如果项目配置了，完全替换
    if !overlay.rules.is_empty() {
        base.rules = overlay.rules;
    }
}

/// 获取用户主目录
fn dirs_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = CodeConnectConfig::default();
        assert_eq!(config.workspace.roots.len(), 1);
        assert!(config.languages.rust);
        assert!(!config.languages.java);
        assert!(config.index.incremental);
        assert_eq!(config.search.max_results, 100);
        assert_eq!(config.complexity.warning_threshold, 15);
        assert_eq!(config.complexity.error_threshold, 30);
        assert!(config.dead_code.is_empty());
        assert!(config.rules.is_empty());
    }

    #[test]
    fn test_languages_config_default() {
        let config = LanguagesConfig::default();
        assert!(config.rust);
        assert!(config.typescript);
        assert!(config.javascript);
        assert!(!config.java);
        assert!(!config.kotlin);
        assert!(!config.csharp);
    }

    #[test]
    fn test_workspace_config_default() {
        let config = WorkspaceConfig::default();
        assert_eq!(config.roots, vec![PathBuf::from(".")]);
        assert!(!config.excludes.is_empty());
    }
}
