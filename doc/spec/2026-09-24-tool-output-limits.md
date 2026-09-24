# Spec: 工具输出上限治理

> 关键词：输出上限 limit trace_callers 结果过大 上下文 文件全量 落盘 完整
> PROJECT_MAP budget_tokens 默认值 截断告警 total

- 状态：已批准实施
- 日期：2026-09-24
- 范围：① 项目地图的「文件全量 / 响应预算」分工 ② 9 个无上限工具补上限

---

## 一、问题（实测）

### 1. 项目地图

`PROJECT_MAP.md` **没有任何字节上限**（`std::fs::write(path, &map)` 直接写）。
`map` 的大小只由 `budget_tokens` 间接决定，且**不是硬保证** ——
连 `Summary` 都超预算时仍会返回 `Summary`，只加 warning。
`estimate_tokens` 是 `字符数/3` 的粗略估算，非真实 token 数。

每文件符号数有上限（Detailed 40 / Names 25 / Counts 6），但**文件数与目录数完全没有上限**。

**实测**（本项目 3650 符号 / 73 文件）：

| 预算 | 落到层级 | 实际体积 |
|---|---|---|
| 200 | summary | 2.3 KB / 48 行 |
| **3000（默认）** | **summary** | **2.3 KB / 48 行** |
| 6000 | counts | 15.9 KB / 589 行 |

默认预算下**连符号名都拿不到**（`counts` 需 4715 token、`names` 约 10.5k）。

### 2. 工具响应全局无上限

统一出口 `response_to_call_tool_result` 只做 `serde_json::to_string`，**不做长度检查**。

逐 handler 审计（18 个），**9 个没有任何数量上限**：

| 工具 | 危险点 |
|---|---|
| `trace_callers` / `trace_callees` | 只限深度、不限数量。热点函数一次可返回数千条 |
| `analyze_impact` | BFS 全量展开 |
| `get_call_graph` | callers + callees 数量不限 |
| `get_metrics`（file 模式） | 返回文件内全部符号的指标 |
| `detect_dead_code` | 返回全部死代码符号 |
| `get_type_hierarchy` | 祖先 + 后代全部 |
| `get_dependency_graph` | 图全量 |
| `check_arch_rules` | 全量 |

其中 `trace_callers` 是 AI 最常调用的工具之一，风险最高。

## 二、设计

### A. 项目地图：文件全量 / 响应预算

**职责分离**：

| 产物 | 内容 | 理由 |
|---|---|---|
| 落盘 `PROJECT_MAP.md` | **全量** —— 不受预算降级影响、不受每文件条数上限影响，保留所有符号 | 它是持久记录，要能被 Read/Grep 当作「项目索引」用 |
| 工具**响应** | 按 `budget_tokens` 降级（与现在一致） | 响应要进 AI 上下文，必须受控 |

响应中新增元信息，让 AI 知道「响应是摘要、文件才是全量」：

- `file_path` / `file_bytes` / `file_lines` —— 文件在哪、多大
- `response_level` / `response_degraded` —— 响应自身降到了哪一级
- `file_is_complete: true` —— 明确声明文件是全量

默认 `budget_tokens` 从 3000 提到 6000（3000 连本项目都只够 summary）。

### B. 工具输出上限

给 9 个无上限工具统一加数量上限，遵循三条原则：

1. **参数可控** —— 新增 `limit` 参数（默认 50，硬上限 200）。
2. **截断必告警** —— 截断时 `warnings` 明确写出「共 N 条，本次返回 M 条」，绝不静默截断。
3. **总数如实** —— 响应中的 `total_*` 反映**真实总数**而非返回数，让 AI 知道还有多少没拿到。

`get_metrics` 的 file 模式额外对符号数设上限。

## 三、实施结果（2026-09-24）

### A 已实施并实测

`render_project_map` 新增 `full` 参数：为真时不受预算降级影响、不设每文件符号上限，
用于落盘；为假时按层级渲染，用于响应。响应新增
`response_level` / `response_degraded` / `response_estimated_tokens` /
`written_to` / `file_bytes` / `file_lines` / `file_is_complete`。
默认 `budget_tokens` 由 3000 提到 6000。

**实测**（本项目 3650 符号）：`budget_tokens=1500` 时
响应降级为 `summary`（669 token），而落盘文件 **124,333 字节 / 3,787 行、
`file_is_complete: true`，含全部 3650 个符号**。告警明确写出
「本次响应仅为摘要；全量地图已写入 …，需要细节请直接读取该文件」。

### B 已实施并实测

新增 `clip()` 与 `truncation_warning()`，对 8 个有列表输出的工具接入
`limit`（默认 50，硬上限 `MAX_RESULT_LIMIT=200`）：

| 工具 | 实测 |
|---|---|
| `trace_callers` | total=30 → returned=2，告警含真实总数 ✅ |
| `get_metrics`（文件） | symbol_count=199 → returned=4 ✅ |
| `detect_dead_code` | 3643 → returned=5 ✅ |
| `trace_callees` / `get_call_graph` / `analyze_impact` / `get_type_hierarchy` / `get_dependency_graph` | 同一 `clip` 路径 |

**审计修正**：`check_arch_rules` 原本被我列为「无上限」是**误判** ——
它当前是半空壳，输出里只有计数与回显的入参，没有列表，无需 limit。

## 四、验证过程中新发现的两个缺陷（未修，待定）

### 缺陷 C — Rust 解析器把 impl 方法重复产出（同一行代码两份符号）

`queries/rust/symbols.scm` 两条模式重叠：

```scheme
(function_item ...) @symbol.function                     ;; 匹配任意层级
(impl_item ... (function_item ...) @symbol.method)       ;; 同一节点再匹配一次
```

tree-sitter 查询默认不锚定层级，于是 **`impl` 块内每个方法被匹配两次**，
产出 `function` 与 `method` 两个符号。因 `kind` 参与 stable_id 计算，
两者是不同 ID → 索引里两份。

**实证**：`search_symbol "search_by_name"` 返回 4 条 ——
`query_engine.rs:63` 的 `method::` 与 `function::` 各一份，`tantivy_index.rs:284` 同样。
直接后果之一：按名字解析时被判「不唯一」，必须改用完整 ID。

**影响**：索引膨胀（Rust 项目里方法的数量约翻倍）、符号计数虚高、
按名解析频繁歧义、`get_metrics`/`detect_dead_code` 都看到重复项。

### 缺陷 D — `detect_dead_code` 不传入口点时把几乎全部符号判为死代码

**实证**：不传 `entry_points` 调用，3650 个符号里报了 **3643 条死代码**。
无入口点即无可达起点，于是「全部不可达」。
输出本身有截断与告警（这部分正常），但**结论是误导性的** ——
调用方很可能误信「这个项目 99.8% 是死代码」。

## 五、缺陷 C / D 修复结果（2026-09-24）

### C 已修复并验证

`rust.rs` 在 `@symbol.function` 分支检查节点是否有 `impl_item` 祖先，有则整条匹配丢弃，
只保留 `@symbol.method` 那一条。

**实测**（60 文件的隔离副本）：符号数 **3416 → 3139**（少 277 个重复）；
`search_symbol "search_by_name"` 命中 **4 条 → 2 条**，且都是 `Method`。

### D 部分修复 —— 真正的根因未完全定位

修掉的两处（均有单测覆盖）：

1. **候选集错误**：此前把字段/参数/局部变量也当死代码候选，它们从不作为调用目标、
   必然 100% 不可达。现只取 `function` / `method`（候选 3139 → 714）。
2. **名字 / ID 错配**：`get_callees` 返回符号 **ID**，而 `all_symbols` 是符号 **名**，
   `reachable.contains(symbol_name)` 永远为假 —— 除入口点自身外全部判死。
   现改为遍历中同时记录 ID 与名字。（另修一处自己引入的 bug：
   用 `reachable` 判是否入队会导致入口点 ID 与名字相同时不进队列，BFS 一步不走。）

**但端到端结果仍然荒谬**：修复后实测仍是 **713 / 714 判为死代码**。

已查实的关键线索：

- `trace_callers("run_command")` 正确返回 `main`，**入边正常**；
- 但 `trace_callees("run_command")` 只返回 **`Ok` / `unwrap_or` / `map`** ——
  全是标准库方法名，**项目内真实的被调用者没连上边**。

**未定位的部分**：为何 BFS 从入口点只覆盖到 1 个符号。已排除「入口点同名歧义」
（用唯一名 `run_command` 做对照，结果相同）。怀疑点在调用图的**被调用者解析**：
路径调用（`commands::serve::run`）与方法调用未能解析到项目内对应符号，
于是每条链走一两跳就断。

**当前采取的处置**：加**覆盖度守卫** —— 当可达覆盖度 < 5% 且候选数 ≥ 20 时，
显式告警「这几乎可以肯定是调用图解析能力不足所致，**不是**真实死代码结论，
请勿据此删除代码」。宁可让调用方知道这个数字不可用，也不能把误导性结果当结论输出。

**结论：D 尚未真正修复，只是被改成「诚实报告自己不可信」。**
调用图被调用者解析能力是后续独立议题。

## 六、自动监控两个断点（已修复并验证）

1. **serve 判据由 `doc_count > 0` 改为「索引存储是否已加载」** ——
   此前「先起服务、再 reindex 建索引」时监控永远不启动，必须重启进程。
2. **MCP `reindex` 建完索引后自动挂上监控**（`ToolRegistry::try_claim_watcher`
   保证 serve 与 reindex 只挂一次）。

**实测**：空索引（0 符号）启动 serve → 日志显示「文件监控已启动」
（修复前是「索引为空，跳过文件监控启动」）→ 新增 `src/new.rs` →
「增量索引完成: 1 重索引」→ `search_symbol "brand_new_symbol"` **可搜到**，路径 `src/new.rs`。

## 七、验收标准

1. `PROJECT_MAP.md` 含本项目**全部**符号（3695 个左右），不受 budget 影响。
2. `get_project_map` 响应仍受 budget 控制，且返回文件路径与体积。
3. `trace_callers` 对热点函数返回 ≤ limit 条，`total_callers` 为真实总数，`warnings` 说明被截断。
4. 其余 8 个工具同样具备 limit + 告警 + 真实总数。
5. `cargo test` 全绿。

## 四、风险

| 风险 | 应对 |
|---|---|
| 超大型仓库的 `PROJECT_MAP.md` 可能很大 | 响应中报告文件体积，AI 可据此决定是否 Read；不做静默截断 |
| 新增 `limit` 参数属行为变更 | 默认值给足（50），既有调用方不传也能拿到合理结果 |
