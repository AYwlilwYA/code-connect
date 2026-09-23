# Spec: 支持指定 MCP 索引目录

> 关键词：MCP 索引目录 project_root data_dir CODECONNECT_ROOT CODECONNECT_DATA_DIR
> mcp-setup 配置文件查找 cwd workspace.roots 静默失效

- 状态：待用户确认
- 日期：2026-09-23
- 范围：仅解决「索引哪个目录」这一主题，不涉及解析器/索引内容正确性

---

## 一、现象（实际使用中遇到的问题）

在 Claude Code 里以 MCP 方式使用 CodeConnect 时，**无法可靠地控制「索引哪个目录」**：

1. 想把 A 项目的索引装进 MCP，但在 B 目录下启动 Claude → 索引的是 B。
2. `mcp-setup --global` 写出的全局配置**不带任何目录参数**，只能靠「Claude Code 恰好以项目目录为 cwd 启动子进程」这一隐含巧合。
3. 配置文件 `.codeconnect.toml` 里写了 `[workspace].roots`，**改了完全没反应**。

## 二、根因（为什么错、怎么错的）

### 2.1 「索引目录」只有一条通路，且该通路没有贯穿到配置层

| 事实 | 证据 |
|---|---|
| 索引目录 = CLI 全局参数 `-p/--project-root`，默认 `.` | `crates/cli/src/main.rs:38-39` |
| **配置文件按 cwd 查找，完全不理会 `-p` 的值** | `crates/core/src/config.rs:311` 用 `std::env::current_dir()`；`main.rs:171` 调 `load_config()` 时没传 root |
| 数据目录 `--data-dir` 只有 `serve` 一个子命令有 | `main.rs:51-52`，其余命令只能走配置文件 |

**错在哪**：`-p` 只影响「扫哪个目录」，不影响「读哪份配置」。于是
`codeconnect serve -p F:/proj-a` 会得到 **proj-a 的文件 + cwd 的配置**（语言开关、`dead_code.entry_points`、`data_dir` 全是错的）。这不是崩溃，是**静默给出错误结果**，比报错更难发现。

### 2.2 `[workspace].roots` 是死字段

全项目 grep `\.roots` 只命中 `config.rs` 内部的赋值与测试（342/343/410/437），**没有任何索引流程消费它**。`crates/index/src/full_indexer.rs:315` 的 `WalkBuilder::new(&self.project_root)` 只接受单一根目录。

用户配了 `roots = ["crates/a", "crates/b"]`，索引器照旧扫全仓 —— **配了没反应、也不报错**。

### 2.3 顺带发现的同类静默失效（本 spec 只记录，不在本次修复范围）

serde 默认忽略未知字段，导致以下字段**全部静默失效**且无任何告警：

| 配置里写的 | 代码中真实字段 | 证据 |
|---|---|---|
| `[index].exclude_patterns` | 不存在 | grep 零命中 |
| `[search].default_limit` | `max_results` | `config.rs:194` |
| `[complexity].warn_threshold` | `warning_threshold` | `config.rs:219` |

README:209/216/222 与 `.codeconnect.example.toml:25/29/35` 都在教用户写这些无效字段。

另：`[workspace].excludes` 只被 `serve.rs:125` 的文件监控使用，**索引器 `FullIndexer` 根本没接收它**（`index.rs:117` 传入时不带 excludes）。

### 2.4 `mcp-setup` 与文档互相打脸

| 实现 | 文档 |
|---|---|
| 项目级写 `args: ["serve"]`（`setup.rs:53`） | `docs/mcp-setup.md:31` 写 `["serve", "-p", "."]` |
| 全局级写 `args: ["serve"]`（`setup.rs:128`） | `docs/mcp-setup.md:59` 强调「全局配置的 `-p` 需要写绝对路径」 |

## 三、设计方案

### 3.1 统一解析优先级（四级）

```
CLI 参数  >  环境变量  >  .codeconnect.toml  >  内置默认
```

新增两个环境变量，使 MCP 配置可以纯靠 `env` 字段指定目录（无需改 args）：

| 环境变量 | 作用 | 等价 CLI |
|---|---|---|
| `CODECONNECT_ROOT` | 项目根目录 | `-p/--project-root` |
| `CODECONNECT_DATA_DIR` | 索引数据目录 | `--data-dir` |

**为何要环境变量**：MCP 客户端（Claude Code / Claude Desktop / Cline）的配置里 `env` 字段比 `args` 更稳定 —— `args` 里塞绝对路径会被转义、被路径转换规则改写（本项目已踩过 MSYS 路径转换的坑），环境变量无此问题。

### 3.2 根目录解析

- `-p/--project-root` 由 `PathBuf`（`default_value = "."`）改为 `Option<PathBuf>`（默认 `None`）。
  **原因**：`default_value = "."` 无法区分「用户显式传了 `.`」和「用户没传」，前者应覆盖环境变量，后者不应。
- 解析顺序：`-p` → `CODECONNECT_ROOT` → `std::env::current_dir()`。
- 解析后统一 `canonicalize`，得到绝对路径，向下传给所有子命令（子命令签名不变）。
- 若 `-p`/环境变量给出的路径不存在或不是目录 → **明确报错退出**，不静默回退到 cwd。

### 3.3 数据目录解析

- `--data-dir` 从 `serve` 专属提升为**全局参数**（`global = true`），行为对齐 `-p`。
- 解析顺序：`--data-dir` → `CODECONNECT_DATA_DIR` → `config.index.data_dir`（相对 root 解析）→ `<root>/.codeconnect`。
- `config.index.data_dir` 若为绝对路径则直接用，不再拼接 root。

### 3.4 配置加载改为「以 root 为基准」

- 新增 `load_config_from(root: &Path) -> CodeConnectConfig`：从 `root` 开始向上查找 `.codeconnect.toml`（而非从 cwd）。
- 保留现有 `load_config()` 为 `load_config_from(&current_dir())` 的薄封装，避免破坏其它调用点。
- 全局配置 `~/.codeconnect/config.toml` 的加载逻辑不变。

### 3.5 `[workspace].roots` 真正生效

语义定义（**限定索引范围**，而非「并列多个根」）：

> `workspace.roots` 是**相对于 project_root 的子目录路径列表**；为空或 `["."]` 时表示整个 project_root。
> 索引时只遍历这些子目录，但相对路径（即 symbol 的 `file_path`）**仍以 project_root 为基准计算**。

**为什么不做成「多个并列根」**：`parse_single_file` 用 `strip_prefix(project_root)` 算相对路径（`full_indexer.rs:547`），symbol_id 依赖该相对路径。多根并列会让不同根下的同名文件产生路径歧义，破坏 symbol_id 稳定性。做成「子目录限定」则零破坏。

- 配置了不存在或越出 project_root 的 root → 索引时跳过并打印警告，不 panic。

### 3.6 `mcp-setup` 写入显式目录

| 场景 | 写入内容 |
|---|---|
| 项目级（`mcp-setup`） | `args: ["serve", "-p", "<project_root 绝对路径>"]` |
| 全局级（`--global`）+ `--project-root <path>` | `args: ["serve", "-p", "<绝对路径>"]` |
| 全局级 + 不指定项目 | 保持 `args: ["serve"]` + 写入 `env: {"CODECONNECT_ROOT": "${workspaceFolder}"}` 不可行（MCP 不透传变量），故**保持不带 -p，并在输出中明确提示**「此配置依赖客户端以项目目录为 cwd 启动」 |

> 全局级默认不带 `-p` 是**有意保留**的：一个全局 MCP 配置要服务所有项目，写死某个路径反而错。此时由 cwd 兜底是正确行为，但**必须在输出里说清楚**，不能像现在这样让人误以为配好了。

### 3.7 可观测性（日志）

按项目规范要求，随代码落地调试输出：

- 所有命令启动时，用 `tracing::info!` 打印**解析后的** root / data_dir / 配置来源（哪个文件、还是默认值）。
- `serve` 额外把这三项写进 MCP 初始化日志，便于排查「索引到别处去了」。
- 输出走 stderr（MCP 项目禁止污染 stdout）。

## 四、改动清单

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/core/src/config.rs` | 新增 `load_config_from(root)`；`load_config()` 改为薄封装；补 `[workspace].roots` 解析辅助 |
| 2 | `crates/cli/src/main.rs` | `-p` 改 `Option`；新增全局 `--data-dir`；新增 root/data_dir 解析函数（含环境变量、存在性校验）；`serve` 的命令级 `--data-dir` 移除（改由全局承接） |
| 3 | `crates/cli/src/commands/index.rs` | 把 `config.workspace.roots` 传给 `FullIndexer` |
| 4 | `crates/index/src/full_indexer.rs` | `FullIndexer` 增加 `roots` 字段；`collect_files` 按多 root 遍历，相对路径仍以 project_root 计算 |
| 5 | `crates/cli/src/commands/setup.rs` | 项目级写绝对路径 `-p`；全局级输出补充 cwd 依赖提示 |
| 6 | `README.md` | 补环境变量说明、`-p`/`--data-dir` 优先级；同时修正 §2.3 中三个失效字段的文档（改名为真实字段或标注未实现） |
| 7 | `docs/mcp-setup.md` | 修正与实现打脸的 `-p` 说明；补 `env` 方式配置 |
| 8 | `doc/README.md` | 新建文档索引 |

**不在本次范围**（记录待办）：`exclude_patterns` 字段做实或删除；`ComplexityConfig` 字段别名兼容；`import_resolver` 的 8 处 TODO；`check_arch_rules` 空壳。

## 五、验收标准

1. `codeconnect serve -p F:/proj-a` 时，加载的是 **proj-a 的** `.codeconnect.toml`（用不同语言开关构造两个项目验证）。
2. 不传 `-p`、设 `CODECONNECT_ROOT=F:/proj-a` → 结果同上。
3. `-p` 与环境变量同时给 → `-p` 胜出。
4. `-p` 指向不存在的路径 → 报错退出，退出码非 0，**不回退** cwd。
5. `.codeconnect.toml` 配 `[workspace].roots = ["crates/cli"]` 后 `codeconnect index -f`，`list_files` 结果只含 `crates/cli/` 下文件，且 `file_path` 形如 `crates/cli/src/main.rs`。
6. `mcp-setup` 生成的 `.mcp.json` 含绝对路径 `-p`；在非项目目录下启动 Claude，MCP 仍索引正确项目。
7. `cargo test --workspace` 通过；`cargo clippy` 无新增告警。

## 六、风险

| 风险 | 应对 |
|---|---|
| `-p` 由 `default_value` 改 `Option` 会改变 clap 生成的 help，可能影响已有脚本 | 解析后行为对「不传 -p」保持完全一致（回退 cwd） |
| `serve --data-dir` 命令级参数移除是**破坏性变更** | 保留命令级 `--data-dir` 作为覆盖项，同时新增全局项；两级同名参数在 clap 中需确认无冲突（**待实测**） |
| 多 root 遍历改动索引主路径 | 默认 `roots` 为空 → 行为与改动前逐字节一致，先验证默认路径不回归 |
