# CodeConnect MCP 接入配置教程

本文介绍如何将 CodeConnect 接入各类支持 MCP（Model Context Protocol）的 AI 编程助手。

## 前置条件

- 已安装 `codeconnect` 命令行工具（`cargo build --release` 或下载预编译版本）
- 已在目标项目中运行 `codeconnect index -p . -f` 构建索引（首次必须全量索引）
- 确认 `codeconnect` 在系统 PATH 中，或记下其完整路径

验证安装：

```bash
codeconnect --version
codeconnect status   # 查看索引是否就绪
```

---

## 一、接入 Claude Code（CLI）

### 1.1 项目级配置（推荐）

在项目根目录创建 `.mcp.json`：

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "codeconnect",
      "args": ["serve", "-p", "."]
    }
  }
}
```

将此文件提交到 Git，团队成员 clone 后只需构建索引即可自动接入。

### 1.2 全局配置（所有项目生效）

编辑用户级 MCP 配置文件：

| 系统 | 路径 |
|------|------|
| **Windows** | `%USERPROFILE%\.claude\mcp.json` |
| **macOS / Linux** | `~/.claude/mcp.json` |

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "codeconnect",
      "args": ["serve", "-p", "F:/your-project"]
    }
  }
}
```

> ⚠️ 注意：全局配置中的 `-p` 参数需要写**绝对路径**，否则 MCP 服务器无法定位到正确的项目目录。

### 1.3 Claude Code 中验证

在 Claude Code 中运行 `/mcp` 查看已连接的服务器列表，应能看到 `codeconnect` 及其 17 个工具。

---

## 二、接入 Claude Desktop

编辑 Claude Desktop 配置文件：

| 系统 | 路径 |
|------|------|
| **Windows** | `%APPDATA%\Claude\claude_desktop_config.json`（通常是 `C:\Users\<用户名>\AppData\Roaming\Claude\claude_desktop_config.json`） |
| **macOS** | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| **Linux** | `~/.config/Claude/claude_desktop_config.json` |

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "codeconnect",
      "args": ["serve", "-p", "/绝对/路径/到/你的/项目"]
    }
  }
}
```

配置完成后，**完全退出并重启 Claude Desktop**。

> ⚠️ 如果 codeconnect 不在系统 PATH 中，需要将 `"command"` 改为完整路径，例如：
> ```json
> "command": "F:\\_other\\code-connect\\target\\release\\code-connect.exe"
> ```

---

## 三、接入 VS Code / Cursor

### 3.1 使用 Claude Dev / Continue 等扩展

如果你使用的是 VS Code 中支持 MCP 的 AI 扩展（如 Claude Dev 扩展、Continue），在扩展的设置中通常会提供 MCP 服务器配置入口。

以 **Cline（原 Claude Dev）** 为例，编辑 `~/.cline/mcp_settings.json`（Windows: `%USERPROFILE%\.cline\mcp_settings.json`）：

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "codeconnect",
      "args": ["serve", "-p", "."]
    }
  }
}
```

### 3.2 使用 VS Code 工作区设置

部分 MCP 扩展支持通过 `.vscode/mcp.json` 进行工作区级配置：

```json
{
  "servers": {
    "codeconnect": {
      "command": "codeconnect",
      "args": ["serve", "-p", "${workspaceFolder}"]
    }
  }
}
```

> 具体格式取决于你使用的 MCP 扩展，请参考对应扩展的文档。

---

## 四、可用的 MCP 工具

接入成功后，AI 助手可以调用以下 17 个工具：

| 工具名称 | 功能描述 | 关键参数 |
|----------|----------|----------|
| `search_symbol` | 按名称搜索代码符号 | `query`, `kind`, `language`, `limit` |
| `get_symbol` | 获取符号完整详情 | `symbol_id` |
| `trace_callers` | 追溯上游调用者 | `symbol_id`, `max_depth` |
| `trace_callees` | 追溯下游被调用者 | `symbol_id`, `max_depth` |
| `analyze_impact` | 变更影响评估 | `symbol_ids`, `max_depth` |
| `get_call_graph` | 获取局部调用子图 | `symbol_id`, `caller_depth`, `callee_depth` |
| `get_metrics` | 代码质量指标 | `symbol_id` / `file_path` |
| `detect_dead_code` | 死代码检测 | `entry_points` |
| `check_arch_rules` | 架构规则验证 | `rule_names` |
| `semantic_search` | 语义搜索 | `description`, `language`, `limit` |
| `find_references` | 查找引用位置 | `symbol_id`, `limit` |
| `reindex` | 触发索引重建 | `file_paths`, `full` |
| `get_index_status` | 查看索引状态 | `verbose` |
| `list_files` | 列出已索引文件 | `language`, `limit`, `offset` |
| `get_type_hierarchy` | 类型继承链 | `symbol_id`, `direction` |
| `get_file_symbols` | 文件内符号列表 | `file_path` |
| `get_dependency_graph` | 获取依赖关系图 | `level`, `file_path` |

---

## 五、故障排查

### 5.1 MCP 服务器启动失败

检查 codeconnect 是否在 PATH 中：

```bash
where codeconnect    # Windows
which codeconnect    # Linux / macOS
```

如果找不到，在 MCP 配置中使用完整路径。

### 5.2 索引为空或工具返回空结果

先确认索引已构建：

```bash
codeconnect status
```

如果索引为空，重新构建：

```bash
codeconnect index -p . -f
```

### 5.3 Claude Desktop 看不到工具

1. 确认配置文件路径正确（见上方各系统路径）
2. 确认 JSON 格式有效（不能有尾随逗号、注释等）
3. **完全退出** Claude Desktop（关闭窗口 ≠ 退出，需从托盘完全退出）后重新启动
4. 在 Claude Desktop 中查看 MCP 连接状态（设置 → MCP 服务器）

### 5.4 路径中包含空格

Windows 路径中经常有空格（如 `Program Files`），需要在 JSON 中正确处理：

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "F:\\Program Files\\codeconnect\\codeconnect.exe",
      "args": ["serve", "-p", "F:\\my project"]
    }
  }
}
```

JSON 中的反斜杠需要双写（`\\`），但 args 参数中的路径使用单斜杠即可（clap 会自动处理）。

### 5.5 多项目场景

如果同时开发多个项目，建议使用**项目级 `.mcp.json`** 配置。这样切换到不同项目时，CodeConnect 会自动索引对应的项目代码，无需修改全局配置。

---

## 六、参考链接

- [MCP（Model Context Protocol）官方文档](https://modelcontextprotocol.io)
- [Claude Code MCP 集成指南](https://docs.anthropic.com/en/docs/claude-code/mcp)
- [CodeConnect 使用文档](../README.md)
