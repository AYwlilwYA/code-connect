//! CodeConnect 文件监控模块
//!
//! - [`watcher`] — notify 文件系统事件监控、debounce、批量处理
//! - [`filter`] — .gitignore 过滤 + 文件扩展名过滤

pub mod filter;
pub mod watcher;
