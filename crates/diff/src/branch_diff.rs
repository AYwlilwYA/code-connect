//! Git 分支差异比较
//!
//! 使用 git2 库比较两个 Git 引用（分支/标签/commit），
//! 返回变更文件列表及变更类型（新增/修改/删除/重命名）。

use serde::{Deserialize, Serialize};

/// 文件变更类型
///
/// 表示文件在两个引用之间的变更状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    /// 新增文件
    Added,
    /// 修改文件
    Modified,
    /// 删除文件
    Deleted,
    /// 重命名文件（从旧路径到新路径）
    Renamed,
    /// 类型变更（如文件变为目录）
    TypeChanged,
}

/// 变更文件条目
///
/// 描述单个文件在两个 Git 引用之间的变化详情。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    /// 文件路径（相对于仓库根目录）
    pub path: String,
    /// 旧路径（重命名情况下有值）
    pub old_path: Option<String>,
    /// 变更类型
    pub change_kind: ChangeKind,
    /// 是否为二进制文件
    pub is_binary: bool,
}

/// 分支差异结果
///
/// 汇总两个引用之间的所有文件变更。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiff {
    /// 源引用名称
    pub from_ref: String,
    /// 目标引用名称
    pub to_ref: String,
    /// 变更文件列表
    pub changed_files: Vec<ChangedFile>,
    /// 新增文件数
    pub added_count: usize,
    /// 修改文件数
    pub modified_count: usize,
    /// 删除文件数
    pub deleted_count: usize,
}

/// 分支差异比较器
///
/// 使用 git2 库比较同一仓库中两个引用之间的差异，
/// 返回结构化的变更文件列表。
pub struct BranchDiffer;

impl BranchDiffer {
    /// 比较两个 Git 引用之间的差异
    ///
    /// 打开指定路径的 Git 仓库，解析两个引用（分支名、标签名或 commit SHA），
    /// 通过 diff 树获取所有变更文件。
    ///
    /// # 参数
    /// - `repo_path` — Git 仓库的本地路径
    /// - `from_ref` — 源引用名称（如 `"main"`, `"HEAD~3"`, `"abc1234"`）
    /// - `to_ref` — 目标引用名称
    ///
    /// # 返回
    /// 包含所有变更文件的差异结果
    ///
    /// # 错误
    /// - 仓库打不开
    /// - 引用无法解析
    /// - diff 计算失败
    pub fn diff(
        repo_path: &str,
        from_ref: &str,
        to_ref: &str,
    ) -> Result<BranchDiff, String> {
        // 打开 Git 仓库
        let repo = git2::Repository::open(repo_path)
            .map_err(|e| format!("无法打开 Git 仓库 '{}': {}", repo_path, e))?;

        // 解析源引用
        let from_commit = Self::resolve_ref(&repo, from_ref)?;
        let from_tree = from_commit
            .tree()
            .map_err(|e| format!("无法获取 '{}' 的树对象: {}", from_ref, e))?;

        // 解析目标引用
        let to_commit = Self::resolve_ref(&repo, to_ref)?;
        let to_tree = to_commit
            .tree()
            .map_err(|e| format!("无法获取 '{}' 的树对象: {}", to_ref, e))?;

        // 计算 diff
        let diff = repo
            .diff_tree_to_tree(
                Some(&from_tree),
                Some(&to_tree),
                None, // 使用默认选项
            )
            .map_err(|e| format!("计算 diff 失败: {}", e))?;

        // 收集变更文件
        let mut changed_files = Vec::new();
        let mut added_count = 0;
        let mut modified_count = 0;
        let mut deleted_count = 0;

        diff.foreach(
            &mut |delta, _progress| {
                let change_kind = match delta.status() {
                    git2::Delta::Added => ChangeKind::Added,
                    git2::Delta::Modified => ChangeKind::Modified,
                    git2::Delta::Deleted => ChangeKind::Deleted,
                    git2::Delta::Renamed => ChangeKind::Renamed,
                    git2::Delta::Typechange => ChangeKind::TypeChanged,
                    _ => ChangeKind::Modified, // 其他类型视为修改
                };

                // 获取新文件路径
                let new_file = delta.new_file();
                let path = new_file
                    .path()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();

                // 获取旧文件路径（仅重命名时有意义）
                let old_file = delta.old_file();
                let old_path = if delta.status() == git2::Delta::Renamed {
                    old_file
                        .path()
                        .map(|p| p.to_string_lossy().to_string())
                } else {
                    None
                };

                let is_binary = new_file.is_binary();

                let changed_file = ChangedFile {
                    path,
                    old_path,
                    change_kind,
                    is_binary,
                };

                changed_files.push(changed_file);
                true // 继续遍历
            },
            None,
            None,
            None,
        )
        .map_err(|e| format!("遍历 diff 失败: {}", e))?;

        // 统计各类型数量
        for file in &changed_files {
            match file.change_kind {
                ChangeKind::Added => added_count += 1,
                ChangeKind::Modified => modified_count += 1,
                ChangeKind::Deleted => deleted_count += 1,
                // Renamed 和 TypeChanged 计入 Modified
                _ => modified_count += 1,
            }
        }

        Ok(BranchDiff {
            from_ref: from_ref.to_string(),
            to_ref: to_ref.to_string(),
            changed_files,
            added_count,
            modified_count,
            deleted_count,
        })
    }

    /// 解析 Git 引用为 commit 对象
    ///
    /// 支持分支名、标签名和 commit SHA。
    fn resolve_ref<'repo>(
        repo: &'repo git2::Repository,
        ref_name: &str,
    ) -> Result<git2::Commit<'repo>, String> {
        // 尝试直接解析为 Oid（commit SHA）
        if let Ok(oid) = git2::Oid::from_str(ref_name) {
            return repo
                .find_commit(oid)
                .map_err(|e| format!("无法找到 commit '{}': {}", ref_name, e));
        }

        // 尝试作为引用名解析（refs/heads/、refs/tags/、HEAD）
        let reference = repo
            .resolve_reference_from_short_name(ref_name)
            .map_err(|e| format!("无法解析引用 '{}': {}", ref_name, e))?;

        let commit = reference
            .peel_to_commit()
            .map_err(|e| format!("无法将 '{}' 解析为 commit: {}", ref_name, e))?;

        Ok(commit)
    }

    /// 列出两个引用之间的变更文件路径（简化版）
    ///
    /// 返回变更文件的路径列表，不做分类统计。
    /// 适合只需要文件路径列表的场景。
    ///
    /// # 参数
    /// - `repo_path` — Git 仓库的本地路径
    /// - `from_ref` — 源引用名称
    /// - `to_ref` — 目标引用名称
    pub fn list_changed_files(
        repo_path: &str,
        from_ref: &str,
        to_ref: &str,
    ) -> Result<Vec<String>, String> {
        let branch_diff = Self::diff(repo_path, from_ref, to_ref)?;
        Ok(branch_diff
            .changed_files
            .into_iter()
            .map(|f| f.path)
            .collect())
    }
}
