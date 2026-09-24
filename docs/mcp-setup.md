# CodeConnect MCP 接入配置教程

> 关键词：MCP 接入 mcp-setup .mcp.json 全局配置 project_root CODECONNECT_ROOT
> env 环境变量 args 绝对路径 cwd 索引目录 Claude Code Claude Desktop Cline

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

一条命令搞定，**不要手写**：

```bash
cd /path/to/your-project
codeconnect mcp-setup
```

它会在项目根目录创建/更新 `.mcp.json`，并写入**项目绝对路径**：

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

**为什么要写绝对路径**：MCP 客户端从哪个目录启动子进程不可控，
`-p .` 依赖「客户端恰好以项目目录为 cwd 启动」这一巧合；写绝对路径后索引范围被固定。

将此文件提交到 Git，团队成员 clone 后只需构建索引即可自动接入（注意路径是本机绝对路径，跨机器需各自重新执行 `mcp-setup`）。

### 1.2 全局配置（所有项目生效）

推荐用 `mcp-setup --global` 自动写入，它会顺带把 `codeconnect` 所在目录加进用户 PATH：

```bash
codeconnect mcp-setup --global --project-root F:/your-project   # 绑定到固定项目
codeconnect mcp-setup --global                                  # 不绑定，服务所有项目
```

写入的是用户级 MCP 配置文件（**注意是 `.claude.json` 这个文件本身**，不是 `.claude` 目录下的其它文件）：

| 系统 | 路径 |
|------|------|
| **Windows** | `%USERPROFILE%\.claude.json` |
| **macOS / Linux** | `~/.claude.json` |

内容形如（顶层 `mcpServers`）：

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

**两种模式的取舍**：

| 模式 | `args` | 索引范围 | 适用场景 |
|------|--------|----------|----------|
| 带 `--project-root` | `["serve", "-p", "<绝对路径>"]` | 固定为该项目，与 cwd 无关 | 主要就分析这一个项目 |
| 不带 `--project-root` | `["serve"]` | **取决于客户端启动子进程时的 cwd** | 需要在多个项目间切换 |

> ⚠️ 不带 `--project-root` 时**并非配置失败**：一个全局配置要服务所有项目，
> 写死某个路径反而是错的，此时由 cwd 兜底是正确行为。
> 但你必须知道：**客户端以哪个目录为 cwd 启动 MCP 子进程，就索引哪个目录**。
> 若客户端从别处启动（如从开始菜单/全局快捷方式），索引的将不是你的项目。
> 拿不准就用带 `--project-root` 的形式，或用下一节的 `env` 方式。
>
> 手动编辑时，`-p` 必须写**绝对路径**，且建议用**正斜杠**（`F:/your-project`）：
> 反斜杠在 JSON 转义与各客户端的路径转换规则下容易被改写。

### 1.3 Claude Code 中验证

在 Claude Code 中运行 `/mcp` 查看已连接的服务器列表，应能看到 `codeconnect` 及其 18 个工具。

---

## 二、指定项目目录：`args` 与 `env` 两种方式

目录解析遵循统一优先级：

```
CLI 参数 (-p / --data-dir)  >  环境变量  >  .codeconnect.toml  >  内置默认
```

- **项目根目录**：`-p` → `CODECONNECT_ROOT` → 当前工作目录（cwd）。兜底就是 cwd，这正是「索引到错误目录」的根源。
- **数据目录**：`--data-dir` → `CODECONNECT_DATA_DIR` → `.codeconnect.toml` 的 `[index].data_dir` → `<项目根>/.codeconnect`。

写 MCP 配置时有两条路子，**推荐 `env`**。

### 2.1 方式一：`args` 传 `-p`

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

`codeconnect mcp-setup` / `codeconnect mcp-setup --global --project-root <路径>` 生成的就是这种。

### 2.2 方式二：`env` 传 `CODECONNECT_ROOT`（更稳，推荐）

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "codeconnect",
      "args": ["serve"],
      "env": {
        "CODECONNECT_ROOT": "F:/your-project",
        "CODECONNECT_DATA_DIR": "F:/your-project/.codeconnect"
      }
    }
  }
}
```

**为什么 `env` 比 `args` 稳**：
`args` 里的绝对路径要穿过 JSON 转义 → 客户端的参数拼接 → 平台路径转换（Windows/MSYS 等）
好几层规则，容易被改写或截断；`env` 是键值对直传，不经过这些处理。

| 环境变量 | 作用 | 等价 CLI 参数 |
|----------|------|---------------|
| `CODECONNECT_ROOT` | 项目根目录 | `-p` / `--project-root` |
| `CODECONNECT_DATA_DIR` | 索引数据目录 | `--data-dir` |

`CODECONNECT_DATA_DIR` 可省略，默认是 `<项目根>/.codeconnect`。

> 注意：`-p` 与 `CODECONNECT_ROOT` 同时存在时 **`-p` 优先**。
> 若 `args` 里已有 `-p`，`env` 里的 `CODECONNECT_ROOT` 不会生效。
>
> MCP 协议**不透传** `${workspaceFolder}` 这类客户端变量给 `env`，
> 所以 `env` 里也要写绝对路径，不能写成 `${workspaceFolder}`。

### 2.3 路径写法

- **绝对路径**，不要用 `.` 或相对路径（除非你确定客户端以项目目录为 cwd）。
- 一律用**正斜杠**：`F:/your-project`。反斜杠要写成 `F:\\your-project`，
  且仍可能被路径转换规则改写。

---

## 三、接入 Claude Desktop

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
> "command": "F:/_other/code-connect/target/release/codeconnect.exe"
> ```
> （二进制名是 `codeconnect`，不是 `code-connect`）

---

## 四、接入 VS Code / Cursor

### 4.1 使用 Claude Dev / Continue 等扩展

如果你使用的是 VS Code 中支持 MCP 的 AI 扩展（如 Claude Dev 扩展、Continue），在扩展的设置中通常会提供 MCP 服务器配置入口。

以 **Cline（原 Claude Dev）** 为例，编辑 `~/.cline/mcp_settings.json`（Windows: `%USERPROFILE%\.cline\mcp_settings.json`）：

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "codeconnect",
      "args": ["serve"],
      "env": { "CODECONNECT_ROOT": "F:/your-project" }
    }
  }
}
```

> `-p .` 依赖扩展以项目目录为 cwd 启动子进程，各扩展行为不一致，**不要用**。

### 4.2 使用 VS Code 工作区设置

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

> `${workspaceFolder}` 是 **VS Code 自身**的变量替换能力 —— 只在 VS Code 支持的
> `.vscode/mcp.json` 这类位置有效，普通 MCP 客户端（Claude Code / Claude Desktop / Cline）
> **不会**替换它。在其他客户端请改用绝对路径或 `env`。
>
> 具体格式取决于你使用的 MCP 扩展，请参考对应扩展的文档。

---

## 五、可用的 MCP 工具

接入成功后，AI 助手可以调用以下 18 个工具：

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
| `get_project_map` | 项目语义地图（压缩后重建认知） | `budget_tokens`, `focus` |

---

## 六、故障排查

### 6.1 MCP 服务器启动失败

检查 codeconnect 是否在 PATH 中：

```bash
where codeconnect    # Windows
which codeconnect    # Linux / macOS
```

如果找不到，在 MCP 配置中使用完整路径。

### 6.2 索引为空或工具返回空结果

先确认索引已构建：

```bash
codeconnect status
```

如果索引为空，重新构建：

```bash
codeconnect index -p . -f
```

### 6.3 Claude Desktop 看不到工具

1. 确认配置文件路径正确（见上方各系统路径）
2. 确认 JSON 格式有效（不能有尾随逗号、注释等）
3. **完全退出** Claude Desktop（关闭窗口 ≠ 退出，需从托盘完全退出）后重新启动
4. 在 Claude Desktop 中查看 MCP 连接状态（设置 → MCP 服务器）

### 6.4 路径中包含空格

Windows 路径中经常有空格（如 `Program Files`）。**统一用正斜杠**，空格本身无需转义：

```json
{
  "mcpServers": {
    "codeconnect": {
      "command": "F:/Program Files/codeconnect/codeconnect.exe",
      "args": ["serve"],
      "env": { "CODECONNECT_ROOT": "F:/my project" }
    }
  }
}
```

若坚持用反斜杠，JSON 里必须双写（`F:\\my project`），且更推荐把路径放进 `env` 而不是 `args`：

- `command`：可执行文件路径，由客户端直接启动，空格写正斜杠即可。
- `args`：路径要穿过转义与路径转换规则，**最容易被改写**。
- `env`：键值对直传，**最稳**，推荐。

### 6.5 索引到了错误的目录（索引范围不受控）

**现象**：工具返回的符号来自另一个项目，或 `list_files` 列出的文件不是你想要的目录。

**原因**：MCP 配置里没写死目录，CodeConnect 回退到了 **cwd**（即客户端启动子进程时所在目录）。

**排查**：

```bash
# 1. 看配置里到底写了什么
codeconnect mcp-setup --global --project-root F:/your-project   # 重新写入固定路径
# 2. 确认索引落在哪个目录
codeconnect status -p F:/your-project
```

**修复**：任选其一

- 重新执行 `codeconnect mcp-setup --global --project-root <项目绝对路径>`；
- 或在 MCP 配置的 `env` 中设置 `CODECONNECT_ROOT`（见第二章）；
- 或确认客户端确实以项目目录为 cwd 启动 MCP 子进程。

### 6.6 多项目场景

如果同时开发多个项目，**给每个项目建一份项目级 `.mcp.json`**：

```bash
cd /path/to/project-a && codeconnect mcp-setup
cd /path/to/project-b && codeconnect mcp-setup
```

每份配置各自写死本项目的绝对路径，互不干扰。

**不要**用「全局配置 + 固定 `--project-root`」来服务多个项目 —— 那样切到别的项目时，
索引仍然指向被写死的那个项目。全局配置要么不写 `-p`（靠 cwd 切换），要么就只服务单一项目。

---

## 七、参考链接

- [MCP（Model Context Protocol）官方文档](https://modelcontextprotocol.io)
- [Claude Code MCP 集成指南](https://docs.anthropic.com/en/docs/claude-code/mcp)
- [CodeConnect 使用文档](../README.md)
