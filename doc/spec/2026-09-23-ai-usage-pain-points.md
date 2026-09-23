# Spec: 面向 AI 使用者的工具可用性改造

> 关键词：AI 痛点 索引陈旧 staleness 搜不到候选 suggest symbol_id 兼容名字
> language kind 过滤失效 to_string_pretty 响应体积 上下文 消耗 token brief 精简

- 状态：已批准实施
- 日期：2026-09-23
- 范围：解决「AI 调用工具时拿不到可用事实 / 拿到事实不可信 / 上下文被吃光」

---

## 一、问题来源

不是凭空设计，而是从「AI 实际用 CodeConnect 写代码时卡壳的地方」反推。每一条都有代码证据。

## 二、问题清单与根因

### ⑥ 响应体积失控，一次搜索吃掉半个上下文（**最高优先级**）

| 事实 | 证据 |
|---|---|
| `search_symbol` 默认返回 20 条，每条是完整 `Symbol` 结构 | `crates/mcp/src/tools.rs:194` `limit`；`Symbol` 共 11 字段 |
| `Symbol` 含 `doc_comment`（整段文档注释）、`signature`、`modifiers` 等 | `crates/core/src/types.rs` `pub struct Symbol` |
| 响应经 `to_string_pretty` 多行展开 | `crates/mcp/src/server.rs:354` |
| 无任何精简模式 | grep `brief`/`compact`/`fields` 在 `schemas.rs` 零命中 |

**后果**：一次 `search_symbol` 约 8k~15k token。AI 为了拿「有哪些符号」这一句话的事实，付出整个上下文预算。AI 被迫减少调用次数 → 又退回盲改代码。

**修法**：
1. `to_string_pretty` → `to_string`（紧凑 JSON，纯省，零语义变化）。
2. `search_symbol` 新增 `detail` 参数，默认 `brief`：只回 `symbol_id / name / kind / language / file_path / line / signature(截断 150 字符)`；`detail="full"` 时保持现状。
   **理由**：搜索是「定位」，不是「取详情」；详情应由 `get_symbol` 按需取。默认值给 brief 才符合 AI 的使用路径。

### ① 索引过期而 AI 无从知晓

`ResponseMeta.index_staleness_ms`（`crates/core/src/response.rs:65`）与 `with_staleness()`（`:137`）已存在，但**全项目零业务调用点** —— 只有自身定义与默认值 `None`。

**后果**：AI 基于陈旧索引回答，且没有任何信号提示它该重新索引。**静默错误比报错危险**：报错 AI 会停，静默过期 AI 会继续写。

**修法**：
- sled meta 命名空间新增 `index_built_at`（Unix 秒），全量索引与增量索引完成时写入。
- `serve` 启动时读出 → 存入 `ToolRegistry`。
- 在**统一出口** `response_to_call_tool_result`（所有 17 个工具都经过它）附加 staleness；超阈值（默认 300s）追加一条 warning，明确提示「若你刚改过代码，请先调用 reindex」。
- 索引无 `index_built_at` 记录（旧索引）→ 报「索引时间未知」，**不得假装新鲜**。

### ② 搜不到被 AI 解读成「不存在」

`handle_search_symbol` 无匹配时返回 `data: []` + `status: Success`。AI 的默认解读是「项目里没有这个函数」，转而重复造轮子。

**修法**：匹配为空时，
- 用 tantivy 模糊查询（编辑距离）产出相近候选；
- 状态置 `Partial`，`warnings` 写明：查了什么、候选有哪些（带 kind/语言/文件:行）、下一步建议（检查拼写 / 确认语言开关 / 先跑 index）。
- **绝不返回「看起来正常」的空 Success。**

### ③ 手里只有名字，工具却硬要 `symbol_id`

`get_symbol` / `trace_callers` / `trace_callees` / `analyze_impact` / `get_metrics` / `find_references` / `get_type_hierarchy` / `get_call_graph` 均按 `symbol_id` 精确查（`tools.rs:239/279/338/476/584/781`）。AI 通常只记得名字，被迫先 `search_symbol` 再调目标工具，每次多一轮往返。

更糟：查不到时 `trace_callers` 等会**静默把输入当名字继续跑**（`tools.rs:281` `_ => params.symbol_id.clone()`），产出空结果而非报错。

**修法**：新增统一解析 `resolve_symbol_ref(input)`：
- 精确 ID 命中 → 直接用；
- 否则按名字搜：唯一精确匹配 → 用；多个匹配 → 返回候选让 AI 抉择；无匹配 → 走 ② 的候选提示。
- 应用于上述 8 个 handler。

### ⑤ `language` / `kind` 过滤被丢弃，且结果被截断

**两处叠加的缺陷**：

1. `query_engine.rs:60-61` 签名写死 `_language` / `_kind`，注释自认「暂未实现」，参数直接丢弃。
2. `handle_search_symbol` 是**先按 limit 取结果、再在内存里过滤**（`tools.rs:194-215`）。于是 `language=rust&limit=20` 可能只回 3 条，尽管库里有 200 条 rust 符号 —— AI 看到的是「明明有却搜不全」。

**修法**：用 tantivy 的 `TermQuery` 在查询侧做过滤（`language`/`kind` 在 schema 中是 `STRING | STORED`，**已是精确匹配字段，无需改 schema、无需重建索引**），并把过滤条件下推到查询，消除「先截断后过滤」。

## 三、改动清单

| # | 文件 | 改动 | 对应 |
|---|---|---|---|
| 1 | `crates/index/src/sled_store.rs` | 新增 `put_index_built_at` / `get_index_built_at` | ① |
| 2 | `crates/index/src/full_indexer.rs` | 索引完成后写入 built_at | ① |
| 3 | `crates/index/src/incremental.rs` | 增量更新后写入 built_at | ① |
| 4 | `crates/mcp/src/tools.rs` | `ToolRegistry` 增 `index_built_at`；`resolve_symbol_ref`；`handle_search_symbol` 重写（过滤下推 + brief + 候选）；8 个 handler 接入名字解析 | ①②③⑤ |
| 5 | `crates/mcp/src/server.rs` | 统一出口附加 staleness + 超阈值 warning；`to_string_pretty` → `to_string` | ①⑥ |
| 6 | `crates/mcp/src/schemas.rs` | `SearchSymbolParams` 增 `detail` 字段 | ⑥ |
| 7 | `crates/index/src/tantivy_index.rs` | `search_by_name` 支持 language/kind 过滤；新增 `suggest_similar_names` | ②⑤ |
| 8 | `crates/index/src/query_engine.rs` | 透传过滤参数 | ⑤ |
| 9 | `crates/cli/src/commands/serve.rs` | 读取 built_at 注入 registry | ① |
| 10 | `skills/code-connect/SKILL.md` | 更新工具用法（brief 默认、可传名字） | ①②③⑥ |

## 四、验收标准

1. `search_symbol` 返回体不含 `doc_comment`/`modifiers` 等冗余字段；`detail="full"` 时恢复完整。
2. 响应 JSON 为单行紧凑格式。
3. 每次工具响应 `meta.index_staleness_ms` 有值；索引超 300s 时 `warnings` 含提示。
4. 旧索引（无 built_at）→ 提示「索引时间未知」，不谎报新鲜。
5. `search_symbol "不存在的名字"` → `status=Partial` + 候选列表，而非空 `Success`。
6. `trace_callers "handle_search_symbol"`（传名字不传 ID）→ 正常工作。
7. 传 `language=rust&limit=20`，返回**满 20 条** rust 符号（不再被跨语言结果挤掉）。
8. `cargo test --workspace` 通过，`cargo clippy` 无新增告警。

## 五、风险

| 风险 | 应对 |
|---|---|
| `search_symbol` 默认改 brief 属**行为变更**，可能影响既有调用方（如 Dubhe CLI） | `detail="full"` 保留原样；CLI 侧若依赖完整字段需同步确认（**待实测**） |
| staleness 阈值 300s 对慢速会话偏激进 | 先给常量 + 明确文案，不做静默吞掉 |
| 增加 `resolve_symbol_ref` 多一次搜索，可能变慢 | 仅精确 ID 未命中时才走名字搜索，正常路径零开销 |
