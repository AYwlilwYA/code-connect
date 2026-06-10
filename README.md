# CodeConnect

高性能多语言代码分析工具 —— 面向 LLM Agent 的 MCP 服务器。

> **给 AI 助手**：请先加载 `skills/code-connect/SKILL.md`，了解如何使用 CodeConnect 的 17 个 MCP 工具来分析代码。

## 功能特性

- **全文符号搜索** — 基于 tantivy 的高性能符号索引与搜索，支持按名称、类型、语言过滤
- **调用图分析** — 构建完整的函数调用图、依赖图和类型层次关系，支持循环检测
- **变更影响评估** — 基于 Git 分支对比的符号级变更分析，精确评估修改影响范围
- **死代码检测** — 自动识别项目中未被引用的函数、类型和导入
- **架构规则验证** — 可配置的分层架构约束、循环依赖检测，适合 CI 集成
- **MCP 服务** — stdio 模式 MCP 服务器，可直接接入 Claude Desktop、VS Code Copilot 等 AI 助手

## 支持的编程语言

- Rust
- TypeScript
- JavaScript
- Java
- C#

> Kotlin 支持待其 tree-sitter grammar 稳定后启用。

## 编译安装方法

### 前置要求

- **Rust 1.80+**（[安装 rustup](https://rustup.rs)）
- **C 编译器**（tree-sitter grammar 以 C 源码编译，需要系统安装 C 工具链）：
  - **Windows**：安装 [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022)（选"使用 C++ 的桌面开发"工作负载）或 [LLVM/Clang](https://releases.llvm.org/)
  - **Linux**：`sudo apt install build-essential`（Debian/Ubuntu）或 `sudo dnf install gcc`（Fedora）
  - **macOS**：`xcode-select --install`（自动安装命令行工具含 clang）
- **Git**（可选，变更对比功能需要）

### 直接下载预编译版本

从 [Releases](https://github.com/AYwlilwYA/code-connect/releases) 页面下载对应平台的二进制文件：

| 平台 | 文件名 |
|------|--------|
| Windows (x64) | `codeconnect.exe` |
| Linux (x64) | `codeconnect` |
| macOS (x64) | `codeconnect` |

下载后放到 PATH 中即可使用。

### 从源码编译安装

```bash
git clone https://github.com/AYwlilwYA/code-connect.git
cd code-connect
cargo build --release
```

编译完成后，二进制文件位于：
- **Windows**: `target/release/code-connect.exe`
- **Linux / macOS**: `target/release/code-connect`

项目所有核心依赖（tree-sitter、tantivy、sled、petgraph 等）均为跨平台库，完全支持 **Windows / Linux / macOS** 三平台编译运行。

### 添加到 PATH（可选）

**Windows (CMD，管理员权限):**
```
setx PATH "%PATH%;F:\_other\code-connect\target\release"
```

**Linux / macOS:**
```bash
sudo cp target/release/code-connect /usr/local/bin/
```


## CLI 命令参考

| 子命令 | 说明 |
|--------|------|
| `serve` | 启动 MCP 服务器（stdio 模式），供 AI 助手直接调用 |
| `index` | 遍历项目目录，解析源文件并构建全文索引 |
| `search <query>` | 按名称搜索符号，返回位置、类型和签名信息 |
| `analyze` | 离线分析：圈复杂度、死代码检测、指标统计等 |
| `status` | 查看索引状态：文档数、存储占用、各语言分布 |
| `check-rules` | 架构规则验证：层依赖、循环依赖等，退出码反馈结果 |

所有命令均支持 `-p <路径>` 指定项目根目录。

## 接入 AI 工具（MCP 配置）

CodeConnect 支持接入任何遵循 MCP（Model Context Protocol）的 AI 编程助手，包括 **Claude Code**、**Claude Desktop**、**VS Code / Cursor** 等。

📖 **完整配置教程请参见：[docs/mcp-setup.md](docs/mcp-setup.md)**

### 快速上手

``全局配置``

```bash
codeconnect mcp-setup --global
```

``局部配置``

```bash
codeconnect mcp-setup
```

Claude Code 将自动加载此配置。详细步骤、其他工具配置、故障排查请查看上方链接。

## 应用配置

### 1. 创建配置文件

在项目根目录创建 `.codeconnect.toml`：

```toml
[workspace]
roots = ["."]

[languages]
rust = true
typescript = true
javascript = true
java = true
csharp = true

[index]
data_dir = ".codeconnect"

[search]
max_results = 50
```

完整配置项说明见 `.codeconnect.example.toml` 及下方 [配置参考](#配置参考) 章节。

### 2. 建立索引

```bash
codeconnect index -p . -f
```

`-f` / `--force` 表示强制全量重建索引，首次使用建议加上。

### 3. 查看状态

```bash
codeconnect status
```

查看索引文档数、存储空间占用及各语言分布。

### 4. 搜索符号

```bash
codeconnect search "函数名"
```

支持 `--language` 语言过滤和 `--kind` 符号类型过滤：

```bash
codeconnect search "handle_request" --language rust --kind function
```


## 配置参考

`.codeconnect.toml` 完整配置项说明：

### `[workspace]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `roots` | `string[]` | `["."]` | 项目根目录列表，monorepo 场景可指定多个 |
| `excludes` | `string[]` | — | 需要额外排除的目录模式（自动合并 `.gitignore`） |

### `[languages]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `rust` | `bool` | `true` | 启用 Rust 解析与索引 |
| `typescript` | `bool` | `true` | 启用 TypeScript 解析与索引 |
| `javascript` | `bool` | `true` | 启用 JavaScript 解析与索引 |
| `java` | `bool` | `true` | 启用 Java 解析与索引 |
| `csharp` | `bool` | `true` | 启用 C# 解析与索引 |

### `[index]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `data_dir` | `string` | `".codeconnect"` | 索引数据目录（相对于项目根） |
| `incremental` | `bool` | `true` | 是否启用增量索引（仅处理变更文件） |
| `exclude_patterns` | `string[]` | — | 索引时需要跳过的路径模式 |

### `[search]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `max_results` | `int` | `50` | 全局默认最大返回结果数 |
| `default_limit` | `int` | `20` | 搜索默认返回条数 |

### `[complexity]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `warning_threshold` / `warn_threshold` | `int` | `15` | 圈复杂度告警阈值，超过会输出警告 |
| `error_threshold` | `int` | `30` | 圈复杂度错误阈值，超过会标记为质量问题 |

### `[[dead_code]]`

数组格式，每个条目指定一组入口点，用于死代码分析的起点：

```toml
[[dead_code]]
entry_points = ["src/main.rs", "src/lib.rs"]
```

### `[[rules]]`

数组格式，每条规则定义一个架构约束：

```toml
[[rules]]
name = "no-circular-deps"
description = "检测模块间的循环依赖"

[[rules]]
name = "layer-architecture"
description = "分层架构约束（领域层不可依赖基础设施层）"
layers = ["domain", "application", "infrastructure"]
allowed = [
    { from = "application", to = "domain" },
    { from = "infrastructure", to = "domain" },
    { from = "infrastructure", to = "application" },
]
```

## 项目结构

| Crate | 说明 |
|-------|------|
| `codeconnect-core` | 核心类型、符号 ID、配置解析、错误处理、统一响应格式 |
| `codeconnect-parser` | 多语言 tree-sitter 解析器：Rust / TypeScript / JavaScript / Java / C# |
| `codeconnect-index` | 索引引擎：tantivy 全文搜索 + sled K/V 存储 + 并行索引构建 |
| `codeconnect-graph` | 图分析模块：调用图、依赖图、类型层次、循环检测、LRU 缓存 |
| `codeconnect-services` | 业务逻辑服务层：符号查找、调用分析、语义搜索、影响分析、架构查询、指标 |
| `codeconnect-diff` | Diff 感知模块：Git 分支对比、符号级变更分析 |
| `codeconnect-watcher` | 文件监控模块：notify 文件变更检测、debounce、批量处理 |
| `codeconnect-mcp` | MCP 服务器：rmcp 集成、工具注册、JSON Schema |
| `codeconnect-cli` | CLI 入口：索引、搜索、分析、MCP 服务启动（二进制 `codeconnect`） |
