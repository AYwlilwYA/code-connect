//! CodeConnect CLI 入口
//!
//! 提供七个子命令：
//! - `serve` — 启动 MCP 服务器（stdio 模式）
//! - `index` — 触发代码全量索引
//! - `search` — 快速符号搜索
//! - `analyze` — 离线分析（复杂度、死代码检测等）
//! - `status` — 查看索引进度与统计
//! - `check-rules` — 架构规则验证（CI 适用）
//! - `mcp-setup` — MCP 一键配置

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt};

use codeconnect_core::config::load_config;

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

    /// 项目根目录（可选，默认当前目录）
    #[arg(short, long, global = true, default_value = ".")]
    project_root: PathBuf,
}

/// CodeConnect CLI 子命令枚举
#[derive(Subcommand)]
enum Commands {
    /// 启动 MCP 服务器
    ///
    /// 以 stdio 模式启动 Model Context Protocol 服务器，
    /// 供 AI 助手（如 Claude Desktop、VS Code Copilot）直接调用代码分析工具。
    Serve {
        /// 数据目录路径（默认从 .codeconnect.toml 配置中读取）
        #[arg(short, long)]
        data_dir: Option<String>,
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
        /// 符号类型过滤（function, class, method 等）
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

    /// MCP 一键配置
    ///
    /// 自动将 CodeConnect 注册到 Claude Code 的 MCP 配置中。
    /// 默认创建项目级 `.mcp.json`，使用 `--global` 则写入全局 `~/.claude.json`。
    McpSetup {
        /// 全局配置（写入 ~/.claude.json），默认项目级
        #[arg(long)]
        global: bool,
        /// 项目路径（全局配置时可选，不传则用当前目录）
        #[arg(long)]
        project_root: Option<PathBuf>,
    },
}

// ============================================================================
// 主函数
// ============================================================================

#[tokio::main]
async fn main() {
    // 初始化日志系统（输出到 stderr，避免干扰 MCP stdio 协议）
    fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // 尝试加载配置文件
    let config = load_config();
    let data_dir = cli.project_root.join(&config.index.data_dir);

    if let Err(e) = run_command(cli, config, data_dir).await {
        eprintln!("错误: {}", e);
        std::process::exit(1);
    }
}

/// 根据子命令执行对应的业务逻辑
async fn run_command(
    cli: Cli,
    config: codeconnect_core::config::CodeConnectConfig,
    data_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Commands::Serve { data_dir: serve_data_dir } => {
            let dir = serve_data_dir
                .map(PathBuf::from)
                .unwrap_or(data_dir);
            commands::serve::run(&cli.project_root, &dir, &config).await?;
        }

        Commands::Index { force } => {
            commands::index::run(&cli.project_root, &data_dir, &config, force).await?;
        }

        Commands::Search {
            query,
            limit,
            language,
            kind,
        } => {
            commands::search::run(&cli.project_root, &data_dir, &query, limit, language, kind)
                .await?;
        }

        Commands::Analyze { analyze_type } => {
            commands::analyze::run(&cli.project_root, &data_dir, &analyze_type).await?;
        }

        Commands::Status => {
            commands::status::run(&cli.project_root, &data_dir).await?;
        }

        Commands::CheckRules { rules } => {
            commands::analyze::run_check_rules(&cli.project_root, &data_dir, rules).await?;
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
