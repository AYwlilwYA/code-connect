# Spec: 符号签名/文档注释提取 + 索引「替换而非追加」

> 关键词：signature doc_comment 为 null 解析器未填充 重复索引文档翻倍 delete_term
> TantivyIndex 没有删除 全量索引追加而非替换 增量索引旧符号残留 H4
> symbols upsert 幂等 reindex

- 状态：**已批准实施**
- 日期：2026-09-25
- 范围：① 解析器补 `signature` / `doc_comment` ② 索引写入改为替换语义（含增量路径的删除）

---

## 一、问题 1：`signature` / `doc_comment` 恒为 null

### 现象

`semantic_search` 的嵌入本应是「符号名 + 签名 + doc 注释首行」，
实测**索引里这两个字段全是 null** —— 嵌入实际退化成**只有符号名**，
明显拖累中文自然语言召回。这是当前语义检索质量的**最大限制**。

### 根因（实测）

字段本身是好的，索引侧读取也对，**只是没有任何解析器去填**：

```
crates/parser/src/rust.rs:263        signature: None,
crates/parser/src/rust.rs:264        doc_comment: None,
```

**7 个解析器（rust / typescript / javascript / java / csharp / c / cpp）全部如此**，
一个不落。索引侧 `full_indexer.rs:593-594` 是
`symbol.signature.as_deref().unwrap_or("")` —— 正确，只是上游永远是 `None`。

### 设计

各解析器在产出 `Symbol` 时填充：

- **`signature`**：从声明节点取**签名文本**（参数列表，必要时含返回类型），
  做成**单行、规范化空白**的形式（换行折叠成空格），避免把整段声明塞进索引。
  **不含函数体**。
- **`doc_comment`**：取**紧邻声明之上**的文档注释，**只取首行或前若干行**（建议上限 200 字符）。
  各语言形态不同，需分别处理：
  - Rust `///`、`//!`、`#[doc = "..."]`
  - Java / C# / TS / JS `/** ... */`（C# 另有 `///`）
  - C / C++ `/** ... */`、`///`
  - 无文档注释时留 `None`，**不要用普通注释冒充**

**要点**：注释与声明之间**隔着空行或别的声明**时不算文档注释 —— 这是最容易写错的地方，
必须写单测钉住。

---

## 二、问题 2：重复 `index` 让文档翻倍（且增量路径残留旧符号）

### 现象 A：不加 `-f` 重跑 `index`，同一符号出现两次

实测：同一文件同一行同一符号，索引里两份。会把「符号命中数」抬高，
进而**让文本真值回显的「充足 / 稀少」档位判断失真**。

### 现象 B：增量索引产出重复符号（即此前未解决的 H4）

文件改动后重索引，**旧符号记录永远留在索引里**，与新增记录并存。

### 根因（实测，A 与 B 是同一个）

- `TantivyIndex` 与 `CallEdgeIndex` **连一个删除方法都没有**
  —— 公开 API 只有 `add_*` / `search_*` / `commit` / `doc_count`
- `crates/index/src/incremental.rs:287` 的 `remove_file_from_index`
  **只删 sled 的元信息与指纹，完全不碰 tantivy**
  （注释里也承认了这一点）
- `full_indexer.rs` 同样是**纯追加**：`-f` 只是 `remove_dir_all` 删目录，
  不给 `-f` 就一路 `add_document`

⇒ **两个现象是同一个缺失：索引没有「删」这个动作。**

### 设计

1. **给两个 tantivy 索引补删除能力**
   ```rust
   impl TantivyIndex {
       /// 按文件路径删除该文件的所有符号文档；返回删除前匹配到的数量
       pub fn delete_by_file_path(&self, file_path: &str) -> Result<u64, CodeConnectError>;
   }
   impl CallEdgeIndex { /* 同理，按 caller 文件路径 */ }
   ```
   用 `IndexWriter::delete_term(Term::from_field_text(schema.file_path, path))`；
   注意 **tantivy 的删除在 `commit()` 时才生效**，调用方需知道这一点。
   同时提供 `delete_all_documents()` 的封装（见下）。

2. **全量索引改为「替换」语义**：`FullIndexer::run()` 在写入前
   **先清空符号索引与调用边索引**（或按文件逐个 delete 后再 add）。
   `index` 必须**幂等** —— 跑一次和跑十次结果相同。

3. **增量路径补删除**：`remove_file_from_index`（以及 `write_file_index` 之前）
   必须**先删该文件的 tantivy 文档再重新写入**，否则旧符号永远残留（现象 B）。

4. **`-f` 的语义收窄**：既然全量索引本身已是替换，`-f` 的差别只剩
   「是否连 sled 元信息/指纹一起清」。**在报告里说明你最终怎么定的**，
   并确认两种方式都**不会**产生重复。

---

## 三、验收标准

### 问题 1
1. 7 个语言各建一个含「带参数函数 + 文档注释」的 fixture，
   索引后 `search_symbol detail=full` 能拿到**非空**的 `signature` 与 `doc_comment`。
2. 签名是**单行规范化**的（无换行），不含函数体。
3. **注释与声明之间隔空行 / 隔别的声明时，`doc_comment` 必须为 `None`**（单测钉住）。
4. 无文档注释时是 `None`，不得用普通注释冒充。

### 问题 2
5. **幂等性**：同一个项目连续跑 `index` 两次（不加 `-f`），
   **符号总数与 doc_count 完全一致**，且同一 `(file, line, name)` 不出现两次。
6. **增量不残留**：改一个文件后触发增量索引，
   该文件的符号数等于**新内容的符号数**（不是新旧之和）。
7. 既有 `cargo test --workspace` 全绿；**本项目自索引**在跑两次 `index` 后数量稳定。

## 三之二、实施结果与已确认的取舍（2026-09-25 收尾）

### 已达成
- **问题 1**：7 个语言全部填充 `signature` / `doc_comment`，逐语言 fixture + 端到端
  （真建索引 → MCP `search_symbol detail=full`）均已验证。**`crates/mcp/src/` 一行未改** ——
  透传链路本来就通，只是上游从没灌过数据。
- **问题 2**：`TantivyIndex` / `CallEdgeIndex` 补上 `stage_delete_by_file_path` /
  `delete_by_file_path` / `delete_all_documents`；全量索引改为**先清空再写**（幂等），
  增量索引改为**先标删再写、与新增共用同一次 commit**（整批原子替换）。
- `cargo test --workspace`（排除 diff 后）**363 passed / 0 failed**。

### 取舍 1：解析失败的文件，旧符号会从索引里消失 —— **用户已确认接受**
全量索引是「替换」语义：清空后，解析失败的文件不会被重新写入，其旧符号随之消失
（旧的「追加」语义下旧符号会残留）。
- 缓解：`IndexStats.failed_files` 会列出失败文件，CLI 打印；
  另有**「扫描到 0 个文件但索引非空 → 报错中止、不碰索引」**的闸门挡住最坏情况。
- 注意：**增量路径不受此影响** —— `incremental.rs:220-221` 刻意「先解析成功、再删旧数据」，
  解析失败直接 `continue`，旧数据保留。

### 取舍 2：全量索引中途失败会停在「索引已空」
清空是一次 commit、写入是另一次，两者不在同一事务里。

### 取舍 3：增量路径的窄缝（已知、未加兜底）
若 `write_file_index` 返回 Err，`?` 提前返回，本批次已标记的删除不提交；
它们会挂在 `IndexWriter` 里，被**之后某个不相关批次**的 commit 顺带提交。
触发条件是 tantivy add / serde 失败，极罕见。未加 `IndexWriter::rollback`（会扩大 API 面）。

### 附带修复：`codeconnect-diff` 链接失败
`libgit2-sys 0.17.0+1.8.1` 的 build.rs 在 Windows 上**漏声明 `advapi32`**，
而 Rust 1.95 的 std 已不再默认链接它 ⇒ 19 个 `__imp_Crypt*` / `__imp_Reg*` 未解析符号、
`LNK1120` 失败，`codeconnect_diff-*.exe` **从未成功链接过**，`cargo test --workspace` 因此无法全绿。
修复：新增 `crates/diff/build.rs`，在 `CARGO_CFG_TARGET_OS == "windows"` 时
`cargo:rustc-link-lib=advapi32`。

### 附带修复 2：链路修好后暴露的 hunk 解析缺陷
**这两个问题是连锁的** —— 链接修好、测试第一次真的跑起来，立刻暴露 3 个失败：

| 测试 | 输入 | 实测 end | 应得 end |
|---|---|---|---|
| `test_parse_hunk_header_with_count` | `@@ -1,5 +1,10 @@` | 1 | 10 |
| `test_parse_hunk_header_context` | `@@ -10,7 +10,6 @@ fn main()` | 10 | 15 |
| `test_parse_diff_hunks_multi_file` | `@@ -1,3 +1,5 @@` | 1 | 5 |

**症状指纹**：`end` 恒等于 `start`，而**不带 count 的用例是通过的**
（`@@ -5 +10 @@` 本就 start == end）⇒ 说明 count 被忽略。

**根因**（`crates/diff/src/symbol_diff.rs:201`）：提取数字部分时
`find(|c| !c.is_ascii_digit())` 在**逗号**处就截断，`num_part` 只剩 `"1"`，
下面 `find(',')` 落空 → 走「只有起始行号」分支 → `end = start`。
**逗号是数字部分的一部分**（start 与 count 的分隔符）。

**修法**：`find(|c: char| !c.is_ascii_digit() && c != ',')`（注意 `c` 是按值的 `char`，不能写 `*c`）。

**语义确认**：测试期望 `end = new_start + new_count - 1`，即**新文件侧**行号 ——
这是对的，因为 `compute_overlap` 要拿它跟现行代码的符号行号比对。

**仍存的脆弱点（未修）**：`parse_hunk_header` 用 `header.rfind('+')` 取最后一个 `+`
定位新文件侧，hunk 上下文里若含 `+`（如 `@@ -1,3 +1,5 @@ a + b`）会定位错。
当前测试未覆盖该形状。

**影响面**：**没有任何 crate 依赖 `codeconnect-diff`**，尚未接进生产路径，
故以上缺陷实际影响面目前为零 —— 属埋雷，接入前须先修。

### 附带修复 3：MCP `reindex` 的两个空壳参数（响应在说谎）
排查中发现 `handle_reindex` 向调用方**承诺了两个并不存在的能力**：

| 参数 | 文档承诺（`schemas.rs:329-336`） | 实际 |
|---|---|---|
| `file_paths` | 「指定要重新索引的文件路径列表」 | 在 `crates/mcp/src/` 里**除声明外再无任何出现**，完全不读 |
| `full` | 「默认 false，即增量更新」 | 在 `handle_reindex` 里**只出现一次**（就是那句回报），对行为零影响 |

`handle_reindex`（`tools.rs:1918-1928`）**无条件调用 `FullIndexer::run()`**，
没有增量分支。于是调用方不传 `full` 时，服务端干的是全量重建，
回报却是 `"mode": "incremental"` —— **在该项目「响应绝不能误导」的命题下这是假话**。

**修法（如实回报，不实现新功能）**：
- `tools.rs` 的 `mode` **恒报 `"full"`**，并注明该参数对行为无影响；
- `schemas.rs` 两个字段的文档改为**如实说明尚未实现 / 无影响**，字段保留以免破坏调用方；
- 函数参数改名 `_params`（改动后该参数在函数内已无使用）。

真正的「按文件重索引 / 增量 reindex」**未实现**，需另开一轮设计与实施。

## 四、风险

| 风险 | 应对 |
|---|---|
| tantivy 删除在 commit 时才生效，漏 commit 会「删了但没删掉」 | 删除路径必须紧跟 `commit()`，并**实测**验证 doc_count 真的下降 |
| 全量索引清空后中途失败 → 索引空 | 报告里说明取舍；`-f` 本来就是删目录，风险等级相同 |
| 签名提取把函数体也带进去 | 单测断言签名**不含 `{`**、长度上限 |
| doc 注释误取到普通注释 | 单测覆盖「隔空行 / 隔声明」两种反例 |
| 7 个语言的注释形态差异大 | **每个语言独立 fixture 验证**，不做「改完一起测」 |
