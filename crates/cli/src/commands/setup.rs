//! MCP 一键配置子命令
//!
//! 自动将 CodeConnect 注册到 Claude Code 的 MCP 配置中，
//! 支持项目级配置（.mcp.json）和全局配置（~/.claude.json）。

use std::path::{Path, PathBuf};

/// 运行 mcp-setup 子命令
///
/// # 参数
///
/// - `global` — 是否全局配置（写入 `~/.claude.json`），默认项目级
/// - `project_root` — 项目根目录（全局配置时可选，不传则用当前目录）
pub fn run(
    global: bool,
    project_root: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("无法获取当前可执行文件路径: {}", e))?;

    // 检测 codeconnect 是否在 PATH 中
    let codeconnect_in_path = which::which("codeconnect").is_ok();

    if global {
        setup_global(&current_exe)
    } else {
        let root = project_root.unwrap_or_else(|| PathBuf::from("."));
        setup_project(&root, &current_exe, codeconnect_in_path)
    }
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

    let server_config = serde_json::json!({
        "type": "stdio",
        "command": command,
        "args": ["serve"]
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
    eprintln!("  参数:  serve");

    Ok(())
}

/// 全局配置 — 写入 `~/.claude.json` 顶层 `mcpServers`，并尝试添加到 PATH
fn setup_global(current_exe: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let home_dir = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let claude_json_path = home_dir.join(".claude.json");

    // 自动添加到 PATH
    add_to_path(current_exe);

    let server_config = serde_json::json!({
        "type": "stdio",
        "command": "codeconnect",
        "args": ["serve"]
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
    eprintln!("  参数:  serve");

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
