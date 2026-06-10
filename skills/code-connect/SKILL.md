---
name: code-connect
description: 多语言代码分析 MCP 工具（Rust/TS/JS/Java/C#/C/C++）。启动时先读此 skill，再执行。用于：搜索符号、追踪调用链、查找引用、检测死代码、圈复杂度、变更影响分析。
---

# 先读我，再干活

任何代码分析任务（搜索符号、查调用链、找引用、死代码、复杂度、影响分析），**先用 MCP 工具 `search_symbol` 探一下项目里有什么**，不要上来就 grep 或读文件。

## 前置：确保索引可用

```
get_index_status(verbose=true)
```

如果 `indexed_documents = 0`，去终端跑并等它完成：
```bash
codeconnect index -p . -f
```
然后再回到 MCP 工具使用。

## 工具速查

**搜索定位**
- `search_symbol(query, kind?, language?, limit?)` — 符号搜索，拿到 symbol_id
- `get_symbol(symbol_id)` — 符号完整信息（签名、文档）
- `find_references(symbol_id)` — 全局找引用
- `get_file_symbols(file_path)` — 某文件里有什么

**调用分析**
- `trace_callers(symbol_id)` — 谁在调它
- `trace_callees(symbol_id)` — 它调了谁
- `analyze_impact(symbol_ids)` — 改它会波及什么

**质量**
- `get_metrics(file_path?)` — 圈复杂度、扇入扇出
- `detect_dead_code(entry_points?)` — 从入口出发找死代码
