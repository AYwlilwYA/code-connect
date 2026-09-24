# Spec: 支持指定 MCP 索引目录

> 关键词：MCP 索引目录 project_root data_dir CODECONNECT_ROOT CODECONNECT_DATA_DIR
> mcp-setup 配置文件查找 cwd workspace.roots 静默失效

- 状态：**已实施**（2026-09-24）
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

---

## 七、实施结果（2026-09-24）

### 7.1 实际改动

| 文件 | 改动 |
|---|---|
| `crates/core/src/config.rs` | 新增 `load_config_from(root)` / `load_config_from_with_source(root)`；`load_config()` 退化为薄封装；`find_and_load_project_config` → `find_project_config(root)`，不再用 `current_dir()` |
| `crates/cli/src/main.rs` | `-p` 改 `Option<PathBuf>`；新增全局 `--data-dir`；新增 `resolve_root` / `resolve_data_dir`；`Cli::parse()` 提前到日志初始化之前 |
| `crates/index/src/full_indexer.rs` | `FullIndexer` 新增 `roots` 字段 + `with_roots()` + `effective_walk_roots()` |
| `crates/cli/src/commands/index.rs` | 传入 `config.workspace.roots` |
| `crates/cli/src/commands/setup.rs` | 项目级/全局级写绝对路径 `-p`；全局级不指定项目时输出 cwd 依赖告警 |
| `crates/mcp/src/tools.rs` | `handle_reindex` 补 `.with_roots(...)` —— 否则 MCP 的 `reindex` 仍忽略 `workspace.roots` |
| `README.md` / `docs/mcp-setup.md` | 补环境变量与优先级；修正失效字段与被打脸的 `-p` 说明 |

### 7.2 实测结果（全部用 release 二进制）

| 验收项 | 结果 |
|---|---|
| `serve -p proj-a` 加载 proj-a 的 `.codeconnect.toml` | ✅ 日志显示「配置文件: proj-a/.codeconnect.toml」、`data_dir=proj-a/idx-a` |
| `CODECONNECT_ROOT=proj-b` | ✅ 来源标注「环境变量 CODECONNECT_ROOT」 |
| `-p` 与环境变量同时给 | ✅ `-p` 胜出 |
| `-p` 不存在 / 不是目录 | ✅ `exit=1`，明确报错，**不回退 cwd** |
| data_dir 优先级 | ✅ CLI > env > 配置文件，来源标注正确 |
| `roots=["sub"]` 下的 MCP `reindex` | ✅ 只索引 `sub/alpha.rs`，`other/beta.rs` 查不到（全库 1 文件） |
| stdout 纯净（MCP 硬约束） | ✅ `serve` 的 stdout **严格 0 字节**，全部日志走 stderr |
| `cargo test --workspace`（除 diff） | ✅ **248 passed / 0 failed** |
| `cargo build --release` | ✅ 通过 |

### 7.3 与原设计的偏差

1. **release 默认日志级别**：§3.7 要求「所有命令启动时打印」root/data_dir/配置来源，
   但 release 默认 `warn`，`tracing::info!` 看不见。**裁定：只给 `serve` 默认开 info**
   （serve 是 MCP 服务器，其 stderr 不进 AI 上下文），其余命令保持 `warn`，可用 `RUST_LOG` 覆盖。
   已实测：release 下 `status` 的 stderr 为 **0 行**，`serve` 的 INFO 正常可见。
2. **`serve --data-dir` 命令级参数保留**（风险表要求「先实测」）。
   实测结论：clap **不冲突**，但两级共用同一 arg id，**值被合并成同一个**
   （`--data-dir A serve --data-dir B` → B），因此取值无需再分优先级。
3. **新增：删除了实施过程中产生的死代码** `resolve_workspace_roots()`。
   它只被自己的单测引用，用 `eprintln!` 绕过日志过滤，且「返回空 = 整个 project_root」
   会与「所有 roots 都无效」撞车 → 静默扩大索引范围，正是本 spec 要消灭的 bug 类型。
   真正接上线的是 `full_indexer.rs` 的 `effective_walk_roots()`（语义更安全、有 tracing 告警）。
4. **`roots` 全部无效时索引 0 个文件**（不回退全量）。理由同上：静默扩大范围是更坏的失败模式。

### 7.4 实施中查出的新事实（修正 §2.3 的表述）

§2.3 把 `[search].max_results` 当作「真实字段」列出，**这个说法不完整**：

| 字段 | 实际状况 |
|---|---|
| `[index].exclude_patterns` | 不存在（原判断正确） |
| `[search].default_limit` / `max_results` | **字段存在，但零消费方** —— 同样是死字段，只是「存在」而非「接线」 |
| `[complexity].warn_threshold` | 不存在（原判断正确）；真实字段 `warning_threshold` 有消费方 |
| `[[dead_code]].entry_points` | **同样是死字段** —— `tools.rs` 的兜底是硬编码 `vec!["main"]`，从不读配置；而报错信息还在叫用户去配置它 |

判定方法：**用错误类型做判别器** —— serde 不认识字段就静默忽略，是已知字段则类型不符必然报解析失败。
实验组（三个可疑字段给错类型）零告警，对照组（真字段给错类型）报错 → 方法有效。

另修正两处行号漂移：`serve.rs` 的 excludes 实为 **:140**（spec 写 125）；
`config.rs` 的 excludes 合并实为 **:433/434**（spec 写 342/343）。

### 7.5 `.codeconnect.example.toml`：照抄会解析失败（已修）

复核示例文件时实测出一个比「失效字段」更严重的问题：
**原示例照抄即解析失败，而且只警告、不中断（`exit=0` 静默回退默认配置）**。

| 问题 | 实测报错 |
|---|---|
| 第一条 `[[rules]]` 缺 `layers` | `missing field 'layers'` at line 46 |
| `allowed = [{ from = "a", to = "b" }]` | `invalid type: map, expected a string` —— 真实类型是 `Vec<String>`（`config.rs:274`） |

已修正示例：补 `layers`/`allowed`、`allowed` 改为 `"来源层 -> 目标层"` 字符串格式、
删除不存在的 `exclude_patterns` / `default_limit`、`warn_threshold` → `warning_threshold`、
`max_results` 默认值 50 → 100（与 `config.rs:197` 一致）、`entry_points` 示例改符号名并标注未接线、
`roots` 注释改为「子目录限定」语义、`excludes` 注明只作用于文件监控、文件头补「serde 静默忽略未知字段」的提醒。
**实测修正后无告警、可正常建索引。**

### 7.6 未纳入本次

- `[workspace].excludes` 仍**不作用于全量索引**（`FullIndexer` 没有 excludes 字段），只作用于文件监控 —— 已实测确认。
- `[search].max_results` / `[[dead_code]].entry_points` 两个死字段的接线或删除。

---

## 八、独立审查发现的问题（2026-09-24）

主 agent 在提交前派了独立审查 agent，复现出 4 个高危 + 4 个中危。

### 8.0 修复状态

| 问题 | 状态 |
|---|---|
| H1 watcher/增量索引不遵守 roots | **已修复** |
| H2 roots 写错 + `index -f` → 清空索引、退出码 0 | **已修复** |
| H3 `\\?\` 泄漏进 AI 可见字段 | **已修复** |
| M1 `数据目录` 来源标注恒为「配置文件」 | **已修复** |
| M2 README 称「所有命令支持 `-p`」但 `mcp-setup -p` 报错 | **已修复**（补 `short = 'p'`，让文档成真而非降级文档） |
| M3 README 把 `excludes` 默认值写成裸目录名 | **已修复**（改 glob；裸名实测不生效） |
| M4 MCP `reindex` 在 roots 全无效时静默报成功 | **已修复**（改为返回 error） |
| H4 增量索引重复符号（`symbol_id` 绝对路径） | **未修** —— 根因在 HEAD、非本次引入，用户裁定不在本轮范围 |
| L1–L5 低危 | L1（错误注释）、L3（UNC）已顺带修掉；L2/L4/L5 见下 |

### 8.1 修复方式（关键设计）

**H1 的单一事实来源**：把 `FullIndexer::effective_walk_roots()` 提取为
`pub fn effective_walk_roots(project_root, roots)` 自由函数（`full_indexer.rs:655`），
全量索引 / 增量索引 / 文件监控 / CLI 预校验**四处全部转调它**，全项目无第二份 roots 校验逻辑。
watcher 改为**只监控 roots 限定的目录**（而非监控全项目再过滤事件），并区分
「默认不限定」与「显式传空」—— 用 `Vec` 的默认值而非空值兼表两义，避免重蹈「静默扩大范围」。

**H2 的 Ok(0) / Err 区分**（这是修复的核心，不是简单加个报错）：
- roots 为空或含 `"."`（不限定）、或 roots 合法但目录里确实没源文件 → `Ok(0)`、退出码 0（**合法**）
- roots 非空、不含 `"."`、且推导出的遍历起点为空（**全部无效**）→ `Err`、退出码 1，
  且 `index.rs` 在 `remove_dir_all` **之前**校验，**不删旧索引**

**H3 在唯一出口剥离**：`resolve_root` / `resolve_data_dir` 是 `canonicalize` 的唯二调用点，
在出口统一 `strip_verbatim_prefix`（新增 `crates/core/src/path_util.rs`，**含 `\\?\UNC\` 处理**），
下游 `tools.rs` 的 `written_to` 一行未改即自动干净。

**M1 把「显式性」当作解析的一等输出**：新增 `ConfigSource`，用 `has_explicit_data_dir`
（复用 `find_project_config` 已读入内存的内容，零额外 IO）判定「配置里是否**显式**写过 data_dir」，
且与 `merge_configs`「值 == 默认值就跳过」的语义对齐 ——
项目配置显式写了默认值时，实际生效的是全局配置，朴素实现会在这里说假话。

### 8.2 修复后实测（主 agent 独立复验，release 二进制）

| 项 | 实测 |
|---|---|
| H1 | 监控日志「**监控起点 1 个（…\sub）**」；改范围外 `other/beta.rs` → **全程零事件**；改范围内 `sub/alpha.rs` → `增量索引完成: 1 重索引`；之后 `beta_fn` 仍**查不到** |
| H2 | roots 写成 `subb` + `index -f` → **退出码 1**、报错明确、**旧索引完好**（1 文档 / 状态：就绪）；对照组「合法 roots 但目录为空」→ **退出码 0** |
| H3 | CLI `status`/`index` 输出**零** verbatim 前缀（唯一残留来自 tantivy crate 自己的内部日志，非我方输出） |
| M1 | 三种场景标注分别为「配置文件 index.data_dir (<文件路径>)」/「内置默认值 .codeconnect」，**不再说假话** |
| M2 | `mcp-setup -p <路径>` 成功，写入绝对正斜杠路径；**9/9 子命令**实测 `-p` 可用 |
| M3 | 决定性对照：裸名**不过滤**、`**/skip/**` 生效 |
| 回归 | `cargo test --workspace`（除 diff）**263 passed / 0 failed**；不配 roots 时文件数/符号数与改动前一致 |

### 8.3 仍未修 / 新发现

- **H4**（增量索引重复符号、`symbol_id` 用绝对路径）：根因在 HEAD 的 `incremental.rs`
  与 `full_indexer.rs` 路径口径不一致（一个绝对、一个相对）。H3 剥前缀后表现形式变干净，
  **但根因仍在**，同一符号仍会出现「全量相对路径 + 增量绝对路径」两份记录。**别被「看起来干净了」误导。**
- **新发现：`tools.rs` 的 `start_watcher_after_index` 在当前流程下不可达。**
  serve 只要三个索引存储都加载就 `try_claim_watcher()`，而该函数要求同样三个都是 `Some` ——
  两者互斥。实测「全新 data_dir + reindex」场景返回 `watcher_started: false` 印证。
  即 MCP `reindex` 自动挂监控这条路径实际不生效（serve 自己已挂）。**属既存死路径，未修。**
- **残余风险（已明确保留）**：roots 合法但目录下确实没有源文件时，`index -f` 仍会删旧索引并写入空索引
  （`Ok(0)`、退出码 0）。这是 §8.1 刻意做的区分，但用户若把 roots 指向「存在但无源码的目录」仍会静默清空。
  可考虑后续补「扫描到 0 文件且旧索引非空」的提示。
- **L2**（子命令 `--project-root` 覆盖顶层 `-p`）、**L4**（非 Windows 平台仅静态审查、未实机编译）、
  **L5**（`\\?\` 曾出现在 CLI 输出，已随 H3 修掉）。

### H1（高危）`workspace.roots` 对文件监控/增量索引**完全无效**

`serve.rs:136-159` 的 `IncrementalIndexer::new(&project_root, ...)` **只传 project_root、不传 roots**；
`incremental.rs` 全文件没有 roots 概念；`FileWatcher::new(&self.project_root, excludes)` 递归监控整个项目根。
roots **只在 `FullIndexer::collect_files` 里生效**。

**复现**（`roots = ["sub"]`，改 `other/beta.rs` 后查 `beta`）：
```
增量索引完成: 1 重索引 / 0 跳过
search_symbol beta -> {"file_path":"other/beta.rs", ...}
```

**为什么严重**：本次整合改动让 **MCP `reindex` 遵守 roots**，而**同一进程里自动挂上的 watcher 不遵守** —— 两者语义相反。
用户配 `roots = ["crates/cli"]` 想限定范围，第一次 `index` 是对的，但 serve 一跑，任何一次保存都会把范围外的文件重新灌回索引。
**直接推翻 README 本次新写的承诺与 §3.5。**

### H2（高危）roots 写错 + `index -f` → **旧索引被清空，退出码 0**

`index.rs:64-69` 的 `force` 先 `remove_dir_all`，而 `full_indexer.rs:240-255` 在 `files.is_empty()` 时 `return Ok(0)` **不报错**。

**复现**（把 `roots = ["sub"]` 手抖写成 `["subb"]`）：
```
WARN workspace.roots 中的所有项都无效，本次不索引任何文件，请修正配置
扫描文件数: 0 / 索引完成! / EXIT CODE: 0
after: 索引文档数 0
```

**为什么严重**：用户手抖 → **旧索引被删干净** → 10 个 MCP 工具全部返回空 → 退出码 0，CI 判成功。
仅靠 stderr 一条 WARN 不够（没人看 CI 的 stderr）。

**评估**：§7.3 第 4 条「不能静默回退全量」的**方向对，但当前实现给出了第三种、最坏的结果 —— 静默清空**。
应改为：roots 非空且全部无效时**报错退出**（或 `-f` 先校验 roots 合法性再删旧索引）。

### H3（高危）Windows `\\?\` verbatim 前缀泄漏进 **AI 可见字段**

来源是 §3.2 要求新增的 `canonicalize`（改动前 data_dir 是 `project_root.join(...)`，无前缀）。
`tools.rs:1855-1859` 的 `written_to` 直接 `path.display().to_string()`。

**复现**（MCP `get_project_map`）：
```
written_to = '\\?\F:\_other\...\.codeconnect\PROJECT_MAP.md'
```
该字段还会拼进 `with_warning` 文案「全量地图已写入 {}，需要细节请直接读取该文件」，**AI 会照这个路径去 Read**。
Win32 API 能吃，但 **bash / MSYS / 部分 shell 工具读不了 `\\?\` 路径**。
CLI 的 `index` / `status` 输出同样泄漏（观感问题）。

**建议**：在 `resolve_root` / `resolve_data_dir` 出口统一剥离，复用 `setup.rs:44` 的 `abs_slash_path` 做法。

### H4（高危，**根因在 HEAD、非本次引入**）增量索引产出重复符号，且 `symbol_id` 含绝对 `\\?\` 路径

`incremental.rs:208` 把**绝对** `file_path` 传给 `extract_symbols`，而 `full_indexer.rs:660-663` 传**相对**路径
（注释明写「用相对路径生成 StableSymbolId，确保可移植」）。旧记录永不清理（`remove_file_from_index` 只删 sled）。

**复现**（改 roots **之内**的 `sub/alpha.rs`）：
```
symbol_id: "rust::sub/alpha.rs::..."                    ← 全量索引的旧记录
symbol_id: "rust::\\?\F:\...\sub\alpha.rs::..."         ← 增量新写入
```
同一符号被返回两次。**本次改动没引入它，但改变了表现形式**（从「某种绝对路径」变成 `\\?\F:\...`）。

### 中危

| # | 问题 | 位置 / 证据 |
|---|---|---|
| M1 | `数据目录` 的「来源」标注**恒为**「配置文件 index.data_dir」，`内置默认值` 分支是死代码 —— observability 形同虚设。**无 `.codeconnect.toml` 的项目也这么标**，用户会照着日志去找不存在的配置 | `main.rs:291-295`；`config.rs:172` 默认值非空 ⇒ else 永不执行。已复现 |
| M2 | README 新写的「**所有命令**均支持 `-p <路径>`」不成立 —— `mcp-setup -p X` 报 `unexpected argument '-p'`（退出码 2）。根因是 `Cli` 与 `McpSetup` 同 arg id，子命令定义覆盖全局定义、**丢掉短选项** | 本次新写的文档失实。已复现 |
| M3 | README 把 `excludes` 默认值写成**裸目录名**，实测裸名**不生效**，必须 glob 形式（真实默认是 `**/node_modules/**`） | 对照实验：`["skip"]` → 处理 2 个；`["**/skip/**"]` → 处理 1 个 |
| M4 | MCP `reindex` 在 roots 全无效时静默返回 `status: reindex_complete`、无 warning（`tracing::warn!` 只进 stderr，AI 看不到）。**不清空索引**，破坏性小于 H2 | 已复现 |

### 低危

- **L1** `main.rs:283-284` 注释说「值被合并成同一个」——机制其实是**同槽位被最后一次赋值覆盖**。结论仍成立，只是注释描述错了。
- **L2** `mcp-setup` 场景下子命令的 `--project-root` 会**覆盖顶层 `-p`**（同 arg id）。对 mcp-setup 结果无害，但属隐晦副作用。
- **L3** `setup.rs:44` 的 `strip_prefix(r"\\?\")` 不处理 `\\?\UNC\...`（会被剥成 `UNC/...`）。本机无 UNC 环境，**未实测**。
- **L4** 非 Windows 平台仅做静态审查，**未实际编译运行**。
- **L5** CLI 的 `index` / `status` 输出泄漏 `\\?\`，与 H3 同源。

### 已验证为「没问题」（免重复劳动）

- 主 agent 的三处整合改动**全部核验通过**：删除 `resolve_workspace_roots` 无残留、无行为丢失；`Cli::parse()` 提前无日志丢失（clap 输出不经 tracing）；`.with_roots()` 借用/移动无冲突、空 Vec 语义与默认一致。
- `strip_prefix` 在 verbatim 前缀下**成立**；`get_symbol` 带源码能正常读文件；`\\?\` 未破坏 watcher 的 gitignore 匹配。
- 新增测试全绿、无新增 clippy 告警、日志全在 stderr（坏 TOML 时 MCP stdout 非 JSON 行数 = 0）。
