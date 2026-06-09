//! 文件系统监控
//!
//! 基于 notify crate 的跨平台文件系统事件监控，
//! 支持 debounce 去抖和批量事件处理。
//!
//! # 架构
//!
//! - [`FileWatcher`] 封装 `notify::PollWatcher`，在内部事件循环中
//!   接收原始文件变更事件
//! - 事件按 500ms 窗口进行 debounce 合并，同一批次内的所有变更文件
//!   以去重后的路径集合形式通过 tokio channel 发送出去
//! - 使用 `filter::should_watch` 预过滤不相关的事件

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Config, Event, EventKind, PollWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::filter::should_watch;

/// debounce 窗口时长（毫秒）
const DEBOUNCE_MS: u64 = 500;

/// 文件系统监控器
///
/// 封装 notify PollWatcher，负责监控项目目录中的源文件变更，
/// 在 debounce 后批量发送变更文件路径。
///
/// # 使用示例
///
/// ```ignore
/// let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
/// let watcher = FileWatcher::new(project_root, excludes)?;
/// watcher.start(tx).await?;
///
/// // 从 rx 接收变更事件
/// while let Some(changed_files) = rx.recv().await {
///     println!("检测到 {} 个文件变更", changed_files.len());
///     // 处理增量索引更新...
/// }
/// ```
pub struct FileWatcher {
    /// 项目根目录
    project_root: PathBuf,
    /// 排除模式列表（glob 格式）
    excludes: Vec<String>,
}

impl FileWatcher {
    /// 创建新的文件监控器
    ///
    /// # 参数
    ///
    /// - `project_root` — 项目根目录路径，所有监控事件路径均以此为基准
    /// - `excludes` — 排除模式列表，用于过滤不需要监控的目录/文件
    pub fn new(project_root: &Path, excludes: Vec<String>) -> Self {
        Self {
            project_root: project_root.to_path_buf(),
            excludes,
        }
    }

    /// 启动异步文件监控
    ///
    /// 此方法会阻塞当前线程（在 notify 事件循环中运行），
    /// 因此调用方应在独立的 tokio 任务中调用它。
    ///
    /// # 参数
    ///
    /// - `tx` — 用于发送 debounce 后变更文件集合的通道
    ///
    /// # 工作流程
    ///
    /// 1. 创建 `PollWatcher` 并递归监控项目根目录
    /// 2. 在事件循环中接收原始 notify 事件
    /// 3. 过滤无关事件（排除不支持的文件、匹配排除规则的文件）
    /// 4. 在 500ms debounce 窗口内收集变更路径
    /// 5. 窗口期满后发送去重后的路径集合
    pub async fn start(
        &self,
        tx: mpsc::UnboundedSender<Vec<PathBuf>>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let project_root = self.project_root.clone();
        let excludes = Arc::new(self.excludes.clone());

        // 创建 notify PollWatcher
        let config = Config::default()
            .with_poll_interval(Duration::from_secs(2)); // 每 2 秒轮询一次

        let (event_tx, event_rx) = std::sync::mpsc::channel();

        let mut poll_watcher = PollWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = event_tx.send(event);
                }
            },
            config,
        )?;

        // 递归监控项目根目录
        poll_watcher.watch(&project_root, RecursiveMode::Recursive)?;

        // --------------------------------------------------------------------
        // Debounce 事件循环（在独立的 tokio 阻塞任务中运行）
        // --------------------------------------------------------------------
        tokio::task::spawn_blocking(move || {
            // 当前 debounce 窗口开始时间
            let mut window_start: Option<std::time::Instant> = None;
            // 当前窗口内收集的变更路径集合
            let mut pending_paths: HashSet<PathBuf> = HashSet::new();

            loop {
                // 计算下次接收事件的超时时间
                let timeout = window_start.map_or(
                    Duration::from_secs(3600), // 初始状态无窗口，长时间等待
                    |start| {
                        let elapsed = start.elapsed();
                        if elapsed >= Duration::from_millis(DEBOUNCE_MS) {
                            Duration::ZERO
                        } else {
                            Duration::from_millis(DEBOUNCE_MS) - elapsed
                        }
                    },
                );

                // 带超时的接收事件
                match event_rx.recv_timeout(timeout) {
                    Ok(event) => {
                        // 处理事件，收集变更路径
                        let changed_paths = extract_changed_paths(&event);

                        for path in changed_paths {
                            // 过滤不相关的文件
                            if should_watch(&path, &excludes) {
                                if window_start.is_none() {
                                    window_start = Some(std::time::Instant::now());
                                }
                                pending_paths.insert(path);
                            }
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        // 超时：当前 debounce 窗口期满
                        if let Some(_start) = window_start.take() {
                            if !pending_paths.is_empty() {
                                let batch: Vec<PathBuf> =
                                    pending_paths.drain().collect();
                                tracing::debug!(
                                    "文件监控 debounce 完成，发送 {} 个变更文件",
                                    batch.len()
                                );
                                // 发送到上层处理，忽略通道关闭错误
                                let _ = tx.send(batch);
                            }
                        }
                        // 重置窗口
                        window_start = None;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        // 事件通道断开，停止监控
                        tracing::info!("文件监控事件通道已关闭，停止监控");
                        break;
                    }
                }
            }
        })
        .await
        .map_err(|e| format!("监控任务失败: {}", e))?;

        Ok(())
    }
}

/// 从 notify Event 中提取变更的文件路径集合
///
/// 处理以下事件类型：
/// - **Create** — 新文件创建
/// - **Modify** — 文件内容修改
/// - **Remove** — 文件删除
/// - **Rename** — 文件重命名
///
/// 对于每种事件类型，收集第一个路径（通常为文件自身路径）。
fn extract_changed_paths(event: &Event) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
            for path in &event.paths {
                paths.push(path.clone());
            }
        }
        EventKind::Any | EventKind::Access(_) | EventKind::Other => {
            // 忽略 access 和未分类事件
        }
    }

    paths
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_watcher_creation() {
        let root = PathBuf::from("/tmp/test_project");
        let excludes = vec!["**/node_modules/**".to_string()];
        let watcher = FileWatcher::new(&root, excludes);

        assert_eq!(watcher.project_root, root);
        assert_eq!(watcher.excludes.len(), 1);
    }

    #[test]
    fn test_file_watcher_empty_excludes() {
        let watcher = FileWatcher::new(Path::new("/tmp/project"), vec![]);
        assert!(watcher.excludes.is_empty());
    }

    #[test]
    fn test_extract_changed_paths_modify() {
        use notify::event::ModifyKind;

        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("src/main.rs")],
            attrs: Default::default(),
        };

        let paths = extract_changed_paths(&event);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("src/main.rs"));
    }

    #[test]
    fn test_extract_changed_paths_create() {
        use notify::event::CreateKind;

        let event = Event {
            kind: EventKind::Create(CreateKind::File),
            paths: vec![PathBuf::from("src/new_module.rs")],
            attrs: Default::default(),
        };

        let paths = extract_changed_paths(&event);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("src/new_module.rs"));
    }

    #[test]
    fn test_extract_changed_paths_remove() {
        use notify::event::RemoveKind;

        let event = Event {
            kind: EventKind::Remove(RemoveKind::File),
            paths: vec![PathBuf::from("src/old_file.ts")],
            attrs: Default::default(),
        };

        let paths = extract_changed_paths(&event);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], PathBuf::from("src/old_file.ts"));
    }

    #[test]
    fn test_extract_changed_paths_access_ignored() {
        use notify::event::AccessKind;

        let event = Event {
            kind: EventKind::Access(AccessKind::Read),
            paths: vec![PathBuf::from("src/main.rs")],
            attrs: Default::default(),
        };

        let paths = extract_changed_paths(&event);
        assert!(paths.is_empty());
    }
}
