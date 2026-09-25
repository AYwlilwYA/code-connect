# codeconnect-embed

本地 ONNX 文本嵌入，供 `semantic_search` 做**真向量检索**用。离线、无网络调用。

关键词：嵌入 embedding ONNX ort 模型 download ORT_DYLIB_PATH 离线 语义检索

## 对外契约

```rust
pub fn load(model_dir: &Path) -> Result<Embedder, EmbedError>;      // 任意模型目录路径
pub fn load_source(source: &ModelSource, root: &Path) -> Result<Embedder, EmbedError>;
pub fn load_default() -> Result<Embedder, EmbedError>;              // 默认根目录 + 默认模型名
pub fn resolve_model_dir(root: &Path, name: &str) -> Result<PathBuf, EmbedError>;
pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError>;  // 与输入等长
pub fn dim(&self) -> usize;                                          // 向量维度
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32;
```

返回的向量已做 L2 归一化，因此余弦相似度等价于点积。

## 模型是可选的、也是可指定的

`~/.codeconnect/models` **只是默认根目录**，不是唯一来源。

```rust
pub enum ModelSource {
    NotConfigured,          // 未配置 = 能力关闭（默认状态）
    Named(String),          // 按模型名，在根目录下解析
    Dir(PathBuf),           // 直接给模型目录路径
}
impl ModelSource {
    pub fn from_config(value: Option<&str>) -> Self;  // None/空串→未配置；已存在目录→Dir；其余→Named
    pub fn is_configured(&self) -> bool;
    pub fn resolve(&self, root: &Path) -> Result<PathBuf, EmbedError>;
    pub fn describe(&self) -> String;                 // 回显给 retrieval 元信息用
}
```

配置里就是一个字符串字段（如 `config.toml` 的 `[embed] model = "..."`）：

| 配置值 | 结果 |
|---|---|
| 缺省 / `""` | `NotConfigured` —— 语义检索关闭，工具照常用 |
| `"paraphrase-multilingual-MiniLM-L12-v2"` | 在根目录下解析 `<root>/<name>`；再退回把该串当路径试 |
| `"D:/my/models/bge-m3"`（已存在目录） | 直接用该目录 |
| 根目录本身 | `CODECONNECT_MODEL_DIR` 覆盖，或传 `root` 参数 |

### 「未配置」与「模型缺失」在类型上分开

```rust
pub enum EmbedErrorKind {
    NotConfigured,   // 用户没开这个能力 —— 正常状态，不是故障
    ModelNotReady,   // 配了但模型没就绪（目录不存在 / 文件不全）—— 该提示去下载或改配置
    Runtime,         // 动态库缺失 / 加载失败 / 推理失败 —— 属于故障
}
impl EmbedError { pub fn kind(&self) -> EmbedErrorKind; pub fn is_not_configured(&self) -> bool; }
```

上层据此给不同文案：**「没配」就直说「没配」**，别把关闭能力说成故障。

**任何情况下都不退回 `search_by_name`**（词法检索）假装成功 —— 那正是 `semantic_search`
原来的病（名字叫语义、行为是词法），换个地方复发一样致命。

实测（`examples/similarity_check.rs` 开头会打印这一段）：

```
未配置（None）        → is_configured=false kind=NotConfigured
   未配置向量模型：语义检索处于关闭状态（用户未开启该能力，这不是故障）
按名指定（未安装的名字）  → is_configured=true  kind=ModelNotReady
   向量模型未就绪：找不到模型目录 C:\Users\Administrator\.codeconnect\models\not-installed-model
   已搜索：
     C:\Users\Administrator\.codeconnect\models\not-installed-model
     not-installed-model
按路径指定（真实目录）    → is_configured=true  kind=Ok 已加载(dim=384)
```

## 模型

- 名称：`paraphrase-multilingual-MiniLM-L12-v2`（sentence-transformers，经 Xenova 导出为 ONNX）
- 来源：`Xenova/paraphrase-multilingual-MiniLM-L12-v2`（int8 量化导出）
- 维度：384；池化：mean
- 落盘（用户级，所有项目共用一份）：
  `C:\Users\Administrator\.codeconnect\models\paraphrase-multilingual-MiniLM-L12-v2\`

### 为什么不用 bge-small-zh-v1.5

两者的判别性实测（`examples/similarity_check.rs`，查询与目标名**无任何共同子串**）：

| 用例 | bge-small-zh-v1.5（512d/cls，95 MB） | paraphrase-multilingual-MiniLM-L12-v2（384d/mean，129 MB） |
|---|---|---|
| 中文查询「计算两个时间点相差多少秒」→ `elapsed_seconds`/`duration_between` | 相关 0.385/0.403 vs 无关最高 0.348，**间隔 +0.056** | 相关 0.615/0.637 vs 无关最高 0.108，**间隔 +0.529** |
| 中文查询「把一段文本切分成词」→ 带签名符号 | 间隔 +0.030 | **间隔 +0.462** |
| 英文查询 → 英文符号 | 间隔 +0.278 | 间隔 +0.219 |
| 相关对均值 / 无关对均值 | 0.501 / 0.389 | 0.469 / **0.128** |

中文单语模型在「中文查询 → 英文标识符」这个跨语言场景上判别力太弱（间隔 0.03~0.06，
且有排序反转），而这正是本工具的主场景（符号名是英文，描述可能是中文）。
多语言模型体积多 34 MB，换来跨语言判别力提升近一个数量级。

## 模型获取

```bash
cargo run -p codeconnect-embed --features downloader --bin download-model -- [--root 根目录]
```

默认从 `hf-mirror.com` 拉取（可用 `CODECONNECT_HF_ENDPOINT` 换源），先写 `.part` 再改名，
并做字节数硬校验。落盘位置 = `<root>/<模型名>`；`--root` 缺省时用
`CODECONNECT_MODEL_DIR`，再缺省用 `~/.codeconnect/models`。
库侧入口是 `download_model_to(root, name, endpoint, force)`。

## ONNX Runtime

**构建期不下载运行时**：启用 `ort` 的 `load-dynamic`，它会打开 `ort-sys/disable-linking`，
`build.rs` 直接提前返回（实测构建日志里 ort-sys 只打了三条 `cargo:rustc-check-cfg`，
没有 `cargo:rustc-link-lib`，也没有任何下载）。

运行期按以下顺序解析动态库，第一个存在者胜出：

1. `ORT_DYLIB_PATH`
2. `CODECONNECT_ORT_DYLIB`
3. 可执行文件同目录下的 `onnxruntime.dll`
4. `PATH` 上各目录下的 `onnxruntime.dll`

全都没找到时返回 `EmbedError::RuntimeUnavailable`（含修复指引），不会让 ort 内部 panic。

**版本约束**：编译期特性 `ort/api-N` 的 N 必须 **≤** 运行期动态库的次版本号，
否则 ort 报 `BadVersion` 拒绝加载。当前锁在 `api-24`，即要求运行期 ≥ 1.24，
同时向上兼容更新版本（只打一条 info 日志）。

> 注意：`pip show onnxruntime` 可能报 1.28.0，但 `capi/onnxruntime.dll` 实际可能是被
> `onnxruntime-directml` 覆盖过的旧版本。以 `onnxruntime.InferenceSession` 能起来的那个
> DLL 实测的 `GetVersionString()` 为准。

## 池化方式

模型属性，按下列顺序解析，随模型一起落盘：

1. `<model_dir>/embed-config.json` → `{"pooling":"cls"|"mean"}`
2. `<model_dir>/1_Pooling/config.json`（sentence-transformers 约定）
3. 默认 `cls`（BGE 惯例）

## 实测

```bash
# 判别性 + 确定性
ORT_DYLIB_PATH=<...>/onnxruntime.dll cargo run -p codeconnect-embed --example similarity_check

# 模型缺失 / 运行时缺失分支
cargo test -p codeconnect-embed
```
