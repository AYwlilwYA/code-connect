using System;

namespace Demo
{
    /// 计算器。
    /// 第二行。
    public class Calculator
    {
        /// <summary>
        /// 两数之和。
        /// </summary>
        public int Add(int a, int b)
        {
            return a + b;
        }

        /** 块文档方法。 */
        public int BlockDoc(int x)
        {
            return x;
        }

        // 普通注释，不是文档注释
        public int Plain(int x)
        {
            return x;
        }

        /// 隔了空行，不算文档注释

        public int Gap(int x)
        {
            return x;
        }

        /// 紧邻 Other 的文档
        public int Other(int x)
        {
            return x;
        }

        public int Neighbor(int x)
        {
            return x;
        }
    }
}
