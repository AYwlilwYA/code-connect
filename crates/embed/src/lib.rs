//! CodeConnect 本地文本嵌入（ONNX，离线）
//!
//! 关键词：嵌入 embedding ONNX ort bge 句向量 余弦相似 语义检索 semantic 离线
//!
//! 设计要点：
//! - 推理走 `ort`，**构建期不下载运行时**：启用 `load-dynamic`，运行期从
//!   `ORT_DYLIB_PATH` / `CODECONNECT_ORT_DYLIB` / exe 同目录 / PATH 加载动态库。
//! - **向量模型是可选的**：不配置就是「能力关闭」([`EmbedError::NotConfigured`])，
//!   与「配了但模型没就绪」([`EmbedErrorKind::ModelNotReady`]) 在类型上分开。
//! - 模型名与路径都能配：见 [`ModelSource`]（未配置 / 按名 / 直接给目录）。
//! - **任何情况都不退回词法匹配**，绝不会伪装成功。
//! - 输出向量在返回前统一做 L2 归一化，余弦相似度即点积。

mod model_dir;
mod ort_runtime;

#[cfg(feature = "downloader")]
pub mod download;

pub use model_dir::{
    DEFAULT_MODEL_NAME, MODEL_REPO, REQUIRED_FILES, default_model_dir, how_to_get, model_dir,
    models_root, resolve_model_dir,
};

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use model_dir::ensure_model_files;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::{Tokenizer, TruncationParams, TruncationStrategy};

/// 单次推理的文本条数上限（控制峰值内存）
const BATCH_LIMIT: usize = 32;

/// 默认最大序列长度
const DEFAULT_MAX_LEN: usize = 512;

/// 编译期选定的 ONNX Runtime API 次版本（来自 Cargo 特性 `api-N`）。
/// 运行期动态库的次版本必须 ≥ 此值，否则 ort 拒绝加载。
pub const ORT_API_MINOR: u32 = ort::sys::ORT_API_VERSION;

/// 嵌入过程中的一切失败
///
/// 「未配置」是**一等状态**，不是失败：它表示用户没开启向量语义检索，
/// 上层应当直说「没配」，而不是回退到词法检索假装成功。
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// 未配置向量模型 —— 能力关闭，工具其余功能不受影响
    #[error("未配置向量模型：语义检索处于关闭状态（用户未开启该能力，这不是故障）")]
    NotConfigured,

    /// 已配置，但按该配置找不到模型目录
    #[error("向量模型未就绪：找不到模型目录 {dir}\n已搜索：\n{searched}\n{how_to_get}")]
    ModelDirMissing {
        /// 期望的模型目录（搜索列表第一项）
        dir: PathBuf,
        /// 已搜索过的全部路径（每行一条）
        searched: String,
        /// 获取方式
        how_to_get: String,
    },

    /// 模型目录存在但文件不全
    #[error("嵌入模型不完整：{dir} 下缺少 {missing}\n{how_to_get}")]
    ModelIncomplete {
        /// 模型目录
        dir: PathBuf,
        /// 缺失文件名（逗号分隔）
        missing: String,
        /// 获取方式
        how_to_get: String,
    },

    /// ONNX Runtime 动态库不可用
    #[error("ONNX Runtime 动态库不可用（{detail}）\n{hint}")]
    RuntimeUnavailable {
        /// 细节（找没找到、ort 报了什么）
        detail: String,
        /// 修复指引
        hint: String,
    },

    /// 载入 ONNX 模型失败
    #[error("载入 ONNX 模型失败：{0}")]
    Session(String),

    /// 载入分词器失败
    #[error("载入分词器失败：{0}")]
    Tokenizer(String),

    /// 分词失败
    #[error("分词失败：{0}")]
    Tokenize(String),

    /// 推理失败
    #[error("推理失败：{0}")]
    Inference(String),

    /// 输出张量形状异常
    #[error("模型输出形状异常：{0}")]
    BadOutput(String),
}

/// 错误大类：上层据此给出**不同文案**
///
/// 「没配」和「配坏了」必须分开说 —— 糊成一句错误，或者回退到词法检索，
/// 都是把 `semantic_search` 原来的病换个地方复发。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedErrorKind {
    /// 用户没开启向量能力 —— 正常状态，不是故障
    NotConfigured,
    /// 配了，但模型没就绪（目录不存在 / 文件不全）—— 提示用户去下载或改配置
    ModelNotReady,
    /// 运行时问题（动态库缺失 / 加载失败 / 推理失败）—— 属于故障
    Runtime,
}

impl EmbedError {
    /// 错误大类
    pub fn kind(&self) -> EmbedErrorKind {
        match self {
            EmbedError::NotConfigured => EmbedErrorKind::NotConfigured,
            EmbedError::ModelDirMissing { .. } | EmbedError::ModelIncomplete { .. } => {
                EmbedErrorKind::ModelNotReady
            }
            _ => EmbedErrorKind::Runtime,
        }
    }

    /// 是否「未配置」（能力关闭，非故障）
    pub fn is_not_configured(&self) -> bool {
        matches!(self, EmbedError::NotConfigured)
    }
}

/// 向量模型的来源（用户配置的三种形态）
///
/// 模型根目录**不是**唯一来源；用户可改根目录，也可直接给模型目录路径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSource {
    /// 未配置：用户没开启语义检索（默认状态）
    NotConfigured,
    /// 按模型名，在给定根目录下解析
    Named(String),
    /// 直接指定模型目录路径（绝对或相对）
    Dir(PathBuf),
}

impl ModelSource {
    /// 从配置串解析：
    /// - `None` 或空串 ⇒ [`ModelSource::NotConfigured`]
    /// - 指向**已存在目录**的串 ⇒ [`ModelSource::Dir`]
    /// - 其余 ⇒ [`ModelSource::Named`]（在根目录下按名解析）
    pub fn from_config(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            None | Some("") => ModelSource::NotConfigured,
            Some(v) => {
                let p = PathBuf::from(v);
                if p.is_dir() {
                    ModelSource::Dir(p)
                } else {
                    ModelSource::Named(v.to_string())
                }
            }
        }
    }

    /// 用户是否配置了向量模型
    pub fn is_configured(&self) -> bool {
        !matches!(self, ModelSource::NotConfigured)
    }

    /// 解析成实际模型目录；未配置时返回 [`EmbedError::NotConfigured`]
    pub fn resolve(&self, root: &Path) -> Result<PathBuf, EmbedError> {
        match self {
            ModelSource::NotConfigured => Err(EmbedError::NotConfigured),
            ModelSource::Named(name) => resolve_model_dir(root, name),
            ModelSource::Dir(dir) => Ok(dir.clone()),
        }
    }

    /// 配置里写的是什么（用于回显 `retrieval` 元信息）
    pub fn describe(&self) -> String {
        match self {
            ModelSource::NotConfigured => "未配置".to_string(),
            ModelSource::Named(n) => n.clone(),
            ModelSource::Dir(p) => p.display().to_string(),
        }
    }
}

/// 逐 token 输出的池化方式（模型相关：BGE 用 CLS，MiniLM / e5 用 mean）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pooling {
    /// 取第 0 个 token（`[CLS]`）
    Cls,
    /// 按 attention mask 对有效 token 求平均
    Mean,
}

impl Pooling {
    /// 从模型目录解析池化方式：
    /// 1) `<dir>/embed-config.json` 的 `{"pooling":"cls|mean"}`（最高优先级）
    /// 2) `<dir>/1_Pooling/config.json`（sentence-transformers 约定）
    /// 3) 默认 [`Pooling::Cls`]（BGE 惯例）
    pub fn resolve(model_dir: &Path) -> Self {
        if let Some(p) = read_pooling(&model_dir.join("embed-config.json"), "pooling") {
            return p;
        }
        if let Some(p) = read_st_pooling(&model_dir.join("1_Pooling").join("config.json")) {
            return p;
        }
        Pooling::Cls
    }
}

fn read_st_pooling(path: &Path) -> Option<Pooling> {
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let flag = |k: &str| value.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    if flag("pooling_mode_mean_tokens") {
        Some(Pooling::Mean)
    } else if flag("pooling_mode_cls_token") {
        Some(Pooling::Cls)
    } else {
        None
    }
}

fn read_pooling(path: &Path, key: &str) -> Option<Pooling> {
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    match value.get(key).and_then(|v| v.as_str())? {
        "mean" => Some(Pooling::Mean),
        "cls" => Some(Pooling::Cls),
        _ => None,
    }
}

/// 本地嵌入器
pub struct Embedder {
    /// ort 会话（`Session::run` 需要 `&mut`，故用 Mutex 支撑 `&self` 契约）
    session: Mutex<Session>,
    /// 分词器
    tokenizer: Tokenizer,
    /// 是否需要喂 `token_type_ids`
    needs_token_type_ids: bool,
    /// 实际使用的输出名
    output_name: String,
    /// 逐 token 输出的池化方式
    pooling: Pooling,
    /// 向量维度
    dim: usize,
    /// 模型目录
    model_dir: PathBuf,
}

impl Embedder {
    /// 从模型目录加载。目录缺失 / 文件不全都返回明确错误。
    ///
    /// `dir` 可以是任意路径 —— 调用方自己决定从哪来（配置的绝对路径、按名解析的结果……）。
    pub fn load(dir: &Path) -> Result<Self, EmbedError> {
        ensure_model_files(dir, &models_root())?;
        // 先自行定位动态库：否则 ort 内部 `setup_api` 会直接 panic，拿不到可读错误
        let ort_path = ort_runtime::prepare_ort_dylib().map_err(|detail| {
            EmbedError::RuntimeUnavailable {
                detail,
                hint: ort_runtime::runtime_hint(),
            }
        })?;

        let model_path = dir.join("model.onnx");
        let builder = Session::builder().map_err(|e| {
            EmbedError::Session(format!(
                "初始化 ONNX Runtime 失败（{}）：{e}",
                ort_path.display()
            ))
        })?;
        let session = builder
            .with_intra_threads(default_threads())
            .map_err(|e| EmbedError::Session(e.to_string()))?
            .commit_from_file(&model_path)
            .map_err(|e| EmbedError::Session(format!("{}：{e}", model_path.display())))?;

        let tokenizer_path = dir.join("tokenizer.json");
        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| EmbedError::Tokenizer(format!("{}：{e}", tokenizer_path.display())))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: DEFAULT_MAX_LEN,
                strategy: TruncationStrategy::LongestFirst,
                stride: 0,
                ..Default::default()
            }))
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;

        let needs_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        let output_name = pick_output(&session)?;
        let pooling = Pooling::resolve(dir);

        let mut embedder = Self {
            session: Mutex::new(session),
            tokenizer,
            needs_token_type_ids,
            output_name,
            pooling,
            dim: 0,
            model_dir: dir.to_path_buf(),
        };
        // 用一次真实推理定维度：保证 dim() 报告的就是实际输出维度
        let probe = embedder.embed(&["dimension probe".to_string()])?;
        embedder.dim = probe[0].len();
        tracing::info!(
            model_dir = %dir.display(),
            output = %embedder.output_name,
            pooling = ?embedder.pooling,
            dim = embedder.dim,
            ort_dylib = %ort_path.display(),
            "嵌入模型已加载"
        );
        Ok(embedder)
    }

    /// 批量嵌入；返回与输入**等长**的向量列表，每行已 L2 归一化。
    pub fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(BATCH_LIMIT) {
            out.extend(self.embed_chunk(chunk)?);
        }
        Ok(out)
    }

    /// 向量维度
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// 模型名（模型目录名）
    pub fn model_name(&self) -> &str {
        self.model_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(DEFAULT_MODEL_NAME)
    }

    /// 模型目录
    pub fn model_dir(&self) -> &Path {
        &self.model_dir
    }

    fn embed_chunk(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let mut encodings = Vec::with_capacity(texts.len());
        for t in texts {
            let enc = self
                .tokenizer
                .encode(t.as_str(), true)
                .map_err(|e| EmbedError::Tokenize(e.to_string()))?;
            encodings.push(enc);
        }
        let batch = encodings.len();
        let seq = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0)
            .max(1);

        let mut ids = vec![0i64; batch * seq];
        let mut mask = vec![0i64; batch * seq];
        let mut types = vec![0i64; batch * seq];
        for (i, enc) in encodings.iter().enumerate() {
            let base = i * seq;
            let n = enc.get_ids().len();
            for j in 0..n {
                ids[base + j] = enc.get_ids()[j] as i64;
                mask[base + j] = enc.get_attention_mask()[j] as i64;
                if self.needs_token_type_ids {
                    types[base + j] = enc.get_type_ids()[j] as i64;
                }
            }
        }

        let shape = vec![batch, seq];
        let mut session = self
            .session
            .lock()
            .map_err(|e| EmbedError::Inference(format!("会话锁已中毒：{e}")))?;

        let ids_t = Tensor::from_array((shape.clone(), ids))
            .map_err(|e| EmbedError::Inference(format!("构造 input_ids 失败：{e}")))?;
        // mask 之后还要用于 mean pooling，故喂给张量的是副本
        let mask_t = Tensor::from_array((shape.clone(), mask.clone()))
            .map_err(|e| EmbedError::Inference(format!("构造 attention_mask 失败：{e}")))?;
        let outputs = if self.needs_token_type_ids {
            let types_t = Tensor::from_array((shape, types))
                .map_err(|e| EmbedError::Inference(format!("构造 token_type_ids 失败：{e}")))?;
            session.run(ort::inputs![
                "input_ids" => ids_t,
                "attention_mask" => mask_t,
                "token_type_ids" => types_t,
            ])
        } else {
            session.run(ort::inputs![
                "input_ids" => ids_t,
                "attention_mask" => mask_t,
            ])
        }
        .map_err(|e| EmbedError::Inference(e.to_string()))?;

        let value = outputs
            .get(self.output_name.as_str())
            .ok_or_else(|| EmbedError::BadOutput(format!("模型没有输出 `{}`", self.output_name)))?;
        let (shape, data) = value
            .try_extract_tensor::<f32>()
            .map_err(|e| {
                EmbedError::BadOutput(format!("输出 `{}` 不是 f32 张量：{e}", self.output_name))
            })?;

        // 2 维：[batch, dim] 已池化；3 维：[batch, seq, dim] 取第 0 个 token（BGE 惯例）
        let rank = shape.len();
        let dim = match rank {
            2 => shape[1] as usize,
            3 => shape[2] as usize,
            _ => {
                return Err(EmbedError::BadOutput(format!(
                    "输出 `{}` 的秩为 {rank}，期望 2（已池化）或 3（逐 token）",
                    self.output_name
                )));
            }
        };
        let seq = if rank == 3 { shape[1] as usize } else { 1 };
        let mut result = Vec::with_capacity(batch);
        for i in 0..batch {
            let start = i * seq * dim;
            let block = data
                .get(start..start + seq * dim)
                .ok_or_else(|| EmbedError::BadOutput(format!("输出 `{}` 数据长度不足", self.output_name)))?;
            let row = if rank == 2 {
                block.to_vec()
            } else {
                match self.pooling {
                    // 秩 3 时 block 的第 0 行就是 `[CLS]`
                    Pooling::Cls => block[..dim].to_vec(),
                    Pooling::Mean => mean_pool(block, &mask[i * seq..(i + 1) * seq], dim),
                }
            };
            result.push(l2_normalize(row));
        }
        Ok(result)
    }
}

/// 加载模型；模型缺失时返回可读的错误（含期望路径与获取方式）
///
/// `model_dir` 为任意模型目录路径，契约不限定来源。
pub fn load(model_dir: &Path) -> Result<Embedder, EmbedError> {
    Embedder::load(model_dir)
}

/// 按「模型来源 + 模型根目录」加载
///
/// - [`ModelSource::NotConfigured`] ⇒ [`EmbedError::NotConfigured`]（能力关闭，不是故障）
/// - [`ModelSource::Named`] ⇒ 在 `root` 下按名解析，找不到给含**搜索路径**的错误
/// - [`ModelSource::Dir`] ⇒ 直接用该目录
///
/// 这是配置驱动的入口：上层把用户配置解析成 [`ModelSource`] 后调这里。
pub fn load_source(source: &ModelSource, root: &Path) -> Result<Embedder, EmbedError> {
    Embedder::load(&source.resolve(root)?)
}

/// 用默认根目录 + 默认模型名加载
pub fn load_default() -> Result<Embedder, EmbedError> {
    Embedder::load(&default_model_dir())
}

/// 余弦相似度。向量已 L2 归一化时等价于点积。
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len().min(b.len()) {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// 按 attention mask 对有效 token 求平均（padding 位不参与）
fn mean_pool(block: &[f32], mask: &[i64], dim: usize) -> Vec<f32> {
    let mut acc = vec![0.0f32; dim];
    let mut count = 0.0f32;
    for (t, m) in mask.iter().enumerate() {
        if *m == 0 {
            continue;
        }
        let base = t * dim;
        if let Some(row) = block.get(base..base + dim) {
            for (a, v) in acc.iter_mut().zip(row) {
                *a += *v;
            }
            count += 1.0;
        }
    }
    if count > 0.0 {
        for a in acc.iter_mut() {
            *a /= count;
        }
    }
    acc
}

fn l2_normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
    v
}

/// 从会话元数据里挑输出名：优先已池化的 `sentence_embedding`，其次 `last_hidden_state`
fn pick_output(session: &Session) -> Result<String, EmbedError> {
    let names: Vec<&str> = session.outputs().iter().map(|o| o.name()).collect();
    names
        .iter()
        .find(|n| **n == "sentence_embedding")
        .or_else(|| names.iter().find(|n| **n == "last_hidden_state"))
        .or_else(|| names.first())
        .map(|n| (*n).to_string())
        .ok_or_else(|| EmbedError::BadOutput("模型没有任何输出".to_string()))
}

fn default_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().min(4))
        .unwrap_or(1)
}
