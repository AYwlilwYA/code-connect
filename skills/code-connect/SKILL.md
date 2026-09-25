---
name: code-connect
description: 多语言代码分析 MCP 工具。当需要搜索符号、追踪调用链、查找引用、检测死代码、评估圈复杂度、分析变更影响时使用。先加载此 skill 再执行分析。
---

# CodeConnect — 代码分析

这是一个多语言代码分析 MCP 服务器，可分析 Rust、TypeScript、JavaScript、Java、C#、C、C++ 项目。

## 硬约束

<HARD-GATE>
收到任何代码分析任务时，先加载此 skill，再用 MCP 工具分析。不要直接 grep、不要逐个读文件。MCP 工具能做的事，先用它做。
</HARD-GATE>

三个要点：

1. **不必先查 ID 再调工具** —— 需要 `symbol_id` 的工具都接受**直接传符号名**，
   内部会自动解析。多个同名符号时会返回候选让你选，不会静默乱挑一个。
2. **看源码不用读文件** —— `get_symbol` 默认连源码片段一起返回（`source.code`）。
   只想「看一眼这个函数怎么写的」时，不要用 Read 打开整个文件。
3. **空结果要当心** —— `search_symbol` 搜不到时会返回 `status: Partial` 和相近候选。
   若拿到的一直是这种结果，先怀疑索引过期，而不是「项目里没有这个东西」。

## Step 1: 确认索引就绪

调用 `get_index_status(verbose=true)`。

- 若 `indexed_documents > 0`：索引已就绪，直接进入 Step 2 进行分析。
- 若 `indexed_documents = 0`：**优先在 MCP 连接内调用 `reindex(full=true)` 构建索引**。该工具会在 MCP 服务器内部完成索引，无需用户离开对话。

仅当 MCP 的 `reindex` 调用失败（例如连接断开、超时）时，才提示用户在终端手动执行：

```bash
codeconnect index -p . -f
```

### 注意响应里的索引陈旧度

每次响应都带 `meta.index_staleness_ms`，表示索引距上次更新多久。

- 值很小（几秒~几分钟）：结果可信。
- 出现 `warnings` 提示索引已过期：**先调用 `reindex` 再采信结果**，
  否则你可能在基于旧代码下判断。
- 显示「索引时间未知」：索引由旧版本构建，跑一次 `reindex` 即可消除。

## Step 2: 按需选工具

| 我想做什么 | 用哪个 MCP 工具 |
|-----------|----------------|
| 对话被压缩后重建项目认知 | `get_project_map(budget_tokens?)` |
| 找某个函数/类 | `search_symbol(query, language?, kind?)` |
| 看函数怎么写的 / 签名文档 | `get_symbol("符号名或ID")` —— 含源码片段 |
| 谁调用了它 | `trace_callers("符号名或ID")` |
| 它调用了什么 | `trace_callees("符号名或ID")` |
| 改它会影响什么 | `analyze_impact(["符号名或ID", ...])` |
| 全局查找引用 | `find_references("符号名或ID")` |
| 文件里有什么 | `get_file_symbols(file_path, include_source?)` |
| 这个文件复杂吗 | `get_metrics(file_path?)` |
| 有没有死代码 | `detect_dead_code(entry_points?)` |

### 什么时候用 `get_project_map`

- 上下文被压缩、刚接手项目、或要评估「这个改动涉及哪几个模块」时。
- 它按 `budget_tokens`（默认 3000）返回目录结构、模块与关键符号，超预算会自动降级
  并在 `warnings` 里说明。
- 结果同时写入 `.codeconnect/PROJECT_MAP.md`。
- 想聚焦某块代码时传 `focus="crates/index"`，比全量地图更省且更准。

### 已知边界（别误用）

- `find_references` 目前**只覆盖调用关系**。字段读取、类型标注这类引用查不到，
  那种需求请改用 `search_symbol`，不要退回 grep。
- `semantic_search` 已是**真向量检索**（本地 ONNX 模型，查询串嵌入后与符号向量做余弦相似），
  但模型是**可选配置**：没配 `[semantic]` 时它只回「未配置向量模型，语义检索不可用」，
  **不会退回名称匹配**。响应里的 `retrieval.used` 标明本次实际用了哪种检索，别当成语义结果用。
  要按名字精确查找请直接用 `search_symbol`。
- `check_arch_rules` 的完整规则验证尚未接通参数，别指望它给出架构违规结论。
