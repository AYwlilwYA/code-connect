//! 向量语义检索：符号语料 → 嵌入 → 余弦 top-K
//!
//! 关键词：semantic_search 语义检索 向量 embedding 余弦 相似 top-K 语料 指纹 缓存 embeddings.bin
//!
//! 设计要点（对应 `doc/spec/2026-09-25-text-truth-and-vector-search.md` §3）：
//! - 只嵌「符号名 + 签名 + doc 注释首行」，**不嵌函数体**（体积与收益不成比例）；
//! - 语料取自 tantivy 全量符号扫描；向量落 `<data_dir>/embeddings.bin`，
//!   语料指纹或模型不符即重建；
//! - 暴力扫描（符号量级几千），暂不引入 ANN 索引；
//! - 模型不可用时**只返回错误**，由调用方如实说明 —— 本模块不做任何词法回退。

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use codeconnect_core::config::SemanticConfig;
use codeconnect_embed::{Embedder, EmbedError, ModelSource, how_to_get, models_root};
use codeconnect_index::tantivy_index::SymbolSearchResult;

/// 向量缓存文件名（落在索引数据目录下）
pub const EMBEDDINGS_FILE: &str = "embeddings.bin";

/// 文件魔数
const MAGIC: &[u8; 8] = b"CCEMB01\n";

/// 文件格式版本
const FORMAT_VERSION: u32 = 1;

/// 单条符号参与嵌入的文本上限（字符数；分词器另有 512 token 截断）
const MAX_EMBED_CHARS: usize = 400;

/// 顶分低于此值时提示「很可能没有真正相关的符号」
///
/// 阈值取自本仓库实测（1422 个符号、model = paraphrase-multilingual-MiniLM-L12-v2）：
/// 明确相关的查询顶分在 0.58~0.71，没有对应符号的查询顶分在 0.36~0.45。
/// 这是启发式：余弦值只是线索，响应里始终回传 `top_similarity` 供调用方自行判断，
/// 不靠一个阈值替它下结论。
const LOW_CONFIDENCE_SIMILARITY: f32 = 0.35;

/// 参与嵌入的一条符号（活语料，每行都来自本次扫描，行号等信息不会过期）
#[derive(Debug, Clone)]
pub struct CorpusEntry {
    /// 稳定符号 ID
    pub stable_id: String,
    /// 符号名
    pub name: String,
    /// 符号类型
    pub kind: String,
    /// 语言
    pub language: String,
    /// 文件路径
    pub file_path: String,
    /// 起始行
    pub line: u64,
    /// 签名
    pub signature: String,
    /// 文档注释
    pub doc_comment: String,
}

impl CorpusEntry {
    /// 从索引扫描结果构造
    pub fn from_search_result(r: &SymbolSearchResult) -> Self {
        Self {
            stable_id: r.stable_id.clone(),
            name: r.name.clone(),
            kind: r.kind.clone(),
            language: r.language.clone(),
            file_path: r.file_path.clone(),
            line: r.line,
            signature: r.signature.clone(),
            doc_comment: r.doc_comment.clone(),
        }
    }

    /// 参与嵌入的文本：符号名 + 签名 + doc 注释首行（**不含函数体**）
    pub fn embed_text(&self) -> String {
        let mut s = String::with_capacity(self.name.len() + self.signature.len() + 32);
        s.push_str(&self.name);
        if !self.signature.is_empty() {
            s.push(' ');
            s.push_str(&self.signature);
        }
        if let Some(doc) = self.first_doc_line() {
            s.push(' ');
            s.push_str(&doc);
        }
        truncate_chars(s, MAX_EMBED_CHARS)
    }

    /// doc 注释首行（剥掉注释符号后第一个非空行）
    pub fn first_doc_line(&self) -> Option<String> {
        self.doc_comment.lines().find_map(|raw| {
            let line = raw.trim().trim_start_matches(['/', '*', '!', ' ']).trim();
            (!line.is_empty()).then(|| line.to_string())
        })
    }
}

/// 一次向量检索的命中
#[derive(Debug, Clone, Copy)]
pub struct VectorHit {
    /// 在语料（`CorpusEntry` 切片）中的下标
    pub corpus_index: usize,
    /// 余弦相似度
    pub similarity: f32,
}

/// 语料向量（不含逐条元信息 —— 元信息一律取本次扫描的活语料，避免行号过期）
struct SemanticIndex {
    /// 与语料**顺序一一对应**的向量
    vectors: Vec<Vec<f32>>,
    /// 模型目录（模型换了向量即作废）
    model_key: String,
    /// 语料指纹（stable_id + 名称 + 签名 + doc，顺序敏感）
    fingerprint: u64,
    /// 向量维度
    dim: usize,
}

/// 向量检索的输入
pub struct VectorRequest<'a> {
    /// 模型来源（未配置 ⇒ 直接返回 [`EmbedError::NotConfigured`]）
    pub source: &'a ModelSource,
    /// 模型根目录
    pub root: &'a Path,
    /// 索引数据目录（落盘缓存位置；`None` 则只用进程内缓存）
    pub data_dir: Option<&'a Path>,
    /// 本次扫描到的活语料
    pub entries: &'a [CorpusEntry],
    /// 查询串
    pub query: &'a str,
    /// 返回条数上限
    pub limit: usize,
    /// 语言过滤
    pub language: Option<&'a str>,
}

/// 向量检索的结果与元信息（供响应如实标注检索方式）
pub struct VectorOutcome {
    /// 命中（按相似度降序）
    pub hits: Vec<VectorHit>,
    /// 模型名（模型目录名）
    pub model: String,
    /// 模型目录
    pub model_dir: PathBuf,
    /// 向量维度
    pub dim: usize,
    /// 语料规模（符号数）
    pub corpus_size: usize,
    /// 本次是否重新嵌入了语料（`false` = 命中缓存）
    pub rebuilt: bool,
    /// 本次嵌入耗时（毫秒，仅重建时有意义）
    pub embed_ms: u64,
    /// 顶名相似度低于置信阈值 —— 很可能没有真正相关的符号
    pub low_confidence: bool,
}

/// 进程内缓存：模型只加载一次，语料向量按指纹复用
#[derive(Default)]
pub struct SemanticCache {
    /// 已加载的嵌入器 + 它对应的模型目录
    embedder: Option<(String, Arc<Embedder>)>,
    /// 已就绪的语料向量
    index: Option<Arc<SemanticIndex>>,
}

impl SemanticCache {
    /// 向量检索
    ///
    /// 三态由 [`EmbedError`] 表达并**原样向上抛**：未配置 / 模型未就绪 / 运行时故障。
    pub fn search(&mut self, req: VectorRequest<'_>) -> Result<VectorOutcome, EmbedError> {
        let dir = req.source.resolve(req.root)?;
        let embedder = self.embedder(&dir)?;

        let fingerprint = fingerprint(req.entries);
        let model_key = dir.display().to_string();
        let dim = embedder.dim();

        let mut rebuilt = false;
        let mut embed_ms = 0;
        let index = match self.index.clone() {
            Some(idx) if idx.is_valid(&model_key, fingerprint, dim, req.entries.len()) => idx,
            _ => {
                // 落盘缓存 → 全量重新嵌入
                let from_disk = req
                    .data_dir
                    .map(|d| d.join(EMBEDDINGS_FILE))
                    .filter(|p| p.is_file())
                    .and_then(|p| match SemanticIndex::load(&p, &model_key, fingerprint, dim) {
                        Ok(idx) => Some(idx),
                        Err(reason) => {
                            tracing::warn!(path = %p.display(), reason, "向量缓存不可用，将重建");
                            None
                        }
                    });

                let idx = match from_disk.filter(|i| i.is_valid(&model_key, fingerprint, dim, req.entries.len())) {
                    Some(idx) => idx,
                    None => {
                        let started = Instant::now();
                        let idx = SemanticIndex::build(embedder.as_ref(), req.entries, &model_key, fingerprint)?;
                        embed_ms = started.elapsed().as_millis() as u64;
                        rebuilt = true;
                        tracing::info!(
                            symbols = idx.vectors.len(),
                            dim,
                            took_ms = embed_ms,
                            "符号向量已重建"
                        );
                        idx
                    }
                };

                if rebuilt
                    && let Some(path) = req.data_dir.map(|d| d.join(EMBEDDINGS_FILE))
                    && let Err(e) = idx.save(&path)
                {
                    tracing::warn!(path = %path.display(), error = %e, "向量缓存写入失败（不影响本次结果）");
                }

                let arc = Arc::new(idx);
                self.index = Some(arc.clone());
                arc
            }
        };

        let hits = index.search(
            embedder.as_ref(),
            req.entries,
            req.query,
            req.limit,
            req.language,
        )?;
        let low_confidence = hits
            .first()
            .is_some_and(|h| h.similarity < LOW_CONFIDENCE_SIMILARITY);

        Ok(VectorOutcome {
            hits,
            model: embedder.model_name().to_string(),
            model_dir: dir,
            dim,
            corpus_size: req.entries.len(),
            rebuilt,
            embed_ms,
            low_confidence,
        })
    }

    /// 取（或加载）指定模型目录的嵌入器；模型没变则复用
    fn embedder(&mut self, dir: &Path) -> Result<Arc<Embedder>, EmbedError> {
        let key = dir.display().to_string();
        if let Some((cached_key, e)) = &self.embedder
            && *cached_key == key
        {
            return Ok(e.clone());
        }
        let loaded = Arc::new(codeconnect_embed::load(dir)?);
        self.embedder = Some((key, loaded.clone()));
        Ok(loaded)
    }
}

impl SemanticIndex {
    /// 全量嵌入语料
    fn build(
        embedder: &Embedder,
        entries: &[CorpusEntry],
        model_key: &str,
        fingerprint: u64,
    ) -> Result<Self, EmbedError> {
        let texts: Vec<String> = entries.iter().map(CorpusEntry::embed_text).collect();
        let vectors = embedder.embed(&texts)?;
        Ok(Self {
            vectors,
            model_key: model_key.to_string(),
            fingerprint,
            dim: embedder.dim(),
        })
    }

    /// 向量是否仍适用于当前模型与语料
    ///
    /// `entries_len` 必须一并给：向量是按语料下标返回命中的，
    /// 条数对不上就会出现越界下标（语料条数在指纹里不显式携带）。
    fn is_valid(&self, model_key: &str, fingerprint: u64, dim: usize, entries_len: usize) -> bool {
        self.model_key == model_key
            && self.fingerprint == fingerprint
            && self.dim == dim
            && self.vectors.len() == entries_len
    }

    /// 查询串嵌入 → 余弦相似 → top-K
    ///
    /// 语言过滤**先于**排序：先取 top-K 再过滤会出现「库里有却只回几条」的假象。
    fn search(
        &self,
        embedder: &Embedder,
        entries: &[CorpusEntry],
        query: &str,
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<VectorHit>, EmbedError> {
        let mut embedded = embedder.embed(&[query.to_string()])?;
        let query_vec = match embedded.pop() {
            Some(v) => v,
            None => return Ok(Vec::new()),
        };

        let mut scored: Vec<(usize, f32)> = self
            .vectors
            .iter()
            .enumerate()
            .filter(|(i, _)| match language {
                Some(lang) => entries.get(*i).is_some_and(|e| e.language == lang),
                None => true,
            })
            .map(|(i, v)| (i, codeconnect_embed::cosine_similarity(&query_vec, v)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit.max(1));

        Ok(scored
            .into_iter()
            .map(|(corpus_index, similarity)| VectorHit {
                corpus_index,
                similarity,
            })
            .collect())
    }

    /// 落盘（`<data_dir>/embeddings.bin`，f32 明文 + 小端）
    fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(self.vectors.len() * self.dim * 4);
        for v in &self.vectors {
            for f in v {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
        }

        let mut w = BufWriter::new(File::create(path)?);
        w.write_all(MAGIC)?;
        w.write_all(&FORMAT_VERSION.to_le_bytes())?;
        w.write_all(&(self.dim as u32).to_le_bytes())?;
        w.write_all(&(self.vectors.len() as u32).to_le_bytes())?;
        w.write_all(&self.fingerprint.to_le_bytes())?;
        w.write_all(&str_hash(&self.model_key).to_le_bytes())?;
        w.write_all(&(bytes.len() as u64).to_le_bytes())?;
        w.write_all(&bytes)?;
        w.flush()
    }

    /// 读盘；与当前模型/语料不符时返回原因（调用方据此重建，不当作故障）
    fn load(path: &Path, model_key: &str, fingerprint: u64, dim: usize) -> Result<Self, String> {
        let mut r = BufReader::new(File::open(path).map_err(|e| e.to_string())?);

        let mut magic = [0u8; 8];
        r.read_exact(&mut magic).map_err(|e| e.to_string())?;
        if &magic != MAGIC {
            return Err("魔数不符（不是 CodeConnect 向量缓存）".into());
        }
        let version = read_u32(&mut r)?;
        if version != FORMAT_VERSION {
            return Err(format!("格式版本 {version} != {FORMAT_VERSION}"));
        }
        let file_dim = read_u32(&mut r)? as usize;
        let count = read_u32(&mut r)? as usize;
        let file_fingerprint = read_u64(&mut r)?;
        let file_model = read_u64(&mut r)?;
        let bytes_len = read_u64(&mut r)? as usize;

        if file_fingerprint != fingerprint {
            return Err("语料已变化".into());
        }
        if file_model != str_hash(model_key) {
            return Err("模型已变化".into());
        }
        if file_dim != dim {
            return Err(format!("维度 {file_dim} != 当前模型 {dim}"));
        }
        if count == 0 || bytes_len != count * dim * 4 {
            return Err("向量数据长度与条数不符".into());
        }

        let mut bytes = vec![0u8; bytes_len];
        r.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        let mut vectors = Vec::with_capacity(count);
        for chunk in bytes.chunks_exact(dim * 4) {
            vectors.push(
                chunk
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect(),
            );
        }

        Ok(Self {
            vectors,
            model_key: model_key.to_string(),
            fingerprint,
            dim,
        })
    }
}

// ============================================================================
// 配置 → 模型来源
// ============================================================================

/// 模型根目录：配置里的 `model_dir` 优先，否则用默认根目录（`CODECONNECT_MODEL_DIR` 或 `~/.codeconnect/models`）
pub fn model_root(cfg: &SemanticConfig) -> PathBuf {
    cfg.model_dir.clone().unwrap_or_else(models_root)
}

/// 配置 → 模型来源；未启用时一律 [`ModelSource::NotConfigured`]
pub fn model_source(cfg: &SemanticConfig) -> ModelSource {
    if cfg.is_enabled() {
        ModelSource::from_config(cfg.model_value())
    } else {
        ModelSource::NotConfigured
    }
}

/// 「未配置」状态的说明文案：**直说没配** + 配置方法 + 模型获取方式
///
/// 这一段是 `semantic_search` 三态里的第一态，也是最容易被做成静默降级的一态。
pub fn not_configured_message(cfg: &SemanticConfig, root: &Path) -> String {
    let mut msg = String::from(
        "未配置向量模型，语义检索不可用（这是能力未开启，不是故障；本次没有退回词法匹配，也没有返回任何猜测结果）。",
    );

    if cfg.enabled == Some(true) && cfg.model_value().is_none() {
        msg.push_str("\n注意：配置里写了 [semantic] enabled = true，但没有指定 model —— 仍视为未配置。");
    }
    if cfg.disabled_but_model_set() {
        msg.push_str("\n注意：配置里配了 model 但 enabled = false，模型被忽略。");
    }

    let expected = root.join(codeconnect_embed::DEFAULT_MODEL_NAME);
    msg.push_str(&format!(
        "\n配置方法（任选一处写入）：项目根目录的 .codeconnect.toml，或用户级 ~/.codeconnect/config.toml\n\
         \x20 [semantic]\n\
         \x20 enabled = true\n\
         \x20 model = \"{default_name}\"   # 模型名（在模型根目录下查找，也可直接写模型目录绝对路径）\n\
         \x20 # model_dir = \"D:/models\"  # 可选：覆盖模型根目录\n\
         当前模型根目录：{root}\n\
         模型默认应位于：{expected}\n\
         {get}",
        default_name = codeconnect_embed::DEFAULT_MODEL_NAME,
        root = root.display(),
        expected = expected.display(),
        get = how_to_get(&expected, root),
    ));
    msg
}

/// 「已配置但模型没就绪」的说明文案（含期望路径 + 已搜索路径 + 获取方式，均来自 [`EmbedError`]）
pub fn model_not_ready_message(cfg: &SemanticConfig, root: &Path, err: &EmbedError) -> String {
    format!(
        "已配置向量模型（model = {}，根目录 {}），但模型未就绪：\n{err}",
        match cfg.model_value() {
            Some(m) => format!("\"{m}\""),
            None => "（未指定，用的默认模型名）".to_string(),
        },
        root.display(),
    )
}

/// 把「按名解析」的搜索结果也回显出来，供调用方自行判断该去哪放模型
pub fn searched_paths(cfg: &SemanticConfig, root: &Path) -> Vec<PathBuf> {
    match model_source(cfg) {
        ModelSource::Named(name) => {
            let first = root.join(&name);
            let direct = PathBuf::from(&name);
            if direct == first {
                vec![first]
            } else {
                vec![first, direct]
            }
        }
        ModelSource::Dir(dir) => vec![dir],
        ModelSource::NotConfigured => Vec::new(),
    }
}

// ============================================================================
// 工具
// ============================================================================

/// 语料指纹：stable_id + 名称 + 签名 + doc，顺序敏感
///
/// 只覆盖**参与嵌入的文本**，不含行号/文件路径 —— 符号挪了几行不该触发全量重嵌；
/// 行号等信息一律取本次扫描的活语料，因此不会过期。
fn fingerprint(entries: &[CorpusEntry]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for e in entries {
        e.stable_id.hash(&mut hasher);
        e.name.hash(&mut hasher);
        e.signature.hash(&mut hasher);
        e.doc_comment.hash(&mut hasher);
        0xffu8.hash(&mut hasher);
    }
    hasher.finish()
}

/// 字符串哈希（模型目录的指纹用，只用于「模型换没换」的判断）
fn str_hash(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

fn truncate_chars(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    s.chars().take(max).collect()
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32, String> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64, String> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(u64::from_le_bytes(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, signature: &str, doc: &str) -> CorpusEntry {
        CorpusEntry {
            stable_id: format!("rust::{name}"),
            name: name.to_string(),
            kind: "function".to_string(),
            language: "rust".to_string(),
            file_path: "src/lib.rs".to_string(),
            line: 1,
            signature: signature.to_string(),
            doc_comment: doc.to_string(),
        }
    }

    #[test]
    fn test_embed_text_is_name_signature_doc_first_line() {
        let e = entry("elapsed_seconds", "pub fn elapsed_seconds(a: u64, b: u64) -> u64", "/// 计算时间差\n/// 第二行");
        let text = e.embed_text();
        assert!(text.starts_with("elapsed_seconds pub fn elapsed_seconds"));
        assert!(text.contains("计算时间差"));
        // 只取首行，第二行不参与
        assert!(!text.contains("第二行"));
    }

    #[test]
    fn test_embed_text_skips_blank_doc_lines() {
        let e = entry("f", "", "///\n///\n/// 真正的说明");
        assert_eq!(e.first_doc_line().as_deref(), Some("真正的说明"));
        assert_eq!(e.embed_text(), "f 真正的说明");
    }

    #[test]
    fn test_fingerprint_ignores_line_but_tracks_signature() {
        let a = vec![entry("f", "fn f()", "")];
        let mut b = a.clone();
        b[0].line = 999;
        assert_eq!(fingerprint(&a), fingerprint(&b), "行号变化不该触发重嵌");

        let mut c = a.clone();
        c[0].signature = "fn f(x: u32)".into();
        assert_ne!(fingerprint(&a), fingerprint(&c), "签名变化必须触发重嵌");
    }

    #[test]
    fn test_not_configured_message_mentions_state_and_how_to() {
        let cfg = SemanticConfig::default();
        let msg = not_configured_message(&cfg, Path::new("C:/models"));
        assert!(msg.contains("未配置向量模型"));
        assert!(msg.contains("没有退回词法匹配"));
        assert!(msg.contains("[semantic]"));
        assert!(msg.contains("enabled = true"));
        assert!(msg.contains("C:/models"));
    }

    #[test]
    fn test_not_configured_message_flags_enabled_without_model() {
        let cfg = SemanticConfig {
            enabled: Some(true),
            model: None,
            model_dir: None,
        };
        let msg = not_configured_message(&cfg, Path::new("C:/models"));
        assert!(msg.contains("没有指定 model"), "应指出开了开关却没给模型：{msg}");
    }

    #[test]
    fn test_disabled_but_model_set_is_flagged() {
        let cfg = SemanticConfig {
            enabled: Some(false),
            model: Some("m".into()),
            model_dir: None,
        };
        assert!(!model_source(&cfg).is_configured());
        let msg = not_configured_message(&cfg, Path::new("C:/models"));
        assert!(msg.contains("模型被忽略"));
    }

    fn index_with(vectors: Vec<Vec<f32>>, model_key: &str, fingerprint: u64) -> SemanticIndex {
        let dim = vectors.first().map(Vec::len).unwrap_or(0);
        SemanticIndex {
            vectors,
            model_key: model_key.to_string(),
            fingerprint,
            dim,
        }
    }

    #[test]
    fn test_vector_cache_roundtrip() {
        let dir = std::env::temp_dir().join("cc_semantic_cache_roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(EMBEDDINGS_FILE);

        let idx = index_with(
            vec![vec![0.1, 0.2, 0.3], vec![-0.5, 0.0, 0.5]],
            "C:/models/m1",
            0xdead_beef,
        );
        idx.save(&path).expect("落盘失败");

        let loaded = SemanticIndex::load(&path, "C:/models/m1", 0xdead_beef, 3).expect("读盘失败");
        assert_eq!(loaded.vectors.len(), 2);
        assert_eq!(loaded.vectors[1], vec![-0.5, 0.0, 0.5]);
        assert!(loaded.is_valid("C:/models/m1", 0xdead_beef, 3, 2));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_vector_cache_rejects_stale_or_mismatched() {
        let dir = std::env::temp_dir().join("cc_semantic_cache_stale");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(EMBEDDINGS_FILE);

        let idx = index_with(vec![vec![1.0, 0.0]], "m1", 42);
        idx.save(&path).expect("落盘失败");

        // 语料变了 / 模型换了 / 维度对不上 / 条数对不上 —— 一律判为不可用（调用方据此重建）
        assert!(SemanticIndex::load(&path, "m1", 43, 2).is_err(), "语料指纹变了必须拒绝");
        assert!(SemanticIndex::load(&path, "m2", 42, 2).is_err(), "模型变了必须拒绝");
        assert!(SemanticIndex::load(&path, "m1", 42, 3).is_err(), "维度不符必须拒绝");

        let loaded = SemanticIndex::load(&path, "m1", 42, 2).unwrap();
        assert!(!loaded.is_valid("m1", 42, 2, 2), "语料条数对不上必须判为不可用");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_vector_cache_rejects_foreign_file() {
        let dir = std::env::temp_dir().join("cc_semantic_cache_foreign");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(EMBEDDINGS_FILE);
        std::fs::write(&path, b"not-a-vector-cache-file").unwrap();

        assert!(SemanticIndex::load(&path, "m1", 42, 2).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_truncate_chars_counts_chars_not_bytes() {
        let long = "中".repeat(MAX_EMBED_CHARS + 10);
        assert_eq!(truncate_chars(long, MAX_EMBED_CHARS).chars().count(), MAX_EMBED_CHARS);
    }

    /// 模型根目录：配置 > 环境变量 > 内置默认
    ///
    /// 本用例是**唯一**读写 `CODECONNECT_MODEL_DIR` 的测试（环境变量是进程级的，
    /// 多个用例各写各的会互相打架），因此它同时也负责断言默认值。
    #[test]
    fn test_model_root_precedence_config_env_default() {
        let saved = std::env::var_os("CODECONNECT_MODEL_DIR");

        // 1) 环境变量覆盖内置默认（默认是 ~/.codeconnect/models）
        unsafe { std::env::set_var("CODECONNECT_MODEL_DIR", "F:/cc-env-root") };
        assert_eq!(
            model_root(&SemanticConfig::default()),
            PathBuf::from("F:/cc-env-root"),
            "未配置 model_dir 时必须让 CODECONNECT_MODEL_DIR 生效"
        );

        // 2) 配置里的 model_dir 比环境变量更具体 —— 同时存在时以配置为准
        let cfg = SemanticConfig {
            enabled: None,
            model: None,
            model_dir: Some(PathBuf::from("D:/my-models")),
        };
        assert_eq!(model_root(&cfg), PathBuf::from("D:/my-models"));

        // 3) 两者都没有时回到内置默认
        unsafe { std::env::remove_var("CODECONNECT_MODEL_DIR") };
        assert!(
            model_root(&SemanticConfig::default()).ends_with("models"),
            "无配置无环境变量时应回落到默认模型根目录"
        );

        if let Some(v) = saved {
            unsafe { std::env::set_var("CODECONNECT_MODEL_DIR", v) };
        }
    }
}
