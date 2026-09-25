//! ONNX Runtime 动态库定位
//!
//! 本 crate 走 `ort` 的 `load-dynamic`，构建期不链接、不下载运行时。
//! 运行期 `ort` 读取 `ORT_DYLIB_PATH`；这里在初始化前把它解析成**存在**的绝对路径，
//! 以便「找不到运行时」表现为可读错误，而不是 ort 内部 `setup_api` 的 panic。
//!
//! 解析顺序：`ORT_DYLIB_PATH` → `CODECONNECT_ORT_DYLIB` → 可执行文件同目录 → `PATH`。
//!
//! 关键词：ORT_DYLIB_PATH onnxruntime.dll 动态库 加载 load-dynamic 版本

use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
const DYLIB_NAME: &str = "onnxruntime.dll";
#[cfg(target_os = "macos")]
const DYLIB_NAME: &str = "libonnxruntime.dylib";
#[cfg(all(unix, not(target_os = "macos")))]
const DYLIB_NAME: &str = "libonnxruntime.so";

/// 解析并写回 `ORT_DYLIB_PATH`；返回实际生效的绝对路径。
///
/// 失败时返回给人看的失败原因（不含修复指引，指引见 [`runtime_hint`]）。
pub fn prepare_ort_dylib() -> Result<PathBuf, String> {
    let found = resolve()?;
    verify(&found)?;
    if env_non_empty("ORT_DYLIB_PATH").is_none() {
        // SAFETY: 加载模型是单线程启动路径，此时无其它线程读写环境变量
        unsafe { std::env::set_var("ORT_DYLIB_PATH", &found) };
    }
    Ok(found)
}

/// 预校验：动态库能加载、导出 `OrtGetApiBase`、且次版本号 >= 本 crate 编译期选的 API 次版本。
///
/// 不预校验的话，ort 内部会在 `setup_api` 里直接 panic —— release 档 `panic = "abort"` 会
/// 让整个进程（MCP 服务器）当场终止，调用方只看到连接断开。
fn verify(path: &Path) -> Result<(), String> {
    use ort::sys::OrtApiBase;

    // SAFETY: 只读取 OrtGetApiBase 并调用其 GetVersionString，指针在使用期间由 lib 持有
    unsafe {
        let lib = libloading::Library::new(path)
            .map_err(|e| format!("加载动态库失败（{}）：{e}", path.display()))?;
        let getter: libloading::Symbol<unsafe extern "system" fn() -> *const OrtApiBase> = lib
            .get(b"OrtGetApiBase")
            .map_err(|e| format!("动态库缺少 `OrtGetApiBase` 导出（{}）：{e}", path.display()))?;
        let base = getter();
        if base.is_null() {
            return Err(format!("`OrtGetApiBase` 返回空指针（{}）", path.display()));
        }
        let raw = ((*base).GetVersionString)();
        if raw.is_null() {
            return Err(format!("`GetVersionString` 返回空指针（{}）", path.display()));
        }
        let version = std::ffi::CStr::from_ptr(raw).to_string_lossy().to_string();
        let minor = version
            .split('.')
            .nth(1)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        if minor < crate::ORT_API_MINOR {
            return Err(format!(
                "动态库版本过低：{} 报 {version}，本 crate 编译时选了 ONNX Runtime API 1.{}（要求次版本 >= {}）",
                path.display(),
                crate::ORT_API_MINOR,
                crate::ORT_API_MINOR
            ));
        }
        Ok(())
    }
}

fn resolve() -> Result<PathBuf, String> {
    if let Some(p) = env_non_empty("ORT_DYLIB_PATH") {
        return existing(PathBuf::from(&p), &format!("ORT_DYLIB_PATH={p}"));
    }
    if let Some(p) = env_non_empty("CODECONNECT_ORT_DYLIB") {
        return existing(PathBuf::from(&p), &format!("CODECONNECT_ORT_DYLIB={p}"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(DYLIB_NAME);
            if cand.is_file() {
                return Ok(cand);
            }
        }
    }
    if let Some(p) = search_path() {
        return Ok(p);
    }
    Err(format!(
        "未能在 ORT_DYLIB_PATH / CODECONNECT_ORT_DYLIB / 可执行文件同目录 / PATH 中定位到 {DYLIB_NAME}"
    ))
}

fn existing(p: PathBuf, source: &str) -> Result<PathBuf, String> {
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("{source} 指向的 {DYLIB_NAME} 不存在：{}", p.display()))
    }
}

/// 「运行时不可用」的修复指引
pub fn runtime_hint() -> String {
    format!(
        "请准备 ONNX Runtime 动态库（{name}，次版本须 >= {min}），任选一种放置方式：\n\
         \x20 1) 放到 codeconnect 可执行文件同目录或 PATH 上；\n\
         \x20 2) 用环境变量指定绝对路径：\n\
         \x20      PowerShell: $env:ORT_DYLIB_PATH = '<...>\\{name}'\n\
         \x20      也可用 CODECONNECT_ORT_DYLIB。",
        name = DYLIB_NAME,
        min = crate::ORT_API_MINOR
    )
}

fn env_non_empty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

fn search_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d: PathBuf| d.join(DYLIB_NAME))
        .find(|c: &PathBuf| is_loadable(c))
}

fn is_loadable(p: &Path) -> bool {
    p.is_file()
}
