/**
 * 计算器。
 */
export class Calculator {
    /**
     * 两数之和。
     * @param a 第一个数
     */
    add(a: number, b: number): number {
        return a + b;
    }
}

/** 顶层函数文档 */
export function topAdd(a: number, b: number): number {
    return a + b;
}

// 普通注释，不是文档注释
export function plain(x: number): number {
    return x;
}

/** 隔了空行，不算文档注释 */

export function gap(x: number): number {
    return x;
}

/** 紧邻 other 的文档 */
export function other(x: number): number {
    return x;
}
export function neighbor(x: number): number {
    return x;
}
