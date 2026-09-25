//! 配置文件解析模块
//!
//! 支持从指定项目根目录（而非 cwd）向上查找 `.codeconnect.toml` 项目配置，
//! 并与 `~/.codeconnect/config.toml` 全局配置合并。
//!
//! 配置涵盖：工作区设置、语言支持、索引策略、搜索参数、
//! 复杂度阈值、死代码检测规则、图校验规则和语义检索（向量）配置。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

    /// 向量语义检索配置（可选能力）
    #[serde(default)]
    pub semantic: SemanticConfig,
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
            semantic: SemanticConfig::default(),
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

    /// C 语言支持
    #[serde(default = "default_true")]
    pub c: bool,

    /// C++ 语言支持
    #[serde(default = "default_true")]
    pub cpp: bool,
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
            java: true,
            kotlin: false,
            csharp: true,
            c: true,
            cpp: true,
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
// 语义（向量）检索配置
// ============================================================================

/// 向量语义检索配置（`semantic_search` 用的本地嵌入模型）
///
/// 语义检索是**可选能力**：不写本节 = 关闭，工具照常用，只是没有语义检索。
///
/// ```toml
/// [semantic]
/// enabled = true
/// model = "paraphrase-multilingual-MiniLM-L12-v2"   # 模型名或模型目录路径
/// # model_dir = "D:/models"                         # 可选：覆盖模型根目录
/// ```
///
/// 三个字段都是 `Option`：合并全局配置与项目配置时，只有**写了**的字段才覆盖，
/// 没写的保持基准值 —— 否则项目配置里的默认值会把全局配置静默抹掉。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticConfig {
    /// 是否启用；**不写**时按「是否给了 model」推断（给了模型就是想用）
    #[serde(default)]
    pub enabled: Option<bool>,

    /// 模型名（在模型根目录下查找）或模型目录路径；空 = 未配置
    #[serde(default)]
    pub model: Option<String>,

    /// 覆盖模型根目录，默认 `~/.codeconnect/models`（可被 `CODECONNECT_MODEL_DIR` 覆盖）
    #[serde(default)]
    pub model_dir: Option<PathBuf>,
}

impl SemanticConfig {
    /// 生效的开关
    ///
    /// 显式 `enabled` 优先；未写时「配了 model」即视为启用 ——
    /// 写了模型却被当成没配，是一种静默失效，本文件顶部注释警告过同一类问题。
    pub fn is_enabled(&self) -> bool {
        match self.enabled {
            Some(v) => v,
            None => self.model_value().is_some(),
        }
    }

    /// 模型串（去空白后的非空值）
    pub fn model_value(&self) -> Option<&str> {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// 显式关闭、却又配了 model —— 模型被忽略，属于容易看走眼的组合
    pub fn disabled_but_model_set(&self) -> bool {
        self.enabled == Some(false) && self.model_value().is_some()
    }
}

// ============================================================================
// 配置加载函数
// ============================================================================

/// 加载配置（以当前工作目录为基准）
///
/// `load_config_from(&current_dir())` 的薄封装，保留给未显式指定项目根目录的调用点。
pub fn load_config() -> CodeConnectConfig {
    match std::env::current_dir() {
        Ok(cwd) => load_config_from(&cwd),
        // cwd 不可用时退化为仅全局配置
        Err(_) => load_global_config(),
    }
}

/// 加载配置（以 `root` 为基准）
///
/// 从 `root` 开始向上查找 `.codeconnect.toml`，找到后读取；
/// 同时尝试读取 `~/.codeconnect/config.toml` 作为全局配置基准，
/// 项目配置会覆盖全局配置。
///
/// 如果找不到任何配置文件，返回默认配置。
pub fn load_config_from(root: &Path) -> CodeConnectConfig {
    load_config_from_with_source(root).0
}

/// 配置来源信息
///
/// 供 CLI 如实打印「某个值到底来自哪里」。先前用「合并后的值是否为空」当判据，
/// 而 `index.data_dir` 的 serde 默认值非空，导致无论有没有配置文件都标成
/// 「配置文件 index.data_dir」—— 用户会照着日志去找并不存在的配置项。
#[derive(Debug, Clone, Default)]
pub struct ConfigSource {
    /// 命中的项目配置文件 `.codeconnect.toml`；`None` 表示向上查找无果
    pub project_file: Option<PathBuf>,

    /// 生效的 `index.data_dir` 显式写在哪个配置文件里；
    /// `None` 表示没有任何配置文件写过它，用的是内置默认值
    pub data_dir_file: Option<PathBuf>,
}

/// 项目配置文件的解析结果
struct ProjectConfig {
    config: CodeConnectConfig,
    /// 命中的配置文件路径
    path: PathBuf,
    /// 该文件里是否显式写了 `[index].data_dir`
    data_dir_explicit: bool,
}

/// 加载配置，并返回来源信息
pub fn load_config_from_with_source(root: &Path) -> (CodeConnectConfig, ConfigSource) {
    let (mut config, global_data_dir_file) = load_global_config_with_source();

    match find_project_config(root) {
        Some(project) => {
            // 来源判定必须与 merge_configs 的实际语义一致：它只在
            // 「项目里的值 != 内置默认值」时才采纳该值，否则 data_dir
            // 实际来自全局配置或内置默认，不能标成项目配置文件。
            let data_dir_file = if project.data_dir_explicit
                && project.config.index.data_dir != default_data_dir()
            {
                Some(project.path.clone())
            } else {
                global_data_dir_file
            };

            merge_configs(&mut config, project.config);
            (
                config,
                ConfigSource {
                    project_file: Some(project.path),
                    data_dir_file,
                },
            )
        }
        None => (
            config,
            ConfigSource {
                project_file: None,
                data_dir_file: global_data_dir_file,
            },
        ),
    }
}

/// 加载全局配置文件 `~/.codeconnect/config.toml`
fn load_global_config() -> CodeConnectConfig {
    load_global_config_with_source().0
}

/// 加载全局配置文件，并附带「它是否显式写了 `index.data_dir`」的来源信息
fn load_global_config_with_source() -> (CodeConnectConfig, Option<PathBuf>) {
    let global_path = dirs_home().join(".codeconnect").join("config.toml");

    match std::fs::read_to_string(&global_path) {
        Ok(content) => {
            let explicit = has_explicit_data_dir(&content);
            let config = toml::from_str(&content).unwrap_or_default();
            (config, explicit.then_some(global_path))
        }
        Err(_) => (CodeConnectConfig::default(), None),
    }
}

/// 从 `root` 向上查找并解析 `.codeconnect.toml`
///
/// 返回命中的配置及其文件路径；某个候选文件解析失败时打印警告并继续向上查找。
fn find_project_config(root: &Path) -> Option<ProjectConfig> {
    let mut dir = Some(root);

    while let Some(d) = dir {
        let config_path = d.join(".codeconnect.toml");
        if config_path.is_file() {
            match std::fs::read_to_string(&config_path) {
                Ok(content) => match toml::from_str::<CodeConnectConfig>(&content) {
                    Ok(config) => {
                        return Some(ProjectConfig {
                            data_dir_explicit: has_explicit_data_dir(&content),
                            config,
                            path: config_path,
                        });
                    }
                    Err(e) => {
                        eprintln!("警告：TOML 配置文件解析失败 ({}) : {}", config_path.display(), e);
                    }
                },
                Err(e) => {
                    eprintln!("警告：无法读取配置文件 ({}) : {}", config_path.display(), e);
                }
            }
        }

        // 向上一级目录
        dir = d.parent();
    }

    None
}

/// 判断 TOML 文本里是否**显式**写了 `[index].data_dir`
///
/// 复用已读到的文本再按 `toml::Value` 解析一次，不额外做 IO；
/// 需要它是因为反序列化后的结构体无法区分「显式写了默认值」与「没写」。
fn has_explicit_data_dir(content: &str) -> bool {
    toml::from_str::<toml::Value>(content)
        .ok()
        .and_then(|v| {
            v.get("index")
                .and_then(|index| index.get("data_dir"))
                .map(|_| ())
        })
        .is_some()
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
    base.languages.c = overlay.languages.c;
    base.languages.cpp = overlay.languages.cpp;

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

    // 语义检索配置：**逐字段**覆盖 —— 字段是 Option，None 表示该文件没写这一项，
    // 此时必须保留基准值（全局配置），否则项目配置会把全局的模型设置静默抹掉
    if overlay.semantic.enabled.is_some() {
        base.semantic.enabled = overlay.semantic.enabled;
    }
    if overlay.semantic.model.is_some() {
        base.semantic.model = overlay.semantic.model;
    }
    if overlay.semantic.model_dir.is_some() {
        base.semantic.model_dir = overlay.semantic.model_dir;
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
        assert!(config.languages.java);
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
        assert!(config.java);
        assert!(!config.kotlin);
        assert!(config.csharp);
        assert!(config.c);
        assert!(config.cpp);
    }

    #[test]
    fn test_workspace_config_default() {
        let config = WorkspaceConfig::default();
        assert_eq!(config.roots, vec![PathBuf::from(".")]);
        assert!(!config.excludes.is_empty());
    }

    #[test]
    fn test_semantic_default_is_disabled() {
        // 不写 [semantic] 本节 = 关闭
        let config = CodeConnectConfig::default();
        assert!(!config.semantic.is_enabled());
        assert!(config.semantic.model_value().is_none());
        // 空配置文件解析出来也是关闭
        let back: CodeConnectConfig = toml::from_str("").unwrap();
        assert!(!back.semantic.is_enabled());
    }

    #[test]
    fn test_semantic_model_implies_enabled() {
        // 只写了 model、没写 enabled：按「想用」处理，而不是静默忽略模型
        let cfg: SemanticConfig = toml::from_str("model = \"m1\"\n").unwrap();
        assert!(cfg.is_enabled());
        assert_eq!(cfg.model_value(), Some("m1"));

        // 显式 enabled = false 时，model 被忽略且能被上层察觉
        let cfg: SemanticConfig = toml::from_str("enabled = false\nmodel = \"m1\"\n").unwrap();
        assert!(!cfg.is_enabled());
        assert!(cfg.disabled_but_model_set());

        // 空白串不算配置
        let cfg: SemanticConfig = toml::from_str("model = \"   \"\n").unwrap();
        assert!(!cfg.is_enabled());
    }

    #[test]
    fn test_semantic_merge_is_field_wise() {
        // 全局配了模型，项目配置只写了 enabled —— 模型不能被抹掉
        let mut base = CodeConnectConfig::default();
        base.semantic.model = Some("global-model".into());
        base.semantic.model_dir = Some(PathBuf::from("D:/models"));

        let overlay: CodeConnectConfig = toml::from_str("[semantic]\nenabled = true\n").unwrap();
        merge_configs(&mut base, overlay);

        assert_eq!(base.semantic.enabled, Some(true));
        assert_eq!(base.semantic.model.as_deref(), Some("global-model"));
        assert_eq!(
            base.semantic.model_dir.as_deref(),
            Some(Path::new("D:/models"))
        );
    }

    #[test]
    fn test_semantic_toml_section() {
        // 文档里给用户的写法必须真的能被解析（字段名写错 serde 会静默忽略）
        let text = "[semantic]\nenabled = true\nmodel = \"bge-m3\"\nmodel_dir = \"E:/m\"\n";
        let config: CodeConnectConfig = toml::from_str(text).unwrap();
        assert_eq!(config.semantic.enabled, Some(true));
        assert_eq!(config.semantic.model_value(), Some("bge-m3"));
        assert_eq!(config.semantic.model_dir.as_deref(), Some(Path::new("E:/m")));
    }

    #[test]
    fn test_has_explicit_data_dir() {
        // 显式写到 `[index]` 段里才算
        assert!(has_explicit_data_dir("[index]\ndata_dir = \"idx\"\n"));
        // 段落存在但没写 data_dir
        assert!(!has_explicit_data_dir("[index]\nincremental = true\n"));
        // 完全没有配置
        assert!(!has_explicit_data_dir(""));
        // 写错段名不算（serde 也会静默忽略）
        assert!(!has_explicit_data_dir("[idx]\ndata_dir = \"idx\"\n"));
    }

}
