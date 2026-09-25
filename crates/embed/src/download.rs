//! 模型下载：落到用户级模型目录，所有项目共用一份
//!
//! 关键词：下载模型 download huggingface hf-mirror 体积 落盘 endpoint

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ureq::tls::{TlsConfig, TlsProvider};

use crate::model_dir::{DEFAULT_MODEL_NAME, MODEL_REPO, models_root};

/// 默认下载源（hf-mirror 是可达的 HuggingFace 镜像）
pub const DEFAULT_ENDPOINT: &str = "https://hf-mirror.com";

/// 待下载文件：(仓库内路径, 本地文件名, 期望字节数)
///
/// 用 int8 量化版：实测判别性足够，体积只有 fp32 的 1/4（470 MB → 118 MB）。
/// 期望字节数是硬校验，防止半截文件被当成完整模型用。
pub const FILES: &[(&str, &str, u64)] = &[
    (
        "onnx/model_quantized.onnx",
        "model.onnx",
        118_308_126,
    ),
    ("tokenizer.json", "tokenizer.json", 17_082_913),
];

/// 默认模型的池化方式，随模型一起落盘到 `embed-config.json`，避免运行期猜测
pub const DEFAULT_POOLING: &str = "mean";

/// 下载失败
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// 网络或 HTTP 失败
    #[error("下载 {url} 失败：{source}")]
    Http {
        /// 请求地址
        url: String,
        /// 底层错误
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// 本地文件操作失败
    #[error("写文件失败 {path}：{source}")]
    Io {
        /// 目标路径
        path: PathBuf,
        /// 底层错误
        source: std::io::Error,
    },

    /// 下载到的字节数与期望不符
    #[error("文件 {name} 体积不符：期望 {expected} 字节，实得 {got} 字节（下载可能被截断）")]
    SizeMismatch {
        /// 文件名
        name: String,
        /// 期望字节数
        expected: u64,
        /// 实得字节数
        got: u64,
    },

    /// 请求了下载器不认识的模型
    #[error("不支持下载模型 `{name}`；当前仅支持 `{supported}`")]
    UnknownModel {
        /// 请求的模型名
        name: String,
        /// 唯一受支持的模型名
        supported: String,
    },
}

/// 下载结果
#[derive(Debug, Clone)]
pub struct Report {
    /// 模型落盘目录
    pub dir: PathBuf,
    /// 每个文件的实际体积
    pub files: Vec<(String, u64)>,
    /// 是否全部为已存在而跳过
    pub all_skipped: bool,
}

/// 下载默认模型的全部文件到**默认根目录**（`<models_root>/<name>`）
pub fn download_model(name: &str, endpoint: &str, force: bool) -> Result<Report, DownloadError> {
    download_model_to(&models_root(), name, endpoint, force)
}

/// 下载到**指定根目录**（`<root>/<name>`），供自定义根目录的用户使用
pub fn download_model_to(
    root: &Path,
    name: &str,
    endpoint: &str,
    force: bool,
) -> Result<Report, DownloadError> {
    if name != DEFAULT_MODEL_NAME {
        return Err(DownloadError::UnknownModel {
            name: name.to_string(),
            supported: DEFAULT_MODEL_NAME.to_string(),
        });
    }
    let dest = root.join(name);
    fs::create_dir_all(&dest).map_err(|e| DownloadError::Io {
        path: dest.clone(),
        source: e,
    })?;

    let mut files = Vec::new();
    let mut all_skipped = true;
    for (remote, local, expected) in FILES {
        let target = dest.join(local);
        if !force && target.is_file() && fs::metadata(&target).map(|m| m.len()).unwrap_or(0) == *expected
        {
            let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            eprintln!("[跳过] {} 已存在（{} 字节）", target.display(), size);
            files.push(((*local).to_string(), size));
            continue;
        }
        all_skipped = false;
        let size = fetch_one(endpoint, remote, &target, local, *expected)?;
        files.push(((*local).to_string(), size));
    }

    write_embed_config(&dest)?;
    write_model_info(&dest, name, endpoint, &files)?;
    Ok(Report {
        dir: dest,
        files,
        all_skipped,
    })
}

/// 写 `<dir>/embed-config.json`：池化方式是模型属性，必须随模型落盘
fn write_embed_config(dir: &Path) -> Result<(), DownloadError> {
    let cfg = serde_json::json!({ "pooling": DEFAULT_POOLING });
    let path = dir.join("embed-config.json");
    let text = serde_json::to_string_pretty(&cfg).unwrap_or_default();
    fs::write(&path, text).map_err(|e| DownloadError::Io { path, source: e })
}

fn fetch_one(
    endpoint: &str,
    remote: &str,
    target: &Path,
    local: &str,
    expected: u64,
) -> Result<u64, DownloadError> {
    let url = format!(
        "{}/{}/resolve/main/{}",
        endpoint.trim_end_matches('/'),
        MODEL_REPO,
        remote
    );
    eprintln!("[下载] {url}");

    // 先写 .part 再改名，避免半截文件被误当成完整模型
    let part = target.with_extension(format!(
        "{}part",
        target
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{e}."))
            .unwrap_or_default()
    ));

    // 本 crate 只启用 ureq 的 native-tls（Windows 走 schannel，免 C 工具链），
    // 但 ureq 默认 provider 是 rustls，必须显式切过来，否则 https 请求直接 panic。
    let config = ureq::Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .build(),
        )
        .build();
    let agent = ureq::Agent::new_with_config(config);

    let mut response = agent.get(&url).call().map_err(|e| DownloadError::Http {
        url: url.clone(),
        source: Box::new(e),
    })?;
    let total = response.body_mut().content_length();
    let mut reader = response.body_mut().as_reader();

    let mut file = fs::File::create(&part).map_err(|e| DownloadError::Io {
        path: part.clone(),
        source: e,
    })?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut written: u64 = 0;
    let mut next_report: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| DownloadError::Io {
            path: part.clone(),
            source: e,
        })?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| DownloadError::Io {
            path: part.clone(),
            source: e,
        })?;
        written += n as u64;
        if written >= next_report {
            match total {
                Some(t) if t > 0 => eprintln!(
                    "       {written}/{t} 字节（{:.1}%）",
                    written as f64 * 100.0 / t as f64
                ),
                _ => eprintln!("       {written} 字节"),
            }
            next_report = written + 10 * 1024 * 1024;
        }
    }
    file.sync_all().map_err(|e| DownloadError::Io {
        path: part.clone(),
        source: e,
    })?;
    drop(file);

    if expected > 0 && written != expected {
        return Err(DownloadError::SizeMismatch {
            name: local.to_string(),
            expected,
            got: written,
        });
    }
    fs::rename(&part, target).map_err(|e| DownloadError::Io {
        path: target.to_path_buf(),
        source: e,
    })?;
    Ok(written)
}

/// 在模型目录留一份来源与体积记录，便于事后追溯
fn write_model_info(
    dir: &Path,
    name: &str,
    endpoint: &str,
    files: &[(String, u64)],
) -> Result<(), DownloadError> {
    let info = serde_json::json!({
        "model": name,
        "repo": MODEL_REPO,
        "endpoint": endpoint,
        "files": files.iter().map(|(n, s)| serde_json::json!({"name": n, "bytes": s})).collect::<Vec<_>>(),
        "total_bytes": files.iter().map(|(_, s)| *s).sum::<u64>(),
    });
    let path = dir.join("model-info.json");
    let text = serde_json::to_string_pretty(&info).unwrap_or_default();
    fs::write(&path, text).map_err(|e| DownloadError::Io { path, source: e })
}
