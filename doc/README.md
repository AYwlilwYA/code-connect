# CodeConnect 文档索引

> 关键词：文档索引 doc spec 设计文档 需求记录 踩坑记录

**修改代码前先翻这里**，确认该主题是否已有设计文档或历史结论。

## 目录约定

| 目录 | 用途 |
|---|---|
| `doc/spec/` | 需求设计与缺陷根因分析（本目录，改代码前必读） |
| `docs/` | 面向使用者的说明文档（MCP 接入教程等） |

## 设计与缺陷文档

| 文档 | 主题 | 状态 |
|---|---|---|
| [2026-09-23-mcp-index-dir.md](spec/2026-09-23-mcp-index-dir.md) | 支持指定 MCP 索引目录；`-p` 未贯穿配置层、`workspace.roots` 死字段、`mcp-setup` 与文档不一致 | 已实施 |
| [2026-09-23-ai-usage-pain-points.md](spec/2026-09-23-ai-usage-pain-points.md) | 索引陈旧度回传、搜不到给候选、`symbol_id` 兼容传名字、`language`/`kind` 过滤下推、响应体积瘦身 | 已实施 |
| [2026-09-23-context-loss-and-lookup.md](spec/2026-09-23-context-loss-and-lookup.md) | 对抗 compact 上下文丢失（项目地图落盘）、grep 不准（引用边为死代码）、多次 read（源码字段恒空） | 已实施 |
| [2026-09-24-tool-output-limits.md](spec/2026-09-24-tool-output-limits.md) | 工具输出上限：地图「文件全量/响应预算」分工、8 个无上限工具补 `limit`；附带发现 Rust 重复符号与死代码误判 | 已实施（缺陷 C/D 待定） |
| [2026-09-24-cpp-symbol-query-incomplete.md](spec/2026-09-24-cpp-symbol-query-incomplete.md) | **C++ 类/方法/虚函数全部漏索引**：`queries/cpp/symbols.scm` 是 C 查询的逐字节副本；`references` 的行号「不准」实为**模糊匹配到了别的符号** | 待确认 |

## 使用者文档

- [MCP 接入配置教程](../docs/mcp-setup.md)
- [项目 README](../README.md)
