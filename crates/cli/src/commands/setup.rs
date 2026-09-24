//! MCP 一键配置子命令
//!
//! 自动将 CodeConnect 注册到 Claude Code 的 MCP 配置中，
//! 支持项目级配置（.mcp.json）和全局配置（~/.claude.json）。

use std::path::{Path, PathBuf};

use codeconnect_core::path_util::strip_verbatim_prefix;

/// 运行 mcp-setup 子命令
///
/// # 参数
///
/// - `global` — 是否全局配置（写入 `~/.claude.json`），默认项目级
/// - `project_root` — 项目根目录（全局配置时可选，不传则依赖客户端 cwd）
pub fn run(
    global: bool,
    project_root: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("无法获取当前可执行文件路径: {}", e))?;

    // 检测 codeconnect 是否在 PATH 中
    let codeconnect_in_path = which::which("codeconnect").is_ok();

    if global {
        setup_global(&current_exe, project_root.as_deref())
    } else {
        let root = project_root.unwrap_or_else(|| PathBuf::from("."));
        setup_project(&root, &current_exe, codeconnect_in_path)
    }
}

/// 把路径转成「绝对路径 + 正斜杠」字符串，用于写进 MCP 配置的 args
///
/// MCP 子进程的 cwd 不可控，必须写绝对路径；反斜杠在 JSON 与各客户端的
/// 参数转义规则下容易被改写，统一用正斜杠。
fn abs_slash_path(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let abs = std::fs::canonicalize(path)
        .map_err(|e| format!("无法解析路径 {}: {}", path.display(), e))?;

    // Windows 的 canonicalize 会返回 `\\?\`（UNC 为 `\\?\UNC\`）前缀，
    // MCP 客户端不认，统一用 core 的剥离函数还原
    let abs = strip_verbatim_prefix(&abs);
    let s = abs.to_str().ok_or("路径不是有效的 UTF-8 字符串")?;

    Ok(s.replace('\\', "/"))
}

/// 项目级配置 — 在项目根目录创建或更新 `.mcp.json`
fn setup_project(
    project_root: &Path,
    current_exe: &Path,
    codeconnect_in_path: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mcp_json_path = project_root.join(".mcp.json");

    // 确定 command：优先使用 PATH 中的 codeconnect，否则用当前 exe 绝对路径
    let command = if codeconnect_in_path {
        "codeconnect".to_string()
    } else {
        current_exe
            .to_str()
            .ok_or("无法将可执行文件路径转换为 UTF-8 字符串")?
            .to_string()
    };

    // 项目级配置写死本项目绝对路径，避免客户端从别处启动时索引错项目
    let root_arg = abs_slash_path(project_root)?;

    let server_config = serde_json::json!({
        "type": "stdio",
        "command": command,
        "args": ["serve", "-p", root_arg]
    });

    let config = if mcp_json_path.exists() {
        // 读取已有配置并更新
        let content = std::fs::read_to_string(&mcp_json_path)
            .map_err(|e| format!("无法读取 {}: {}", mcp_json_path.display(), e))?;

        let mut existing: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("无法解析 {}: {}", mcp_json_path.display(), e))?;

        if let Some(mcp_servers) = existing
            .as_object_mut()
            .and_then(|obj| obj.get_mut("mcpServers"))
            .and_then(|v| v.as_object_mut())
        {
            // 更新已有条目
            if mcp_servers.contains_key("codeconnect") {
                eprintln!("✓ 已检测到现有 codeconnect 配置，将更新");
            }
            mcp_servers.insert(
                "codeconnect".to_string(),
                server_config.clone(),
            );
        } else {
            // mcpServers 不存在，新建
            existing
                .as_object_mut()
                .ok_or("配置文件根元素不是 JSON 对象")?
                .insert(
                    "mcpServers".to_string(),
                    serde_json::json!({
                        "codeconnect": server_config
                    }),
                );
        }

        serde_json::to_string_pretty(&existing)
            .map_err(|e| format!("序列化 JSON 失败: {}", e))?
    } else {
        // 新建配置文件
        serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {
                "codeconnect": server_config
            }
        }))
        .map_err(|e| format!("序列化 JSON 失败: {}", e))?
    };

    std::fs::write(&mcp_json_path, &config)
        .map_err(|e| format!("无法写入 {}: {}", mcp_json_path.display(), e))?;

    let cmd_display = if command == "codeconnect" {
        "codeconnect".to_string()
    } else {
        format!("\"{}\"", command)
    };
    eprintln!("✓ MCP 配置已写入: {}", mcp_json_path.display());
    eprintln!("  命令:  {}", cmd_display);
    eprintln!("  参数:  serve -p {}", root_arg);
    eprintln!("  项目:  {}", root_arg);

    Ok(())
}

/// 全局配置 — 写入 `~/.claude.json` 顶层 `mcpServers`，并尝试添加到 PATH
///
/// 指定了 `project_root` 时写死该项目绝对路径；未指定则不带 `-p`，
/// 由客户端启动 MCP 子进程时的 cwd 兜底（一个全局配置要服务所有项目，
/// 写死某个路径反而是错的）。
fn setup_global(
    current_exe: &Path,
    project_root: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let home_dir = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let claude_json_path = home_dir.join(".claude.json");

    // 自动添加到 PATH
    add_to_path(current_exe);

    let root_arg = project_root.map(abs_slash_path).transpose()?;
    let args: Vec<String> = match &root_arg {
        Some(root) => vec!["serve".to_string(), "-p".to_string(), root.clone()],
        None => vec!["serve".to_string()],
    };

    let server_config = serde_json::json!({
        "type": "stdio",
        "command": "codeconnect",
        "args": args
    });

    let config = if claude_json_path.exists() {
        let content = std::fs::read_to_string(&claude_json_path)
            .map_err(|e| format!("无法读取 {}: {}", claude_json_path.display(), e))?;

        let mut existing: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("无法解析 {}: {}", claude_json_path.display(), e))?;

        if let Some(mcp_servers) = existing
            .as_object_mut()
            .and_then(|obj| obj.get_mut("mcpServers"))
            .and_then(|v| v.as_object_mut())
        {
            if mcp_servers.contains_key("codeconnect") {
                eprintln!("✓ 已检测到现有 codeconnect 全局配置，将更新");
            }
            mcp_servers.insert(
                "codeconnect".to_string(),
                server_config.clone(),
            );
        } else {
            existing
                .as_object_mut()
                .ok_or("配置文件根元素不是 JSON 对象")?
                .insert(
                    "mcpServers".to_string(),
                    serde_json::json!({
                        "codeconnect": server_config
                    }),
                );
        }

        serde_json::to_string_pretty(&existing)
            .map_err(|e| format!("序列化 JSON 失败: {}", e))?
    } else {
        serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": {
                "codeconnect": server_config
            }
        }))
        .map_err(|e| format!("序列化 JSON 失败: {}", e))?
    };

    std::fs::write(&claude_json_path, &config)
        .map_err(|e| format!("无法写入 {}: {}", claude_json_path.display(), e))?;

    eprintln!("✓ 全局 MCP 配置已写入: {}", claude_json_path.display());
    eprintln!("  命令:  codeconnect");

    match &root_arg {
        Some(root) => {
            eprintln!("  参数:  serve -p {}", root);
            eprintln!("  项目:  {}", root);
            eprintln!("  索引范围已固定为该目录，与客户端 cwd 无关。");
        }
        None => {
            eprintln!("  参数:  serve");
            eprintln!();
            eprintln!("⚠ 未指定项目目录，配置中不含 -p 参数。");
            eprintln!("  此配置依赖客户端以项目目录为 cwd 启动 MCP 子进程；");
            eprintln!("  若客户端从其他目录启动，索引的将是那个目录而不是你的项目。");
            eprintln!();
            eprintln!("  如需固定索引某个项目，请重新运行:");
            eprintln!("    codeconnect mcp-setup --global --project-root <项目绝对路径>");
        }
    }

    Ok(())
}

/// 尝试将当前 exe 所在目录添加到系统 PATH（Windows）
fn add_to_path(current_exe: &Path) {
    #[cfg(target_os = "windows")]
    {
        let Some(parent) = current_exe.parent() else { return };
        let Some(dir) = parent.to_str() else { return };
        let dir = dir.replace("/", "\\");

        // 用 PowerShell 永久添加到用户 PATH
        let script = format!(
            r#"[Environment]::SetEnvironmentVariable('PATH', ([Environment]::GetEnvironmentVariable('PATH', 'User') + ';{}'), 'User')"#,
            dir
        );

        match std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
        {
            Ok(out) if out.status.success() => {
                eprintln!("✓ 已将 {} 添加到用户 PATH", dir);
                eprintln!("  重新打开终端后 codeconnect 命令即可全局使用");
            }
            Ok(_) => {
                eprintln!("⚠ 添加 PATH 失败，请手动将以下路径添加到 PATH:");
                eprintln!("  {}", dir);
            }
            Err(_) => {
                eprintln!("⚠ 无法自动添加 PATH，请手动添加以下路径:");
                eprintln!("  {}", dir);
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let Some(parent) = current_exe.parent() else { return };
        let Some(dir) = parent.to_str() else { return };

        let bashrc = dirs::home_dir()
            .map(|h| h.join(".bashrc"))
            .unwrap_or_default();
        let export_line = format!("\nexport PATH=\"$PATH:{}\" # codeconnect\n", dir);

        match std::fs::read_to_string(&bashrc) {
            Ok(content) if content.contains(&export_line.trim()) => {
                eprintln!("✓ PATH 已包含: {}", dir);
            }
            Ok(mut content) => {
                if let Err(e) = std::fs::write(&bashrc, format!("{}{}", content, export_line)) {
                    eprintln!("⚠ 写入 .bashrc 失败: {}", e);
                } else {
                    eprintln!("✓ 已将 {} 添加到 ~/.bashrc 的 PATH", dir);
                }
            }
            Err(_) => {
                if let Err(e) = std::fs::write(&bashrc, export_line) {
                    eprintln!("⚠ 写入 .bashrc 失败: {}", e);
                } else {
                    eprintln!("✓ 已将 {} 添加到 ~/.bashrc 的 PATH", dir);
                }
            }
        }
    }
}
