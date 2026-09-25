//! 模型目录定位与必需文件清单
//!
//! 模型根目录**不是**唯一来源，只是默认值：用户可改根目录，也可直接给模型目录路径。
//!
//! 关键词：模型目录 models 用户级 .codeconnect 落盘路径 根目录 按名解析

use std::path::{Path, PathBuf};

use crate::EmbedError;

/// 默认模型名（模型根目录下的子目录名）
///
/// 选型理由见 `crates/embed/README.md`：实测「中文查询 → 英文符号名」的判别间隔上，
/// 多语言模型（+0.53）远优于中文单语模型 bge-small-zh-v1.5（+0.06）。
pub const DEFAULT_MODEL_NAME: &str = "paraphrase-multilingual-MiniLM-L12-v2";

/// 模型来源仓库（含 ONNX 量化导出与 tokenizer.json）
pub const MODEL_REPO: &str = "Xenova/paraphrase-multilingual-MiniLM-L12-v2";

/// 模型目录内必需的文件
pub const REQUIRED_FILES: &[&str] = &["model.onnx", "tokenizer.json"];

/// **默认**模型根目录：`~/.codeconnect/models`
///
/// 这只是默认值。用户可通过 `CODECONNECT_MODEL_DIR` 覆盖，或在配置里显式给根目录 /
/// 模型目录路径（见 [`crate::ModelSource`]）。
pub fn models_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("CODECONNECT_MODEL_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codeconnect")
        .join("models")
}

/// 默认根目录下、按模型名得到的目录
pub fn model_dir(name: &str) -> PathBuf {
    models_root().join(name)
}

/// 默认模型（默认根目录 + 默认模型名）的目录
pub fn default_model_dir() -> PathBuf {
    model_dir(DEFAULT_MODEL_NAME)
}

/// 在给定根目录下按模型名解析出模型目录；找不到时返回明确错误（**含搜索过的路径**）。
///
/// 解析顺序：
/// 1. `<root>/<name>`
/// 2. `<name>` 原样（相对 / 绝对路径，允许配置里直接写路径）
pub fn resolve_model_dir(root: &Path, name: &str) -> Result<PathBuf, EmbedError> {
    let mut candidates = vec![root.join(name)];
    let direct = PathBuf::from(name);
    if direct != candidates[0] {
        candidates.push(direct);
    }

    let mut searched = Vec::new();
    for c in candidates {
        if c.is_dir() {
            return Ok(c);
        }
        searched.push(c);
    }
    Err(EmbedError::ModelDirMissing {
        dir: searched[0].clone(),
        searched: render_searched(&searched),
        how_to_get: how_to_get(&searched[0], root),
    })
}

/// 校验模型目录必需文件是否齐全
pub fn ensure_model_files(model_dir: &Path, root: &Path) -> Result<(), EmbedError> {
    if !model_dir.is_dir() {
        return Err(EmbedError::ModelDirMissing {
            dir: model_dir.to_path_buf(),
            searched: render_searched(std::slice::from_ref(&model_dir.to_path_buf())),
            how_to_get: how_to_get(model_dir, root),
        });
    }
    let missing: Vec<&str> = REQUIRED_FILES
        .iter()
        .copied()
        .filter(|f| !model_dir.join(f).is_file())
        .collect();
    if !missing.is_empty() {
        return Err(EmbedError::ModelIncomplete {
            dir: model_dir.to_path_buf(),
            missing: missing.join(", "),
            how_to_get: how_to_get(model_dir, root),
        });
    }
    Ok(())
}

/// 模型获取方式（拼进错误消息，保证「模型没就绪」是可自救的明确错误）
pub fn how_to_get(model_dir: &Path, root: &Path) -> String {
    format!(
        "获取方式（任选其一）：\n\
         1) 运行内置下载器（约 130 MB）：\n\
         \x20  cargo run -p codeconnect-embed --features downloader --bin download-model -- --root \"{root}\"\n\
         \x20  会落到 {default_dir}（若不是下面这个目录，请把下载好的文件搬过去）\n\
         2) 手动从 HuggingFace 仓库 {repo} 下载 onnx/model_quantized.onnx 与 tokenizer.json，\n\
         \x20  放进 {dir}，分别命名为 model.onnx / tokenizer.json\n\
         \x20  （若该模型需 mean 池化，另放 embed-config.json：{{\"pooling\":\"mean\"}}）",
        dir = model_dir.display(),
        root = root.display(),
        default_dir = root.join(DEFAULT_MODEL_NAME).display(),
        repo = MODEL_REPO,
    )
}

fn render_searched(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!("  {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n")
}
