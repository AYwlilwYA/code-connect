---
name: code-connect
description: CodeConnect 多语言代码分析工具。当需要搜索代码符号、追踪函数调用链、检测死代码、评估代码质量指标、查找引用位置、分析变更影响时使用。适用于任何项目的代码理解、重构评估和代码审查场景。
---

# CodeConnect — 代码分析工具

CodeConnect 是一个多语言代码分析 MCP 服务器，支持 Rust、TypeScript、JavaScript、Java、C#。本 skill 指导如何使用其 MCP 工具来分析项目代码。

## 前置条件

在使用 MCP 工具之前，确保：
1. **索引已构建** — 在项目根目录运行 `codeconnect index -p . -f`
2. **MCP 已连接** — `/mcp` 中 codeconnect 状态为 connected
3. 已有 `.codeconnect.toml` 配置文件（可通过 `codeconnect mcp-setup` 一键配置）

## 工具速查

### 搜索与定位
- `search_symbol(query, kind?, language?, limit?)` — 按名称搜索符号
- `semantic_search(description, language?, limit?)` — 按自然语言描述搜索
- `get_symbol(symbol_id)` — 获取符号完整信息（签名、文档、位置）
- `get_file_symbols(file_path)` — 列出文件内所有符号
- `find_references(symbol_id, limit?)` — 查找符号的所有引用位置

### 调用链分析
- `trace_callers(symbol_id, max_depth?)` — 反向追踪：谁调用了它
- `trace_callees(symbol_id, max_depth?)` — 正向追踪：它调用了谁
- `get_call_graph(symbol_id, caller_depth?, callee_depth?)` — 双向局部调用图
- `analyze_impact(symbol_ids, max_depth?)` — 修改影响范围评估
- `get_type_hierarchy(symbol_id, direction?)` — 类型继承链

### 代码质量
- `get_metrics(symbol_id?)` 或 `get_metrics(file_path?)` — 圈复杂度、扇入扇出
- `detect_dead_code(entry_points?)` — 从入口点出发检测不可达代码

### 项目管理
- `get_index_status(verbose?)` — 索引统计与语言分布
- `list_files(language?, limit?, offset?)` — 分页列出已索引文件
- `reindex(full?, file_paths?)` — 触发增量或全量索引
- `get_dependency_graph(level?, file_path?)` — 文件/模块级依赖图
- `check_arch_rules(rule_names?)` — 验证架构约束规则

## 典型工作流

### 流程 1：快速了解一个函数

```
1. search_symbol(query="函数名")       → 找到符号，复制它的 symbol_id
2. get_symbol(symbol_id="<id>")       → 查看完整签名、文档
3. trace_callees(symbol_id="<id>")    → 它调用了什么
4. trace_callers(symbol_id="<id>")    → 谁在调用它
```

### 流程 2：评估修改风险

```
1. search_symbol(query="函数名")       → 找到要改的符号
2. analyze_impact(symbol_ids=["<id>"]) → 看影响哪些代码
3. trace_callers(symbol_id="<id>")     → 确认所有上游调用
```

### 流程 3：清理死代码

```
1. detect_dead_code(entry_points=["src/main.rs"]) → 从入口出发找死代码
2. 逐个检查标记为 dead 的符号，确认是否真的无用
```

### 流程 4：文件级代码审查

```
1. get_file_symbols(file_path="src/xxx.rs")  → 列出文件所有符号
2. get_metrics(file_path="src/xxx.rs")       → 查看复杂度分布
3. 对高复杂度函数执行 trace_callees/analyze_impact
```

### 流程 5：查找所有使用位置

```
1. search_symbol(query="类型名/函数名")     → 搜索符号
2. find_references(symbol_id="<id>")         → 全局查找引用
```

## 重要提示

- **symbol_id 是关键**：大多数分析工具需要 `symbol_id` 参数。先从 `search_symbol` 获取，结果中直接包含可复制的 `id` 字段
- **先 index 后分析**：如果 MCP 工具返回空结果，先确认索引已构建且文档数 > 0
- **索引锁问题**：如果 `index` 或 `serve` 报 `LockBusy`，说明有残留进程，执行 `taskkill /F /IM codeconnect.exe`（Windows）后重试
- **跨文件分析的限制**：调用图基于项目内已索引的符号，外部依赖的调用不会被追踪
