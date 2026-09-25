/**
 * 计算器。
 */
export class Calculator {
    /**
     * 两数之和。
     * @param a 第一个数
     */
    add(a, b) {
        return a + b;
    }
}

/** 顶层函数文档 */
export function topAdd(a, b) {
    return a + b;
}

// 普通注释，不是文档注释
export function plain(x) {
    return x;
}

/** 隔了空行，不算文档注释 */

export function gap(x) {
    return x;
}

/** 紧邻 other 的文档 */
export function other(x) {
    return x;
}
export function neighbor(x) {
    return x;
}
