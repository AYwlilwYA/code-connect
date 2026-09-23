# Spec: 对抗上下文丢失与低效查找

> 关键词：compact 上下文丢失 重读所有代码 项目地图 project map PROJECT_MAP.md
> grep 不准确 引用索引 put_ref_edge find_references 源码切片 get_symbol include_source
> 符号源码 body_text definition 多次 read

- 状态：⑧⑦ 均已实施并端到端验证；⑨ 未做
- 日期：2026-09-23
- 范围：解决 AI 写代码时的三个核心痛点

---

## 实施结果（2026-09-23）

### ⑧ 已实施

- `GetSymbolParams.include_source`（默认 true）、`GetFileSymbolsParams.include_source`（默认 false，
  一个文件符号多，默认带源码会一次吃掉大量上下文）。
- `tools.rs::extract_source` 用 `line`/`end_line` 对源文件切片，上限 `SOURCE_MAX_LINES = 200`，
  超长标记 `truncated`。
- **实测**：`get_symbol("greet")` 返回 `source.code` = 源文件第 7–10 行原文。
- 失败路径均返回 `available:false` + 原因（文件缺失 / 行号越界 / 未配置项目根目录），
  并有 5 个单测覆盖。

### ⑦ 已实施

- `TantivyIndex::scan_all_symbols` 单遍遍历全量符号（避免 N 次随机读），
  `QueryEngine` 透传。
- `tools.rs::handle_get_project_map` + `render_project_map`，四级降级
  `Detailed → Names → Counts → Summary`，按 `estimate_tokens`（字符数/3，偏保守）
  选择首个不超预算的层级；**超预算时显式 warning，不静默截断**。
- 落盘 `<data_dir>/PROJECT_MAP.md`。
- **实测**（60 文件 / 3416 符号的隔离副本）：
  - `budget_tokens=3000` → 降级 `counts`，~1080 token，warning 明示降级
  - `budget_tokens=300` → 降级 `summary`，~112 token
  - `focus=crates/mcp, budget=2000` → 保持 `detailed`，453 符号，~1768 token
  - 落盘文件与工具返回一致

### 代码审查中查出的两个既有严重缺陷（已修）

这两条**不在原计划内**，是审查 Agent 与端到端实验挖出来的，都不在本次新增代码里，
但本次新功能直接踩在它们上面。

#### 缺陷 A — 增量索引写出的文件路径是反斜杠，导致索引内同一文件存在两份记录

| 项 | 内容 |
|---|---|
| 现象 | 真索引里同一文件有两条记录：`crates/mcp/src/tools.rs`（216 符号）与 `crates\mcp\src\tools.rs`（325 符号），共 11 对 |
| 根因 | `full_indexer.rs:558` 有 `.replace('\\', "/")`（注释「统一使用正斜杠」），`incremental.rs` 的同名计算没有 |
| 后果 | 同一文件被索引两次且内容分裂；按路径查询只命中其中一份（且往往是陈旧的那份）而**状态仍为 Success**；计数虚高；按目录分组、按前缀过滤全部失配 |
| 修法 | `incremental.rs` 补上同样的归一化（根因）；`tantivy_index.rs::doc_to_result` 读取时再归一化一次（兜住已损坏的历史索引） |

**为何此前的端到端测试没测出来**：隔离副本走的是全量索引（有归一化），
只有 `serve` 的文件监控走增量索引。必须真正触发 watcher 才能复现。

#### 缺陷 B — 增量索引从不提交符号索引，文件监控形同虚设

| 项 | 内容 |
|---|---|
| 现象 | 修改源文件后日志显示「增量索引: 处理 1 个变更文件」，但新符号**搜不到** |
| 根因 | `incremental.rs` 收尾只调了 `call_edge_index.commit()`，**从未调用 `tantivy.commit()`**。符号只写进 writer 缓冲，不落盘 |
| 后果 | 文件监控的「自动增量更新」实际无效；写入的文档会一直悬着，直到某次全量索引的 `commit()` 把它们顺带刷盘 —— 这也正是缺陷 A 中反斜杠条目得以出现在索引里的原因 |
| 修法 | `reindexed_count > 0` 时补 `self.tantivy.commit()` |

**验证方式**（隔离项目，撑住 stdin 让 serve 存活，改文件触发 watcher）：
修复前 `gamma` 搜不到且路径为 `src\a.rs`；修复后 `gamma` 可搜到、路径为 `src/a.rs`。

### 未决事项

1. **CLAUDE.md 未改动**（用户决定）。因此「自动挺过 compact」只走通一半：
   文件会生成，但没有东西自动把它拉进上下文。查实 `.codeconnect/` 被 `.gitignore:7`
   忽略，若在已提交的 CLAUDE.md 中写 `@.codeconnect/PROJECT_MAP.md` 导入，
   同事 clone 或删除索引后引用会悬空，故未采用。
2. **本项目自身的 `PROJECT_MAP.md` 尚未生成** —— 运行中的 MCP 服务占用 tantivy
   索引锁（实测 `LockBusy`），需重启 Claude Code 后执行 `index -f` 再调用一次工具。

---

## 一、三个痛点（用户实际使用中提出）

1. **每次 compact 就完全丢失所有代码语义，每次重读所有代码**
2. **grep 搜索代码不正确**
3. **多次 read 找代码**

## 二、根因（均有代码证据）

### 痛点 1 — compact 后重读

根因**不在 compact 本身**，而在于 CodeConnect 的输出只是一次性对话内容：
答完即成为聊天记录，compact 时被摘要丢弃。索引中的语义**从未进入持久上下文**。

且 CodeConnect **没有任何「项目地图」能力** —— 最接近的只有 `list_files` 分页列表。

### 痛点 2 — grep 不正确

根因：**引用边机制是死的**。

| 事实 | 证据 |
|---|---|
| `put_ref_edge` 有定义，**全项目零调用点** | `crates/index/src/sled_store.rs:229` |
| `get_ref_edge` 有定义，**全项目零调用点** | `crates/index/src/sled_store.rs:243` |
| `find_references` 实际调用的是调用图，**只能找到「函数调用」一种引用** | `crates/mcp/src/tools.rs` `handle_find_references` → `call_graph.trace_callers` |

于是字段读取、类型标注、跨文件符号引用一律找不到，AI 只能退回 grep，
再被 grep 的子串误匹配与注释命中坑住。

### 痛点 3 — 多次 read

根因，代码里写得很直白：

```rust
crates/index/src/full_indexer.rs:511    "", // definition — 暂不在解析器中提取
crates/index/src/full_indexer.rs:512    "", // body_text  — 暂不在解析器中提取
```

schema 中的 `definition` / `body_text` **从未被填充，恒为空字符串**。
CodeConnect 只回答「符号在哪一行」，不回答「它长什么样」→ AI 必须 read 文件。

## 三、方案

### ⑧ 符号源码随取（治痛点 3）— 先做

`get_symbol` 增加 `include_source`（默认 `true`）：利用索引里**已有**的
`line` / `end_line` / `file_path`，直接读取该符号对应的源码切片一并返回。

- **无需修改 schema、无需重建索引** —— 这是它优先于 ⑨ 的原因。
- `get_file_symbols` 同样支持按需带源码。
- 源码过长时截断并标注，避免又把上下文吃光（与 ⑥ 一脉相承）。
- 文件已被删除或行号越界时，如实返回「源码不可用」及原因，**不静默给空串**。

**收益**：「这个函数怎么写的」从「read 整个文件」降为「1 次调用」。

### ⑦ 项目语义快照 + 落盘（治痛点 1）— 后做

1. 新增工具 `get_project_map`：
   - 参数 `budget_tokens`（默认约 3000）与可选 `focus`（聚焦子目录/模块）。
   - 返回按模块分组的骨架：公开类型/函数名 + 截断签名 + 入口点 + 模块间依赖方向。
   - **按预算裁剪**：超预算时逐级降级（砍签名 → 只留名字 → 只留模块名与数量），
     并在结果中标明已降级，不静默截断。
2. **落盘写入 `.codeconnect/PROJECT_MAP.md`**，并在项目 `CLAUDE.md` 中加一行引用。

> **为何落盘是关键一手**：这是唯一能让语义**自动挺过 compact** 的机制。
> `CLAUDE.md` 与 `MEMORY.md` 一样，每次会话自动进入上下文 ——
> 不依赖「AI 记得去调这个工具」。仅靠工具，compact 后 AI 未必想得起来调。

### ⑨ 真·引用索引（治痛点 2）— 本轮不做

激活 `put_ref_edge`，索引时记录所有引用位置并区分「定义 / 调用 / 读取」，
`find_references` 改查引用边。成本最高（可能涉及 schema 变更 + 全量重建索引），单独立项。

## 四、验收标准

**⑧**
1. `get_symbol` 返回体中含该符号的源码片段，与源文件对应行区间一致。
2. `include_source=false` 时不返回源码。
3. 文件已删除 / 行号越界 → 明确说明源码不可用及原因，不返回空串伪装成功。
4. 超长符号体被截断且标注截断。

**⑦**
1. `get_project_map` 在指定 `budget_tokens` 内返回，且标明是否因预算降级。
2. `.codeconnect/PROJECT_MAP.md` 被生成，内容与工具返回一致。
3. 项目 `CLAUDE.md` 中含对该文件的引用。
4. compact 后仅凭该文件即可说出项目主要模块与入口点。

## 五、风险

| 风险 | 应对 |
|---|---|
| ⑧ 返回源码会放大响应体积，与 ⑥ 的瘦身目标冲突 | 默认截断 + 提供 `include_source=false`；`search_symbol` 仍保持 brief |
| ⑦ 写入项目 `CLAUDE.md` 属于修改用户仓库文件 | 已获用户明确同意；仅追加引用行，不改动既有内容 |
| ⑦ 的 map 会随代码演进过期 | 复用 ① 的索引陈旧度机制，map 内标注生成时间与索引版本 |
