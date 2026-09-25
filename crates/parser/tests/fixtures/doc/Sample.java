package com.example;

/**
 * 计算器。
 * 第二行。
 */
public class Calculator {
    /**
     * 两数之和。
     * @param a 第一个数
     * @param b 第二个数
     * @return 和
     */
    public int add(int a, int b) {
        return a + b;
    }

    // 普通注释，不是文档注释
    public int plain(int x) {
        return x;
    }

    /** 隔了空行，不算文档注释 */

    public int gap(int x) {
        return x;
    }

    /** 紧邻 other 的文档 */
    public int other(int x) {
        return x;
    }

    public int neighbor(int x) {
        return x;
    }
}
