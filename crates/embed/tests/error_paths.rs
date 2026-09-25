//! 「未配置」与「模型缺失」必须分开，且绝不静默降级
//!
//! 关键词：未配置 关闭 模型缺失 明确报错 不降级 词法 EmbedError ModelSource

use std::path::PathBuf;
use std::sync::Mutex;

use codeconnect_embed::{
    EmbedError, EmbedErrorKind, Embedder, ModelSource, load, load_source, resolve_model_dir,
};

/// 测试会改环境变量，串行化避免互相干扰
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("codeconnect-embed-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).expect("建临时目录失败");
    d
}

// ---------- 未配置：一等状态，是「能力关闭」不是「故障」 ----------

#[test]
fn 未配置是关闭状态而不是故障() {
    assert_eq!(ModelSource::from_config(None), ModelSource::NotConfigured);
    assert_eq!(
        ModelSource::from_config(Some("")),
        ModelSource::NotConfigured
    );
    assert_eq!(
        ModelSource::from_config(Some("   ")),
        ModelSource::NotConfigured
    );
    assert!(!ModelSource::from_config(None).is_configured());

    let err = load_source(&ModelSource::NotConfigured, &temp_dir("未配置"))
        .err()
        .expect("未配置时 load_source 必须返回 Err（不是 Ok，也不是 panic）");
    assert!(
        matches!(err, EmbedError::NotConfigured),
        "实得：{err:?}"
    );
    assert!(err.is_not_configured());
    assert_eq!(err.kind(), EmbedErrorKind::NotConfigured);
    let msg = err.to_string();
    assert!(
        msg.contains("未配置") && msg.contains("关闭"),
        "文案必须说清是「没配」而不是「坏了」：{msg}"
    );
    // 未配置时绝不能出现「模型缺失」这类误导字样
    assert!(
        !msg.contains("不存在"),
        "未配置不该被说成路径不存在：{msg}"
    );
}

// ---------- 可指定模型：按名 / 按路径 ----------

#[test]
fn 按名解析命中时返回目录() {
    let root = temp_dir("按名命中");
    let want = root.join("my-model");
    std::fs::create_dir_all(&want).unwrap();
    assert_eq!(resolve_model_dir(&root, "my-model").unwrap(), want);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn 按名解析落空时报出搜索过的路径() {
    let root = temp_dir("按名落空");
    let err = resolve_model_dir(&root, "no-such-model")
        .err()
        .expect("找不到必须报错");
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        matches!(err, EmbedError::ModelDirMissing { .. }),
        "实得：{err:?}"
    );
    assert_eq!(err.kind(), EmbedErrorKind::ModelNotReady);
    assert!(!err.is_not_configured());
    let msg = err.to_string();
    assert!(
        msg.contains(&root.join("no-such-model").display().to_string()),
        "必须给出期望目录：{msg}"
    );
    assert!(msg.contains("已搜索"), "必须列出搜索过的路径：{msg}");
    assert!(msg.contains("download-model"), "必须给出获取方式：{msg}");
}

#[test]
fn 配置里直接写路径时按路径用() {
    let dir = temp_dir("直接路径");
    std::fs::write(dir.join("model.onnx"), b"x").unwrap();
    std::fs::write(dir.join("tokenizer.json"), b"{}").unwrap();

    let src = ModelSource::from_config(Some(&dir.display().to_string()));
    assert_eq!(src, ModelSource::Dir(dir.clone()));
    assert!(src.is_configured());
    // 根目录随便给一个，路径来源不该受它影响
    assert_eq!(src.resolve(&temp_dir("无关根目录")).unwrap(), dir);
    assert_eq!(src.describe(), dir.display().to_string());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 配置里写名字时按名字解析() {
    let root = temp_dir("配置写名字");
    // 名字不存在 ⇒ 走按名解析，报的是「没就绪」而不是「没配」
    let src = ModelSource::from_config(Some("not-installed-model"));
    assert_eq!(src, ModelSource::Named("not-installed-model".to_string()));
    assert!(src.is_configured());
    let err = load_source(&src, &root).err().expect("应报错");
    assert_eq!(err.kind(), EmbedErrorKind::ModelNotReady);
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- 模型缺失（旧契约仍成立） ----------

#[test]
fn 模型目录不存在时返回明确错误() {
    let missing = std::env::temp_dir().join("codeconnect-embed-不存在的模型目录");
    let err = load(&missing).err().expect("目录不存在时必须报错");
    assert!(
        matches!(err, EmbedError::ModelDirMissing { .. }),
        "实得：{err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("未就绪"), "错误消息未说明模型未就绪：{msg}");
    assert!(
        msg.contains("download-model"),
        "错误消息未给出获取方式：{msg}"
    );
    assert!(
        msg.contains(&missing.display().to_string()),
        "错误消息未给出期望路径：{msg}"
    );
}

#[test]
fn 模型文件不全时返回明确错误() {
    let dir = temp_dir("空目录");
    let err = Embedder::load(&dir).err().expect("文件不全时必须报错");
    let _ = std::fs::remove_dir(&dir);
    assert!(
        matches!(err, EmbedError::ModelIncomplete { .. }),
        "实得：{err:?}"
    );
    assert_eq!(err.kind(), EmbedErrorKind::ModelNotReady);
    let msg = err.to_string();
    assert!(
        msg.contains("model.onnx") && msg.contains("tokenizer.json"),
        "错误消息未列出缺失文件：{msg}"
    );
}

// ---------- 运行时缺失：不静默降级 ----------

#[test]
fn 未找到运行时不静默降级() {
    let dir = temp_dir("假模型");
    std::fs::write(dir.join("model.onnx"), b"not a real model").unwrap();
    std::fs::write(dir.join("tokenizer.json"), b"{}").unwrap();

    let _guard = ENV_LOCK.lock().unwrap();
    let saved = std::env::var_os("ORT_DYLIB_PATH");
    unsafe { std::env::set_var("ORT_DYLIB_PATH", "Z:/definitely/not/here/onnxruntime.dll") };

    let err = Embedder::load(&dir).err().expect("找不到运行时时必须报错");
    unsafe {
        match saved {
            Some(v) => std::env::set_var("ORT_DYLIB_PATH", v),
            None => std::env::remove_var("ORT_DYLIB_PATH"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        matches!(err, EmbedError::RuntimeUnavailable { .. }),
        "实得：{err:?}"
    );
    assert_eq!(err.kind(), EmbedErrorKind::Runtime);
    let msg = err.to_string();
    assert!(
        msg.contains("ORT_DYLIB_PATH") && msg.contains("onnxruntime"),
        "错误消息未指向 ONNX Runtime 与来源变量：{msg}"
    );
}
