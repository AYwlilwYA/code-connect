# Spec: 对抗上下文丢失与低效查找

> 关键词：compact 上下文丢失 重读所有代码 项目地图 project map PROJECT_MAP.md
> grep 不准确 引用索引 put_ref_edge find_references 源码切片 get_symbol include_source
> 符号源码 body_text definition 多次 read

- 状态：已批准实施（⑧ → ⑦，顺序执行）
- 日期：2026-09-23
- 范围：解决 AI 写代码时的三个核心痛点

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
