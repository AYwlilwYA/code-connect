# CodeConnect

高性能多语言代码分析工具 —— 面向 LLM Agent 的 MCP 服务器。

> **给 AI 助手**：请先加载 `skills/code-connect/SKILL.md`，了解如何使用 CodeConnect 的 18 个 MCP 工具来分析代码。

> 关键词：CodeConnect 代码分析 MCP 服务器 project_root data_dir CODECONNECT_ROOT
> CODECONNECT_DATA_DIR .codeconnect.toml 配置项 mcp-setup 索引目录 环境变量 CLI 命令

## 功能特性

- **全文符号搜索** — 基于 tantivy 的高性能符号索引与搜索，支持按名称、类型、语言过滤
- **调用图分析** — 构建完整的函数调用图、依赖图和类型层次关系，支持循环检测
- **变更影响评估** — 基于 Git 分支对比的符号级变更分析，精确评估修改影响范围
- **死代码检测** — 自动识别项目中未被引用的函数、类型和导入
- **架构规则验证** — 可配置的分层架构约束、循环依赖检测，适合 CI 集成
- **向量语义检索（可选）** — 用自然语言描述找符号（本地 ONNX 嵌入模型，离线）；
  **未配置模型时明确告知不可用，不会退回词法匹配假装成功**。配置见 [`[semantic]`](#semantic)
- **MCP 服务** — stdio 模式 MCP 服务器，可直接接入 Claude Desktop、VS Code Copilot 等 AI 助手

## 支持的编程语言

- Rust
- TypeScript
- JavaScript
- Java
- C#
- C
- C++

> Kotlin 支持待其 tree-sitter grammar 稳定后启用。

## 安装

### 下载预编译版本（推荐）

从 [Releases](https://github.com/AYwlilwYA/code-connect/releases) 下载对应平台的最新版本：

| 平台 | 文件 |
|------|------|
| Windows (x64) | `codeconnect-windows-x64.exe` |
| Linux (x64) | `codeconnect-linux-x64` |
| macOS (x64) | `codeconnect-macos-x64` |

**Windows**：将 `.exe` 放到任意 PATH 目录（如 `C:\Windows`），或在系统环境变量中添加所在目录。

**Linux / macOS**：
```bash
chmod +x codeconnect-linux-x64
sudo cp codeconnect-linux-x64 /usr/local/bin/codeconnect
```

### 接入 Claude Code

```bash
codeconnect mcp-setup --global
```

重启 Claude Code，然后 `/mcp` 确认 codeconnect 已连接。即可在对话中用自然语言分析代码。

### 从源码编译

需要 Rust 1.80+ 和 C 编译器。

```bash
git clone https://github.com/AYwlilwYA/code-connect.git
cd code-connect
cargo build --release
```

编译完成后，二进制文件位于：
- **Windows**: `target/release/codeconnect.exe`
- **Linux / macOS**: `target/release/codeconnect`

项目所有核心依赖（tree-sitter、tantivy、sled、petgraph 等）均为跨平台库，完全支持 **Windows / Linux / macOS** 三平台编译运行。

### 添加到 PATH（可选）

**Windows (CMD，管理员权限):**

将 `<你的仓库路径>` 换成本地实际路径（即 `target\release` 所在位置）：

```
setx PATH "%PATH%;<你的仓库路径>\target\release"
```

**Linux / macOS:**
```bash
sudo cp target/release/codeconnect /usr/local/bin/
```


## CLI 命令参考

| 子命令 | 说明 |
|--------|------|
| `serve` | 启动 MCP 服务器（stdio 模式），供 AI 助手直接调用 |
| `index` | 遍历项目目录，解析源文件并构建全文索引 |
| `search <query>` | 按名称搜索符号，返回位置、类型和签名信息 |
| `references <symbol>` | 查找符号的所有引用位置（文件、行号、调用类型） |
| `call-graph <symbol>` | 显示符号的调用关系图（调用者 + 被调用者） |
| `analyze` | 离线分析：圈复杂度、死代码检测、指标统计等 |
| `status` | 查看索引状态：文档数、存储占用、各语言分布 |
| `check-rules` | 架构规则验证：层依赖、循环依赖等，退出码反馈结果 |
| `mcp-setup` | 一键配置 MCP 接入（项目级或全局） |

### 项目根目录与数据目录

所有命令均支持 `-p <路径>` 指定项目根目录、`--data-dir <路径>` 指定索引数据目录。

除命令行参数外，还可以用**环境变量**指定（适合写进 MCP 配置的 `env` 字段，比塞进 `args` 更稳）：

| 环境变量 | 作用 | 等价 CLI 参数 |
|----------|------|---------------|
| `CODECONNECT_ROOT` | 项目根目录 | `-p` / `--project-root` |
| `CODECONNECT_DATA_DIR` | 索引数据目录 | `--data-dir` |

两个目录的解析优先级（从高到低）：

```
CLI 参数  >  环境变量  >  .codeconnect.toml  >  内置默认
```

- **项目根目录**：`-p` → `CODECONNECT_ROOT` → 当前工作目录（cwd）。
  解析结果会 canonicalize 成绝对路径；路径不存在或不是目录时**直接报错退出**，不会静默回退到 cwd。
- **数据目录**：`--data-dir` → `CODECONNECT_DATA_DIR` → 配置文件 `[index].data_dir`（相对项目根解析）→ `<项目根>/.codeconnect`。
- `.codeconnect.toml` 的查找**以项目根为基准**向上查找，而不是以 cwd 为基准。
  因此 `codeconnect serve -p F:/proj-a` 读到的是 **proj-a 的**配置文件。

## 接入 AI 工具（MCP 配置）

CodeConnect 支持接入任何遵循 MCP（Model Context Protocol）的 AI 编程助手，包括 **Claude Code**、**Claude Desktop**、**VS Code / Cursor** 等。

📖 **完整配置教程请参见：[docs/mcp-setup.md](docs/mcp-setup.md)**

### 快速上手

**项目级配置**（写入项目根目录的 `.mcp.json`，推荐）：

```bash
cd /path/to/your-project
codeconnect mcp-setup
```

会写入 `args: ["serve", "-p", "<项目绝对路径>"]`，**索引范围被固定**，与 Claude Code 从哪里启动无关。

**全局配置**（写入 `~/.claude.json`，所有项目共用）：

```bash
codeconnect mcp-setup --global                          # 不绑定项目
codeconnect mcp-setup --global --project-root F:/proj-a # 绑定到 proj-a
```

- 带 `--project-root` 时同样写入绝对路径 `-p`，索引范围固定。
- **不带** `--project-root` 时只写 `args: ["serve"]`：这是有意为之——一个全局配置要服务所有项目，
  写死某个路径反而是错的。此时**依赖客户端以项目目录为 cwd 启动 MCP 子进程**；
  若客户端从别处启动，索引的将是那个目录。命令输出中会明确提示这一点。

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
c = true
cpp = true

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
| `roots` | `string[]` | `["."]` | **限定索引范围**的子目录列表（相对于项目根） |
| `excludes` | `string[]` | `**/node_modules/**`、`**/target/**`、`**/build/**`、`**/dist/**`、`**/.git/**`、`**/vendor/**` | 排除的路径模式（**glob 格式**，自动合并 `.gitignore`） |

> ⚠️ `excludes` 是 **glob 模式**，必须写成 `**/目录名/**` 的形式。
> 写**裸目录名**（如 `["node_modules"]`）**不生效** —— 它不会匹配任何实际路径，
> 过滤会静默失效。正确写法：`excludes = ["**/node_modules/**", "**/target/**"]`。

`roots` 的语义是「**限定索引范围**」，不是「并列多个根」：

- 为空或 `["."]` 表示索引整个项目根（默认行为）。
- 配 `roots = ["crates/cli"]` 时只遍历该子目录，但符号的 `file_path` **仍以项目根为基准**计算（形如 `crates/cli/src/main.rs`）。
- 配置了不存在、或越出项目根的 root，索引时会跳过并打印警告。
- 该字段曾被完全忽略（配了没反应），现已在索引流程中生效。

> ⚠️ `excludes` 目前只作用于 **serve 的文件监控**；索引器 `FullIndexer` 尚未接收它，
> 因此它**不影响全量/增量索引的收录范围**。

### `[languages]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `rust` | `bool` | `true` | 启用 Rust 解析与索引 |
| `typescript` | `bool` | `true` | 启用 TypeScript 解析与索引 |
| `javascript` | `bool` | `true` | 启用 JavaScript 解析与索引 |
| `java` | `bool` | `true` | 启用 Java 解析与索引 |
| `csharp` | `bool` | `true` | 启用 C# 解析与索引 |
| `c` | `bool` | `true` | 启用 C 解析与索引 |
| `cpp` | `bool` | `true` | 启用 C++ 解析与索引 |

### `[index]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `data_dir` | `string` | `".codeconnect"` | 索引数据目录（相对于项目根） |
| `incremental` | `bool` | `true` | 是否启用增量索引（仅处理变更文件） |

> ⚠️ **没有** `[index].exclude_patterns` 这个配置项。跳过索引路径请用 `[workspace].excludes`。

### `[search]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `max_results` | `int` | `100` | 单次搜索最大返回结果数 |

> ⚠️ **没有** `[search].default_limit` 这个配置项，写了不生效。
> 另注：`max_results` 当前仅被解析与合并，**尚未被搜索服务消费**，改了暂不影响结果条数。

### `[complexity]`

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `warning_threshold` | `int` | `15` | 圈复杂度告警阈值，超过会输出警告 |
| `error_threshold` | `int` | `30` | 圈复杂度错误阈值，超过会标记为质量问题 |

> ⚠️ 字段名是 `warning_threshold`，**不是** `warn_threshold`，写错不会报错、也不会生效。

### `[[dead_code]]`

数组格式，每个条目指定一组入口点，用于死代码分析的起点。
入口点是**符号名**（如 `main`、`pub_api`），**不是文件路径**：

```toml
[[dead_code]]
entry_points = ["main", "pub_api"]
```

> ⚠️ 当前版本 `detect_dead_code` 已接入调用边，但**尚未读取本配置**：
> 未显式传参时入口点固定为 `["main"]`。因此在 `.codeconnect.toml` 里配置本项**暂不生效**，
> 请在调用工具时用 `entry_points` 参数直接指定。

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

### `[semantic]`

`semantic_search`（自然语言找符号）用的**本地向量模型**配置。这是**可选能力**：
不写本节 = 关闭，其余工具照常用。

```toml
[semantic]
enabled = true
model = "paraphrase-multilingual-MiniLM-L12-v2"   # 模型名（在模型根目录下查找）或模型目录绝对路径
# model_dir = "D:/models"                         # 可选：覆盖模型根目录
```

| 配置项 | 类型 | 默认值 | 说明 |
|--------|------|--------|------|
| `enabled` | `bool` | 由 `model` 推断 | 是否启用；不写但写了 `model` 即视为启用（写了模型却被静默忽略是另一种坑） |
| `model` | `string` | 无 | 模型名或模型目录路径；为空 = 未配置 |
| `model_dir` | `string` | `~/.codeconnect/models` | 模型根目录；也可被环境变量 `CODECONNECT_MODEL_DIR` 覆盖（两者同时存在时以本字段为准） |

**模型**：默认 `paraphrase-multilingual-MiniLM-L12-v2`（384 维、mean 池化，约 130 MB），
应放在 `<模型根目录>/paraphrase-multilingual-MiniLM-L12-v2/`，内含 `model.onnx` 与 `tokenizer.json`：

```bash
# 下载（落在 <模型根目录>，默认 ~/.codeconnect/models）
cargo run -p codeconnect-embed --features downloader --bin download-model
```

**ONNX Runtime**：另需系统上有 `onnxruntime.dll`（≥ 1.24）。解析顺序：
`ORT_DYLIB_PATH` > `CODECONNECT_ORT_DYLIB` > codeconnect 可执行文件同目录 > `PATH`。

**`semantic_search` 的三种状态**（`retrieval` 字段会如实标注本次实际用了哪种检索方式）：

| 状态 | 行为 |
|------|------|
| 未配置（缺本节 / `enabled = false`） | 明确回「未配置向量模型，语义检索不可用」+ 配置方法；**不是错误**，**绝不退回**按名字的词法搜索 |
| 已配置但模型没就绪 | 回明确错误（`status: Partial`）+ 期望路径 + 已搜索路径 + 获取方式 |
| 已配置且就绪 | 真向量检索：查询串嵌入 → 与符号（**名称 + 签名 + doc 注释首行，不含函数体**）的向量做余弦相似取 top-K |

参数 `mode` 可选 `vector`（默认）/ `lexical`（词法对照，**不是**语义结果）/ `both`（结果逐条标注来源）。
语料是全部已索引符号，向量缓存在 `<数据目录>/embeddings.bin`，语料指纹变化时自动重建。

> ⚠️ 效果依赖索引里有多少文字可嵌：当前 Rust 解析器尚未产出 `signature`/`doc_comment`
> （索引里为空），此时嵌入文本实际退化为**只有符号名**，中文自然语言查询的召回质量会明显打折。

## 项目结构

| Crate | 说明 |
|-------|------|
| `codeconnect-core` | 核心类型、符号 ID、配置解析、错误处理、统一响应格式 |
| `codeconnect-parser` | 多语言 tree-sitter 解析器：Rust / TS / JS / Java / C# / C / C++ |
| `codeconnect-index` | 索引引擎：tantivy 全文搜索 + sled K/V 存储 + 并行索引构建 |
| `codeconnect-graph` | 图分析模块：调用图、依赖图、类型层次、循环检测、LRU 缓存 |
| `codeconnect-services` | 业务逻辑服务层：符号查找、调用分析、语义搜索、影响分析、架构查询、指标 |
| `codeconnect-diff` | Diff 感知模块：Git 分支对比、符号级变更分析 |
| `codeconnect-watcher` | 文件监控模块：notify 文件变更检测、debounce、批量处理 |
| `codeconnect-mcp` | MCP 服务器：rmcp 集成、工具注册、JSON Schema |
| `codeconnect-cli` | CLI 入口：索引、搜索、分析、MCP 服务启动（二进制 `codeconnect`） |
