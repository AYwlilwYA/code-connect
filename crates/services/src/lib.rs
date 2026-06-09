//! CodeConnect 业务逻辑服务层
//!
//! 封装所有代码分析用例：
//! - [`symbol_service`] — 符号查找：精确/模糊/NL 描述 + 类型层次
//! - [`call_analyzer`] — 调用链分析：双向遍历 + 数据流
//! - [`semantic_search`] — 语义搜索：全文 + AST 哈希相似 + 模式
//! - [`impact_analyzer`] — 变更影响分析：BFS 调用链 → 严重度分类
//! - [`arch_query`] — 架构查询：依赖图 + 规则引擎 + 约束验证
//! - [`deadcode`] — 死代码检测
//! - [`metrics`] — 代码质量指标：圈复杂度、扇入扇出、注释率

pub mod arch_query;
pub mod call_analyzer;
pub mod deadcode;
pub mod impact_analyzer;
pub mod metrics;
pub mod semantic_search;
pub mod symbol_service;
