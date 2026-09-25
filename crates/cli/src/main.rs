//! CodeConnect CLI 入口
//!
//! 提供九个子命令：
//! - `serve` — 启动 MCP 服务器（stdio 模式）
//! - `index` — 触发代码全量索引
//! - `search` — 快速符号搜索
//! - `references` — 查找符号的所有引用位置
//! - `call-graph` — 显示符号的调用关系图
//! - `analyze` — 离线分析（复杂度、死代码检测等）
//! - `status` — 查看索引进度与统计
//! - `check-rules` — 架构规则验证（CI 适用）
//! - `mcp-setup` — MCP 一键配置

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use tracing_subscriber::{EnvFilter, fmt};

use codeconnect_core::config::{CodeConnectConfig, load_config_from_with_source};
use codeconnect_core::path_util::strip_verbatim_prefix;

mod commands;

// ============================================================================
// CLI 定义
// ============================================================================

/// CodeConnect — 高性能多语言代码分析 MCP 服务器
///
/// 提供符号搜索、调用图分析、变更影响评估、死代码检测等代码智能分析能力。
/// 支持 Rust、TypeScript、JavaScript、Java、C#、Kotlin 等多种编程语言。
#[derive(Parser)]
#[command(name = "codeconnect", version, about, long_about = None)]
struct Cli {
    /// 子命令
    #[command(subcommand)]
    command: Commands,

    /// 项目根目录
    ///
    /// 优先级：本参数 > 环境变量 CODECONNECT_ROOT > 当前工作目录。
    /// 不传时不再由 clap 填 "." 默认值，以便区分「显式传了 .」与「没传」。
    #[arg(short, long, global = true)]
    project_root: Option<PathBuf>,

    /// 索引数据目录
    ///
    /// 优先级：本参数 > 环境变量 CODECONNECT_DATA_DIR > 配置文件的 index.data_dir
    /// > `<项目根目录>/.codeconnect`。
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
}

/// CodeConnect CLI 子命令枚举
#[derive(Subcommand)]
enum Commands {
    /// 启动 MCP 服务器
    ///
    /// 以 stdio 模式启动 Model Context Protocol 服务器，
    /// 供 AI 助手（如 Claude Desktop、VS Code Copilot）直接调用代码分析工具。
    Serve {
        /// 数据目录路径（比全局 --data-dir 更具体，二者同时给出时以本参数为准）
        #[arg(short, long)]
        data_dir: Option<PathBuf>,
    },

    /// 构建代码索引
    ///
    /// 遍历项目目录、解析源文件并将符号/调用/导入关系
    /// 写入 tantivy 全文索引和 sled 键值存储。
    Index {
        /// 是否强制全量重建（忽略增量索引）
        #[arg(short, long)]
        force: bool,
    },

    /// 符号搜索
    ///
    /// 按名称在已索引的符号库中搜索，
    /// 返回匹配符号的位置、类型和签名信息。
    Search {
        /// 搜索查询字符串
        query: String,
        /// 最大返回结果数
        #[arg(short, long, default_value = "10")]
        limit: usize,
        /// 编程语言过滤
        #[arg(long)]
        language: Option<String>,
        /// 符号类型过滤（function, class, method, enum, constant 等；constant 指枚举量）
        #[arg(long)]
        kind: Option<String>,
    },

    /// 离线分析
    ///
    /// 执行代码质量分析，包括圈复杂度统计、扇入扇出评估、
    /// 死代码检测等。适合在 CI 中批量运行。
    Analyze {
        /// 分析类型（metrics, deadcode, complexity, all）
        #[arg(short, long, default_value = "all")]
        analyze_type: String,
    },

    /// 查看索引状态
    ///
    /// 显示当前索引的文档数、存储空间占用、
    /// 各语言分布以及最近更新时间。
    Status,

    /// 架构规则验证（CI 适用）
    ///
    /// 检查项目架构规则是否被违反，
    /// 包括层依赖隔离、循环依赖检测等。
    /// 退出码反映验证结果（0 = 通过，1 = 违规）。
    CheckRules {
        /// 检查的规则名称列表（不指定则检查全部）
        #[arg(long)]
        rules: Option<Vec<String>>,
    },

    /// 查找符号的所有引用位置
    ///
    /// 在调用图中查找所有调用目标符号的位置，
    /// 输出每个引用处的文件路径、行号和调用类型。
    References {
        /// 符号名称或符号 ID
        symbol: String,
        /// 是否包含声明位置
        #[arg(long, default_value = "false")]
        include_declaration: bool,
    },

    /// 显示符号的调用关系图
    ///
    /// 展示指定符号的调用者和被调用者，
    /// 支持控制追踪方向和搜索深度。
    CallGraph {
        /// 符号名称或符号 ID
        symbol: String,
        /// 追踪方向：callers（调用者）、callees（被调用者）、both（双向，默认）
        #[arg(long, default_value = "both")]
        direction: String,
        /// 最大搜索深度
        #[arg(long, default_value = "2")]
        depth: usize,
    },

    /// MCP 一键配置
    ///
    /// 自动将 CodeConnect 注册到 Claude Code 的 MCP 配置中。
    /// 默认创建项目级 `.mcp.json`，使用 `--global` 则写入全局 `~/.claude.json`。
    McpSetup {
        /// 全局配置（写入 ~/.claude.json），默认项目级
        #[arg(long)]
        global: bool,
        // 必须显式带上 short = 'p'：本参数与顶层全局 -p/--project-root 同名同 id，
        // 子命令级定义会覆盖全局定义、丢掉短选项，不补的话
        // `codeconnect mcp-setup -p X` 会报 unexpected argument '-p'。
        /// 项目路径（全局配置时可选，不传则用当前目录）
        #[arg(short = 'p', long)]
        project_root: Option<PathBuf>,
    },
}

// ============================================================================
// 主函数
// ============================================================================

#[tokio::main]
async fn main() {
    // 先解析命令行 —— 日志默认级别要靠它判断是不是 serve 子命令
    let mut cli = Cli::parse();

    // 初始化日志系统（输出到 stderr，避免干扰 MCP stdio 协议）
    // debug 构建默认 info；release 下仅 serve 默认 info：serve 是 MCP 服务器，
    // 其 stderr 不进 AI 上下文，而「索引到别处去了」正是靠启动时打印的
    // root/data_dir/配置来源来排查。其余命令 release 默认 warn。均可由 RUST_LOG 覆盖。
    let default_level = if cfg!(debug_assertions) || matches!(&cli.command, Commands::Serve { .. })
    {
        "info"
    } else {
        "warn"
    };
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(default_level)),
        )
        .init();

    // 取出 serve 的命令级 --data-dir，与全局 --data-dir 一起去 resolve_data_dir 合并
    let serve_data_dir = match &mut cli.command {
        Commands::Serve { data_dir } => data_dir.take(),
        _ => None,
    };

    // 顺序依赖：先定 root，才能按 root 加载配置，才能读配置里的 index.data_dir
    let root = match resolve_root(cli.project_root.as_deref()) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("错误: {}", e);
            std::process::exit(1);
        }
    };

    let (config, config_source) = load_config_from_with_source(&root);
    tracing::info!(
        "配置文件:   {}",
        match &config_source.project_file {
            Some(p) => p.display().to_string(),
            None => "未找到 .codeconnect.toml，使用内置默认配置".to_string(),
        }
    );

    let data_dir = resolve_data_dir(
        cli.data_dir.as_deref(),
        serve_data_dir.as_deref(),
        &root,
        &config,
        config_source.data_dir_file.as_deref(),
    );

    if let Err(e) = run_command(cli.command, root, config, data_dir).await {
        eprintln!("错误: {}", e);
        std::process::exit(1);
    }
}

// ============================================================================
// 根目录 / 数据目录解析
// ============================================================================

/// 读取非空环境变量
fn env_non_empty(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// 解析项目根目录：`-p/--project-root` > `CODECONNECT_ROOT` > 当前工作目录
///
/// 解析后统一 canonicalize 为绝对路径；路径不存在或不是目录时返回错误，
/// **不回退到 cwd**（避免「索引到别处去了」这种静默错误）。
fn resolve_root(cli_root: Option<&Path>) -> Result<PathBuf, String> {
    let (raw, origin) = match cli_root {
        Some(p) => (p.to_path_buf(), "-p/--project-root"),
        None => match env_non_empty("CODECONNECT_ROOT") {
            Some(v) => (PathBuf::from(v), "环境变量 CODECONNECT_ROOT"),
            None => (
                std::env::current_dir().map_err(|e| format!("无法获取当前工作目录: {}", e))?,
                "当前工作目录（默认）",
            ),
        },
    };

    let abs = std::fs::canonicalize(&raw).map_err(|e| {
        format!(
            "项目根目录无效（来源：{}）：{} —— {}",
            origin,
            raw.display(),
            e
        )
    })?;

    // canonicalize 在 Windows 上会带 `\\?\` verbatim 前缀，下游（CLI 输出、
    // MCP 响应里的路径）会被 AI 直接读取，必须在这一处出口剥干净，
    // 避免每个消费点各自补救。
    let abs = strip_verbatim_prefix(&abs);

    if !abs.is_dir() {
        return Err(format!(
            "项目根目录不是目录（来源：{}）：{}",
            origin,
            abs.display()
        ));
    }

    tracing::info!("项目根目录: {}（来源：{}）", abs.display(), origin);
    Ok(abs)
}

/// 解析数据目录：`-d/--data-dir` > `CODECONNECT_DATA_DIR`
/// > 配置文件的 `index.data_dir`（相对 root 解析，绝对路径直接用）
/// > `<root>/.codeconnect`
///
/// `config_data_dir_file` 是「哪个配置文件显式写了 `index.data_dir`」，
/// 由 `load_config_from_with_source` 如实给出；为 `None` 时说明没有任何
/// 配置文件写过它。**不能用「配置值是否为空」当判据** —— 该字段有非空的
/// serde 默认值，那样写的话永远走不到「内置默认值」分支。
fn resolve_data_dir(
    cli_data_dir: Option<&Path>,
    serve_data_dir: Option<&Path>,
    root: &Path,
    config: &CodeConnectConfig,
    config_data_dir_file: Option<&Path>,
) -> PathBuf {
    // 实测：全局 --data-dir 与 serve 的 -d/--data-dir 共用同一个 arg id，
    // clap 让二者落在同一个槽位、后被赋的值覆盖先前的，取值始终一致，
    // 故无需再分优先级
    let cli_hit = cli_data_dir.or(serve_data_dir);

    let (raw, origin) = if let Some(d) = cli_hit {
        (d.to_path_buf(), "-d/--data-dir".to_string())
    } else if let Some(v) = env_non_empty("CODECONNECT_DATA_DIR") {
        (PathBuf::from(v), "环境变量 CODECONNECT_DATA_DIR".to_string())
    } else if let Some(file) = config_data_dir_file {
        (
            config.index.data_dir.clone(),
            format!("配置文件 index.data_dir ({})", file.display()),
        )
    } else {
        // 没有任何配置文件显式写过 data_dir ⇒ 配置值必为内置默认 `.codeconnect`
        (
            config.index.data_dir.clone(),
            "内置默认值 .codeconnect".to_string(),
        )
    };

    // 相对路径以项目根目录为基准；绝对路径原样使用
    let joined = if raw.is_absolute() {
        raw.clone()
    } else {
        root.join(&raw)
    };

    // 数据目录可能尚未创建（由 index 创建），canonicalize 失败时按词法规范化兜底
    let abs = match std::fs::canonicalize(&joined) {
        Ok(p) => p,
        Err(_) => lexical_normalize(&joined),
    };

    // 与 root 同理：verbatim 前缀不能泄漏到日志与 MCP 响应里
    let abs = strip_verbatim_prefix(&abs);

    tracing::info!("数据目录:   {}（来源：{}）", abs.display(), origin);
    abs
}

/// 词法规范化：消除 `.` 与 `..`，不访问文件系统
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 根据子命令执行对应的业务逻辑
async fn run_command(
    command: Commands,
    root: PathBuf,
    config: codeconnect_core::config::CodeConnectConfig,
    data_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = root.as_path();

    match command {
        // serve 的命令级 --data-dir 已在 main 中合并进 data_dir
        Commands::Serve { .. } => {
            commands::serve::run(project_root, &data_dir, &config).await?;
        }

        Commands::Index { force } => {
            commands::index::run(project_root, &data_dir, &config, force).await?;
        }

        Commands::Search {
            query,
            limit,
            language,
            kind,
        } => {
            commands::search::run(project_root, &data_dir, &query, limit, language, kind).await?;
        }

        Commands::Analyze { analyze_type } => {
            commands::analyze::run(project_root, &data_dir, &analyze_type).await?;
        }

        Commands::Status => {
            commands::status::run(project_root, &data_dir).await?;
        }

        Commands::CheckRules { rules } => {
            commands::analyze::run_check_rules(project_root, &data_dir, rules).await?;
        }

        Commands::References {
            symbol,
            include_declaration,
        } => {
            commands::references::run(project_root, &data_dir, &symbol, include_declaration)
                .await?;
        }

        Commands::CallGraph {
            symbol,
            direction,
            depth,
        } => {
            commands::call_graph::run(project_root, &data_dir, &symbol, &direction, depth).await?;
        }

        Commands::McpSetup {
            global,
            project_root,
        } => {
            commands::setup::run(global, project_root)?;
        }
    }

    Ok(())
}
