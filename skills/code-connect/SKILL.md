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

## Step 1: 确认索引就绪

调用 `get_index_status(verbose=true)`。若 `indexed_documents = 0`，去终端跑：

```bash
codeconnect index -p . -f
```

## Step 2: 按需选工具

| 我想做什么 | 用哪个 MCP 工具 |
|-----------|----------------|
| 找某个函数/类 | `search_symbol` → 拿到 symbol_id |
| 看函数签名和文档 | `get_symbol(symbol_id)` |
| 谁调用了它 | `trace_callers(symbol_id)` |
| 它调用了什么 | `trace_callees(symbol_id)` |
| 改它会影响什么 | `analyze_impact(symbol_ids)` |
| 全局查找引用 | `find_references(symbol_id)` |
| 文件里有什么 | `get_file_symbols(file_path)` |
| 这个文件复杂吗 | `get_metrics(file_path?)` |
| 有没有死代码 | `detect_dead_code(entry_points?)` |
