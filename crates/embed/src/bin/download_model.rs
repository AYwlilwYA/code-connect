//! 嵌入模型下载器
//!
//! 用法：
//!   cargo run -p codeconnect-embed --features downloader --bin download-model \
//!     -- [模型名] [--root 根目录] [--force] [--endpoint URL]
//!
//! 关键词：下载模型 download-model 根目录 落盘路径 体积

use std::path::PathBuf;

use codeconnect_embed::download::{DEFAULT_ENDPOINT, download_model_to};
use codeconnect_embed::{DEFAULT_MODEL_NAME, models_root};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut name = DEFAULT_MODEL_NAME.to_string();
    let mut root = models_root();
    let mut force = false;
    let mut endpoint =
        std::env::var("CODECONNECT_HF_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--force" => force = true,
            "--root" => {
                if let Some(v) = it.next() {
                    root = PathBuf::from(v);
                }
            }
            "--endpoint" => {
                if let Some(v) = it.next() {
                    endpoint = v.clone();
                }
            }
            "-h" | "--help" => {
                println!(
                    "用法: download-model [模型名] [--root 根目录] [--force] [--endpoint URL]\n\
                     默认模型: {DEFAULT_MODEL_NAME}\n\
                     默认根目录: {}\n\
                     下载源可用环境变量 CODECONNECT_HF_ENDPOINT 覆盖",
                    models_root().display()
                );
                return;
            }
            other => name = other.to_string(),
        }
    }

    eprintln!("模型: {name}");
    eprintln!("根目录: {}", root.display());
    eprintln!("落盘: {}", root.join(&name).display());
    eprintln!("下载源: {endpoint}");

    match download_model_to(&root, &name, &endpoint, force) {
        Ok(report) => {
            let total: u64 = report.files.iter().map(|(_, s)| *s).sum();
            println!("模型目录: {}", report.dir.display());
            for (f, s) in &report.files {
                println!("  {f}  {s} 字节");
            }
            println!("合计: {total} 字节（{:.1} MB）", total as f64 / 1_048_576.0);
            if report.all_skipped {
                println!("（全部已存在，未重新下载；加 --force 可强制重下）");
            }
        }
        Err(e) => {
            eprintln!("下载失败：{e}");
            std::process::exit(1);
        }
    }
}
