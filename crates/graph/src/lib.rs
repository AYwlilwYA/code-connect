//! CodeConnect 图分析模块
//!
//! 基于 petgraph 的代码关系图构建与分析：
//! - [`call_graph`] — 符号级调用图（构建、BFS/DFS 遍历、双向查询）
//! - [`dep_graph`] — 三级依赖图（文件级、符号级、模块级）
//! - [`type_hierarchy`] — 类型继承树、CHA 虚拟调用解析
//! - [`cycle_detect`] — 循环依赖检测（Kosaraju SCC + 拓扑排序）
//! - [`cache`] — 图数据 LRU 缓存

pub mod cache;
pub mod call_graph;
pub mod cycle_detect;
pub mod dep_graph;
pub mod metrics;
pub mod type_hierarchy;
