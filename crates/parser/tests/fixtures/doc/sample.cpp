#include <string>

/**
 * 图形基类。
 */
class Shape {
public:
    /** 面积。 */
    virtual double area() const = 0;

    /// 缩放。
    double scaled(double f) const;

    // 普通注释，不是文档注释
    void plain();

    /** 隔了空行，不算文档注释 */

    void gap();
};

/// 自由函数文档
int add(int a, int b) {
    return a + b;
}

/// 紧邻 other 的文档
int other(int a) {
    return a;
}
int neighbor(int a) {
    return a;
}
