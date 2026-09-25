//! 补链接 `advapi32`
//!
//! libgit2-sys 在 Windows 上只声明了 winhttp / rpcrt4 / ole32 / crypt32 / secur32
//! （见 libgit2-sys build.rs 的 win32 分支），**漏了 advapi32**；
//! 而 Rust 1.95 的 std 已不再默认链接它。
//!
//! 后果：链接 codeconnect-diff 的测试二进制时报 19 个未解析符号
//! （`__imp_CryptAcquireContextA`、`__imp_RegOpenKeyExW`、`__imp_GetNamedSecurityInfoW`、
//! `__imp_OpenProcessToken` 等，全在 advapi32.lib 里），LNK1120 失败。
//!
//! 这里按目标平台补上，避免污染其它平台的链接。
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }
}
