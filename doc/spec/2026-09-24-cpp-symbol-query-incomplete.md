# Spec: C++ 符号查询不完整 —— 类/方法/虚函数全部漏索引

> 关键词：C++ cpp symbols.scm class_specifier qualified_identifier field_declaration
> 虚函数 行号不准 references 模糊匹配 错误符号 tree-sitter-cpp

- 状态：**待用户确认**
- 日期：2026-09-24
- 范围：仅 C++ 符号提取（`queries/cpp/symbols.scm` 与其解析器）；不涉及调用边、引用边

---

## 一、现象（用户反馈）

> `references` 的行号对 C++ 虚函数不准（给了 6531 而不是真实定义处），
> 只能当「有没有引用」用，别当准确位置。

## 二、根因（已实测锁定）

**`queries/cpp/symbols.scm` 是 `queries/c/symbols.scm` 的逐字节副本**：

```
$ diff -q queries/c/symbols.scm queries/cpp/symbols.scm
>>> 两个文件完全相同
```

该文件的文件头也写着「**C 符号查询**」。于是它只覆盖 C 的语法形态：

| 查询里的模式 | 覆盖 | 漏掉的 C++ 形态 |
|---|---|---|
| `function_definition` + `declarator: (identifier)` | 只有**裸标识符**的自由函数 | **`Shape::area()` 这类 `qualified_identifier` 限定名定义全部漏** |
| `struct_specifier` | C 风格 `struct S {}` | **`class_specifier`（`class Shape {}`）全部漏** |
| `enum_specifier` / `union_specifier` / `preproc_def` / `type_definition` | C 的对应形态 | —— |
| **完全没有** | | **`field_declaration`** —— 类内方法声明、**虚函数声明**全部漏 |

解析器侧（`crates/parser/src/cpp.rs:127`）认的 capture 名（`@func`/`@struct`/…）
与查询文件是**一致**的 —— **问题不在命名不匹配，在查询覆盖的语法节点太少**。

### 实测证据

**证据 1：两个典型 C++ 文件只提取出 1 个符号**

```
扫描文件数: 2    提取符号数: 1
```
源文件里有 `Shape`、`Circle`、`Shape::area`、`Circle::area`、`compute_area` 至少 5 个符号。
唯一被提取的是**裸标识符的自由函数** `compute_area`。

**证据 2：按名字查全部落空**

| 查询 | 结果 |
|---|---|
| `Shape` | **无** |
| `Circle` | **无** |
| `~Shape`（析构） | **无** |
| `area` | 只**模糊匹配**到 `compute_area` |

**证据 3：决定性对照 —— 换成 C 风格 `struct`**

```cpp
struct S {
    virtual double area() const;
};

double S::area() const { return 1.0; }   // 第 5 行

double free_fn(int x) { return x + 1; }  // 第 7 行
```

```
扫描文件数: 1    提取符号数: 2
  S        ->  S  [struct]                  ← struct_specifier 能提取
  area     ->  free_fn  [function] t.cpp:7  ← S::area 仍然漏；模糊匹配到 free_fn
```

`S` 提取到了（证明 `class_specifier` 是缺口），但 **`S::area()` 仍然不在索引里**
（证明 `qualified_identifier` 是另一个缺口）。

## 三、症状机制：不是"行号算错"，是"查错了符号"

用户看到的是「行号不准」。**真实机制是符号压根不存在，检索退化成模糊匹配**：

```
search/references <方法名>
  → 精确/前缀匹配全落空
  → 模糊匹配到另一个符号（如 compute_area / free_fn）
  → 返回该符号的**真实**位置
  → 用户看到「行号 6531，不是我要的定义处」
```

**那个位置本身是准确的，只是属于另一个符号。** 所以：
- **不能**当作"位置记录有 bug"去查坐标计算 —— 方向会全错。
- **危险点**：`references` 会给出一个**看起来正常**的符号名与行号，**不报错、不告警**，
  调用方很容易把它当成"找到了"。

## 四、影响面

1. **C++ 类、方法、虚函数、构造函数、析构函数、namespace、template** 全部不在符号索引里。
2. 因而 `search` / `references` / `call-graph` / `detect_dead_code` / `get_metrics` 等
   **所有依赖符号表的工具，对 C++ 类成员一律失效**。
3. 与已知的「限定静态调用 `Class.staticMethod()` 不建调用边」是**两个独立问题**：
   本例是**符号都没进来**，那个是**符号在但边没建**。

## 五、修复方向（待确认后实施）

1. **重写 `queries/cpp/symbols.scm`**，按 tree-sitter-cpp 的真实节点补：
   - `class_specifier`（`class`）与 `struct_specifier`（C++ 也用它，但语义是类）
   - `namespace_definition`
   - `function_definition` 的 `declarator` 支持 `qualified_identifier`
     （`Shape::area`）+ `field_identifier`（类内定义）
   - `field_declaration`（类内方法声明 / **虚函数**）
   - `template_declaration`、构造函数/析构函数（`~Shape`）
   - `enum_specifier` 的 `enum class` 形态
2. **解析器侧同步**：`cpp.rs` 的 capture 名匹配表要能表达
   Class / Interface / Method / Constructor 等 `SymbolKind`，
   而不只是 Function / Struct / Enum / Macro / TypeAlias。
3. **加防护**：检索退化到模糊匹配时，响应应**明确标注「未精确命中，以下为相近符号」**
   —— 当前 `search_symbol` 已有 `describe_similar_symbols()`，但 CLI 的
   `search` / `references` 路径需要确认是否同样标注。
   **不标注就等于默许调用方误用。**

## 六、验收标准

1. 建 C++ fixture（含 class / 虚函数 / 派生覆写 / 构造函数 / 析构 / namespace / template），
   索引后**每个符号都能按名字精确查到**、且行号与源码逐一对上。
2. `references <虚函数名>` 不再退化成匹配到无关符号。
3. 现有 `cargo test --workspace` 全绿；C++ 相关的既有测试不回归。
4. 用真实的大 C++ 仓库（用户提供）复跑，确认 6531 那类错位消失。

## 七、风险

| 风险 | 应对 |
|---|---|
| 查询模式写宽了会产出重复符号 | 参考 Rust 侧踩过的坑（`impl_item` 内方法被匹配两次），**必须写去重单测** |
| `SymbolKind` 扩展会牵动多处 match | 先只做「不漏符号」，新 kind 尽量复用现有变体，避免大范围改动 |
| 用户的实际仓库未提供 | 先用自建 fixture 验收；**最终需用真实仓库复现 6531 那例** |

---

## 八、修复结果（2026-09-24）

改动仅 2 个文件：`queries/cpp/symbols.scm`、`crates/parser/src/cpp.rs`
（`queries/c/`、`c.rs`、`factory.rs` 一字未动）。

### 8.1 AST 实测结论（先 dump 再写查询）

| 源码形态 | 节点结构 |
|---|---|
| 自由函数 | `function_definition` → `function_declarator` → `identifier` |
| **类内直接定义** | `function_definition` → `function_declarator` → **`field_identifier`** |
| **类外限定名定义** `Shape::area` | `function_definition` → `function_declarator` → **`qualified_identifier`** |
| **类内声明/纯虚/override** | `field_declaration` → `function_declarator` → **`field_identifier`** |
| 构造/析构声明 | 是 `declaration`（**不是** `field_declaration`）→ `identifier` / **`destructor_name`** |
| 模板特化 `class Box<int>` | `class_specifier` 的 `[name]` 是 **`template_type`** |

**两个凭记忆写不出来的坑**：
1. `int* f()` 的 declarator 是 `pointer_declarator`（子节点**带** `declarator` 字段），
   但 `int& f()` 的 `reference_declarator` 子节点**不带**该字段 —— 只按字段下探会**漏掉全部返回引用的函数**。
2. `int (*fp)(int);` 顶层 declarator 也是 `function_declarator` —— 不校验内层节点类型会把**变量**当成函数。

### 8.2 去重（本项目踩过的坑，专门加固）

`template_declaration` **故意不单独产出符号**（模板类/函数已被其他模式天然覆盖，
再加一条会直接制造重复 —— 正是 Rust 侧 `impl_item` 踩过的坑）。

`cpp.rs` 新增 `definition_rank()` 去重表，**定义(3) > 类内声明(2) > 前置声明(1)**，
同名同类只产出一份且**保留定义处**（避免"声明行 vs 定义行"再次被当成行号不准）。
新增 6 个防重复测试，含一个把 11 条 fixture 全塞进去、用 `HashSet<(name, kind)>` 断言任意组合都不重复的综合用例。

### 8.3 实测

| 语料 | 改动前 | 改动后 |
|---|---|---|
| 自建 fixture（2 文件） | 5 | **28**（逐行对上，0 错位） |
| **一个真实开源 C++ 仓库（217 文件）** | 893 | **2657** |
| 本项目自索引（76 文件，纯 Rust/TS） | 3510 | 3510（**非回归**） |

性能：同一机器背靠背 release 对照，**+5% 耗时换 3 倍符号**。
`cargo test --workspace`（除 diff）**275 passed / 0 failed**。

---

## 九、⚠️ 遗留阻塞：`.h` 文件走的是 **C 解析器**，本次修复对它无效

`crates/parser/src/factory.rs:58`：

```rust
"c" | "h" => "c",
"cpp" | "hpp" | "cc" | "cxx" | "c++" | "h++" | "hh" | "hxx" => "cpp",
```

**`.h` 一律交给 C 解析器（`queries/c/`）**。C++ 项目大量把类写在 `.h` 里 ——
**本次修复只对 `.cpp`/`.hpp` 生效，对 `.h` 完全无效。**

### 决定性对照（同一份真实头文件，**只改扩展名**，内容完全相同）

| 符号 | 存成 `.h`（走 C） | 存成 `.hpp`（走 C++，新查询） |
|---|---|---|
| 5 个虚函数（改前碰巧被错误恢复提取到） | ✅ | ✅ |
| **另外 5 个虚函数（改前完全查不到）** | ❌ | ✅ |

同一个文件里「有的能查到、有的查不到」的谜团就此解开。
（具体符号名涉私有代码，不在此列举；验收用的是真实项目头文件。）

### 这也解释了「同一个 struct 内为什么选择性提取」

C 解析器遇到 `virtual` 这种**非法 C 语法**会产生 ERROR 节点，**错误恢复碰巧把哪些成员
flatten 成合法的 C 函数定义，哪些就被提取** —— 纯属运气，**不是** `void`/`const char*` 之类的语义规则
（那个相关性是巧合；已用最小 fixture 证伪四次：标识符语言、`.h`/`.cpp`、截断闭合、返回类型，**全部不成立**）。

用户的仓库里 **108 个 `.h` 被归为 C、33 个归为 C++** —— 即**大部分 C++ 代码没被正确解析**。

### 可选修法（待用户确认）

| 方案 | 说明 | 风险 |
|---|---|---|
| **A. `.h` 映射改 `cpp`** | tree-sitter-cpp 是 C 的超集，C 头文件同样能解析 | 会改变**纯 C 项目**的行为；`queries/cpp/calls.scm`、`imports.scm` 与 C 版不同，调用/导入抽取会随之变化 |
| **B. 按项目语言判定** | 项目里有 `.cpp` → `.h` 按 C++ 解析；否则按 C | 需在索引流程里传上下文，改动面较大 |
| **C. 配置项** | 由用户在 `.codeconnect.toml` 指定 `.h` 归谁 | 用户需知情，默认行为仍错 |

**另发现（同级问题，未改）**：`queries/c/symbols.scm` 的 `function_definition` 强制要求 `body:`，
因此 **C 的函数原型 `void proto_only(int, double);` 不进索引** —— C 头文件是原型密集区，影响与本次同级。
