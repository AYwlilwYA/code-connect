use std::path::Path;

pub mod analyze;
pub mod index;
pub mod search;
pub mod serve;
pub mod setup;
pub mod status;

/// 检查索引数据目录是否完整（不自动创建——索引应由 `codeconnect index` 命令构建）
/// 返回 Ok(()) 或 Err(错误消息)
pub fn check_index_dirs_exist(data_dir: &Path) -> Result<(), String> {
    let tantivy_dir = data_dir.join("tantivy");
    let tantivy_edges_dir = data_dir.join("tantivy_edges");
    let sled_dir = data_dir.join("sled");

    let required: [(&str, &Path); 3] = [
        ("tantivy", &tantivy_dir),
        ("调用边索引", &tantivy_edges_dir),
        ("sled", &sled_dir),
    ];
    let mut missing = Vec::new();
    for (name, dir) in &required {
        if !dir.exists() {
            missing.push(*name);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "索引数据不完整，缺失: {}\n请先运行 `codeconnect index` 构建索引。",
            missing.join(", ")
        ))
    }
}
