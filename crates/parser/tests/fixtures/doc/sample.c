#include <stdio.h>

/**
 * 两数之和。
 * @param a 第一个数
 * @param b 第二个数
 */
int add(int a, int b) {
    return a + b;
}

/// 行文档函数
void triple(int x) {
    (void)x;
}

// 普通注释，不是文档注释
void plain(int x) {
    (void)x;
}

/** 隔了空行，不算文档注释 */

void gap(int x) {
    (void)x;
}

/** 紧邻 other 的文档 */
void other(int x) {
    (void)x;
}
void neighbor(int x) {
    (void)x;
}
