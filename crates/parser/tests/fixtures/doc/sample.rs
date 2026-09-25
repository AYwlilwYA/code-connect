//! Rust 文档注释 fixture
//!
//! 覆盖：`///` 多行、普通注释、隔空行、隔别的声明、`#[doc = "..."]`

/// 计算两数之和。
/// 第二行说明。
///
/// # 示例
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

// 普通注释，不是文档注释
pub fn plain(x: i32) -> i32 {
    x
}

/// 隔了空行，不算文档注释

pub fn gap(x: i32) -> i32 {
    x
}

/// 紧邻 other 的文档
pub fn other(x: i32) -> i32 {
    x
}
pub fn neighbor(x: i32) -> i32 {
    x
}

#[doc = "属性形式的文档"]
pub fn attr_doc(x: i32) -> i32 {
    x
}

/// 长度单位
pub type Meters = f64;

/// 结构体文档
pub struct Point {
    /// 字段文档
    pub x: Meters,
}

impl Point {
    /// 方法文档
    pub fn len(&self) -> usize {
        0
    }
}
