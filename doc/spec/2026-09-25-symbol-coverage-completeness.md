# Spec: 符号覆盖完整性 —— 枚举量缺失与「静默缺口」机制

> 关键词：枚举量 enumerator SymbolKind Constant 静默缺口 符号完整性 覆盖度审计
> 查不到当不存在 定长表 崩溃 symbols.scm 词汇表 grammar 驱动 豁免名单

- 状态：**已批准实施（A+B+C）**
- 日期：2026-09-25
- 范围：符号**覆盖完整性**这一整类问题；不涉及调用边、引用边

---

## 一、现象（用户反馈，第三次同类）

> `enum` 的枚举量（enumerator）不建索引。而且这条 0 比前两条更毒：
> `references <枚举类型>` 也是 0 —— 没人写 `骨骼点 x;` 这种类型名引用，
> 使用全落在枚举量上。那个 0 会被读成「没人用这个枚举，加删一项很安全」，
> 而这次恰好就是加枚举项，**后果是崩**。

### 复现（最小 fixture）

```cpp
enum class Dir { Up, Down };
enum Color { RED, GREEN, BLUE };
struct S { int x; };
int use_it() { return RED + (int)Dir::Up; }
```

```
提取符号数: 4            ← Dir[enum] Color[enum] S[struct] use_it[function]
Up / RED / GREEN / BLUE  → 全部查不到
references RED           → 引用数 0     （源码第 4 行就写着 RED）
```

## 二、根因（三条实测事实）

### 2.1 8 个语言的 `symbols.scm` **没有一个**捕获 enumerator

```
rust 0 / typescript 0 / javascript 0 / java 0 / csharp 0 / cpp 0 / kotlin 0
c 出现 1 次 —— 但那是 (enumerator_list)，是枚举的「体」，不是体里的项
```

### 2.2 `SymbolKind` **没有「枚举量/常量」这个概念**

现有变体：`Function / Method / Class / Interface / Struct / Enum / Trait /
TypeAlias / Variable / Field / Parameter / Module / Macro / Unknown`。

**数据模型里不存在这个类别** ⇒ 即使有人写了查询也**没地方放**。
这不是「查询漏了一条」，是**概念层就缺**。

### 2.3 全项目**没有任何测试断言「符号完整性」**

所有解析器测试都是**正向**的（「这些特定的东西**能**找到」），
**没有一个**是**穷尽**的（「源码里有的**都**得进索引」）⇒ 每个缺口都不可见，
直到有人在生产里撞上。

### 2.4 结构性归因（这才是「为什么又出现」的答案）

`symbols.scm` 本质是一份**手写的、定义「什么算一个符号」的词汇表**。
它**从来没有从 grammar 系统性推导过** —— 没有任何工序是：

> 把这个 grammar 里所有**能产生声明**的节点列出来，逐个确认：**要么捕获，要么显式记为故意排除**。

靠人想，就必然漏，且**只漏到被发现为止**。

**更致命的是接口设计**：系统里「**找不到**」与「**不存在**」在返回值上**完全同形** ——
工具回 0 时**不带任何警告**，而 0 是一个**看起来有意义的答案**（"没有调用方 / 没人用"）。

三次事故是同一个机制的不同外化：

| 次 | 缺口 | 那个 0 被读成 | 真值 |
|---|---|---|---|
| 1 | 索引过期 | 「这个类方法不存在」 | 存在，只是没索引 |
| 2 | 限定名 `Class::method()` 不建边 | 「没有调用方，改签名很安全」 | 有调用方（TS 7 处 / C++ 203 处） |
| 3 | 枚举量不索引 | 「没人用这个枚举，加一项很安全」 | 有 15 处使用；**漏改定长表 ⇒ SIGSEGV** |

## 三、设计

### A. 补概念 + 各语言捕获 enumerator

1. `SymbolKind` 新增 **`Constant`**（枚举量与常量；本轮的落点是**枚举量**）。
2. 8 个语言的 `symbols.scm` 增加 enumerator 捕获，**统一 capture 名 `@enumerator`**。
3. 各解析器的 capture 表把 `@enumerator` 映射为 `SymbolKind::Constant`。
4. 下游 kind 字符串统一为 **`"constant"`**（落盘、过滤、MCP 输出一致）。

**接口契约定死**（两个 agent 并行实施的前提）：

```rust
SymbolKind::Constant          // 新变体
"@enumerator"                 // 查询里的 capture 名（8 个语言统一）
"constant"                    // 落盘/输出用的 kind 字符串
```

### B. ★ 让「没索引」与「不存在」**可区分**（本 spec 的核心）

**这是唯一能防住第四、第五次的一层。** 前三次共同的致命点不是「漏了某类符号」，
是「**漏了却装成「没有」**」。

**做法**：解析每个文件后，统计该文件 AST 里**能产生声明的节点种类**（declaration node kinds），
与实际**产出了符号的种类**做差集。对差集里的每一种：

- 若它在**已知豁免名单**里（如注释、预处理指令片段）→ 静默
- 否则 → 记入该文件的 `uncovered_declaration_kinds`，并**上浮到索引统计与工具响应**

**响应形态（草案）**：

```json
{
  "coverage": {
    "declaration_kinds_seen": ["enum_specifier", "function_definition", "class_specifier", "enumerator"],
    "kinds_without_symbols": ["enumerator"],
    "note": "本文件含 2 个枚举体，产出 0 个枚举量符号 —— 对该类型的查询会给出「不存在」的假象"
  }
}
```

**为什么要有 `note` 这句人话**：调用方（AI）看到的是工具输出，
必须**在它会误读的地方**直接写明「这里返回的 0 不代表不存在」。

**范围控制**：B 先只覆盖**解析层可见的事实**（节点种类 vs 产出种类），
**不**试图判断「某个具体的名字有没有被引用」——那是另一件事。

### C. 完整性测试（grammar 驱动）

针对每个语言：

1. **枚举该 grammar 里能产生声明的节点类型**（可从 grammar 的 node-types 或人工维护的清单来）。
2. 断言**每一种**：要么**被 `symbols.scm` 捕获**，要么**在显式豁免名单里**（带理由注释）。
3. 新增节点类型（grammar 升级）或删掉捕获时，**测试必须失败**。

**豁免名单要有理由**，例如：`comment`（不是声明）、`preproc_include`（不是符号）等。

## 四、改动清单（预估）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/core/src/types.rs` | `SymbolKind::Constant` + 所有穷尽 match 的补全 |
| 2 | `queries/{rust,typescript,javascript,java,csharp,c,cpp,kotlin}/symbols.scm` | 捕获 `@enumerator` |
| 3 | `crates/parser/src/{rust,typescript,java,csharp,c,cpp}.rs` | capture 表补 `@enumerator` → `Constant` |
| 4 | `crates/parser/src/`（新模块，如 `coverage.rs`） | B：声明节点种类覆盖度统计 |
| 5 | `crates/index/src/full_indexer.rs` | kind 字符串 `"constant"`；汇总 `uncovered_declaration_kinds` |
| 6 | `crates/mcp/src/tools.rs` + `schemas.rs` | 响应里如实带出覆盖度告警 |
| 7 | 各解析器测试 | C：grammar 驱动的完整性断言 + enumerator 正向用例 |

## 五、验收标准

1. **枚举量可查**：`search_symbol RED` 命中，`kind` 为 `constant`，行号与源码一致。
2. **8 个语言都覆盖**（各写一个含枚举的 fixture 验证；语言的枚举语法差异由各自用例兜住）。
3. **静默缺口可见**：人为构造一个「有声明节点但查询没覆盖」的场景（可临时注释掉一条捕获），
   索引后**必须**在统计/响应里报告出来，**不能**静默。
4. **完整性测试有效**：删掉任意一条 enumerator 捕获 → **测试失败**。
5. `cargo test --workspace` 全绿；本项目自索引符号数变化可解释（新增枚举量，只增不减）。
6. 不回归：既有 C/C++/Rust/TS 等解析器测试全绿。

## 六、风险

| 风险 | 应对 |
|---|---|
| `SymbolKind` 是跨 crate 的公共枚举，新增变体牵动多处穷尽 match | **编译器会全部指出**；逐个补齐，不放过一个 |
| 枚举量数量大（大项目可能上千），索引体积上升 | 先测本仓库与一个真实 C++ 仓库的增量；如实报告增幅 |
| B 的「declaration node kinds」清单本身可能漏 | 清单**手工维护 + 注释理由**，并由 C 的测试兜住「新增 grammar 节点必须表态」 |
| 8 个语言的查询改动面大 | 每个语言**独立 fixture 验证**，不做「改完一起测」 |
