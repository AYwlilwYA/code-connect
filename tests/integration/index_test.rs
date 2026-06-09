//! 全量索引集成测试
//!
//! 测试完整流程：解析目录 → 构建符号 → 写入 tantivy + sled → 查询验证

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use codeconnect_index::full_indexer::FullIndexer;
    use codeconnect_index::sled_store::SledStore;
    use codeconnect_index::tantivy_index::TantivyIndex;
    use codeconnect_parser::factory::ParserRegistry;
    use codeconnect_parser::java::JavaParser;
    use codeconnect_parser::rust::RustParser;
    use codeconnect_parser::typescript::TypeScriptParser;

    /// 获取项目根目录（从 crates/index/ 向上两级到 workspace 根）
    fn project_root() -> PathBuf {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // manifest_dir = F:\...\code-connect\crates\index
        manifest_dir
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// 创建临时目录用于索引存储
    fn temp_index_dir(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().expect("创建临时目录失败");
        let tantivy_dir = tmp.path().join(format!("{}_tantivy", name));
        let sled_dir = tmp.path().join(format!("{}_sled", name));
        (tmp, tantivy_dir, sled_dir)
    }

    /// 创建注册了 Rust 解析器的注册表
    fn rust_registry() -> Arc<ParserRegistry> {
        let mut registry = ParserRegistry::new();
        registry.register(Arc::new(RustParser::new()));
        Arc::new(registry)
    }

    /// 创建注册了 TypeScript 解析器的注册表
    fn typescript_registry() -> Arc<ParserRegistry> {
        let mut registry = ParserRegistry::new();
        registry.register(Arc::new(TypeScriptParser::new()));
        Arc::new(registry)
    }

    /// 创建注册了 Java 解析器的注册表
    fn java_registry() -> Arc<ParserRegistry> {
        let mut registry = ParserRegistry::new();
        registry.register(Arc::new(JavaParser::new()));
        Arc::new(registry)
    }

    // ========================================================================
    // Rust 项目索引测试
    // ========================================================================

    #[test]
    fn test_index_rust_project() {
        let root = project_root();
        let fixture_dir = root.join("tests").join("fixtures").join("rust_sample");
        assert!(
            fixture_dir.exists(),
            "rust_sample fixture 目录应存在: {}",
            fixture_dir.display()
        );

        let (_tmp, tantivy_dir, sled_dir) = temp_index_dir("rust");
        let tantivy = TantivyIndex::open_or_create(&tantivy_dir).expect("创建索引失败");
        let sled = SledStore::open(&sled_dir).expect("创建存储失败");

        let registry = rust_registry();
        let mut indexer = FullIndexer::new(&fixture_dir, tantivy, sled, registry);

        let stats = indexer.run().expect("Rust 全量索引应成功");

        println!("Rust 索引统计: {:?}", stats);
        assert!(stats.files_scanned > 0, "应至少扫描到 1 个文件");
        assert!(stats.files_parsed > 0, "应至少成功解析 1 个文件");
        assert!(stats.symbols_found > 0, "应至少提取到 1 个符号");
        assert!(stats.failed_files.is_empty(), "不应有解析失败的文件");

        // 验证 tantivy 中有数据
        let doc_count = indexer.tantivy.doc_count().expect("查询文档数失败");
        assert!(doc_count > 0, "tantivy 索引中应有文档");

        // 验证搜索能返回结果
        let results = indexer
            .tantivy
            .search_by_name("User", 10)
            .expect("搜索 User 失败");
        assert!(!results.is_empty(), "搜索 User 应返回结果");

        let results = indexer
            .tantivy
            .search_by_name("authenticate", 10)
            .expect("搜索 authenticate 失败");
        assert!(!results.is_empty(), "搜索 authenticate 应返回结果");
    }

    #[test]
    fn test_index_rust_project_stats() {
        let root = project_root();
        let fixture_dir = root.join("tests").join("fixtures").join("rust_sample");

        let (_tmp, tantivy_dir, sled_dir) = temp_index_dir("rust_stats");
        let tantivy = TantivyIndex::open_or_create(&tantivy_dir).expect("创建索引失败");
        let sled = SledStore::open(&sled_dir).expect("创建存储失败");

        let registry = rust_registry();
        let mut indexer = FullIndexer::new(&fixture_dir, tantivy, sled, registry);

        let stats = indexer.run().expect("索引应成功");

        // 验证基本统计约束
        assert!(stats.symbols_found > 0, "应有符号");
        assert!(
            stats.files_parsed <= stats.files_scanned,
            "解析文件数不应超过扫描文件数"
        );
    }

    // ========================================================================
    // TypeScript 项目索引测试
    // ========================================================================

    #[test]
    fn test_index_typescript_project() {
        let root = project_root();
        let fixture_dir = root.join("tests").join("fixtures").join("ts_sample");
        assert!(
            fixture_dir.exists(),
            "ts_sample fixture 目录应存在: {}",
            fixture_dir.display()
        );

        let (_tmp, tantivy_dir, sled_dir) = temp_index_dir("ts");
        let tantivy = TantivyIndex::open_or_create(&tantivy_dir).expect("创建索引失败");
        let sled = SledStore::open(&sled_dir).expect("创建存储失败");

        let registry = typescript_registry();
        let mut indexer = FullIndexer::new(&fixture_dir, tantivy, sled, registry);

        let stats = indexer.run().expect("TypeScript 全量索引应成功");

        println!("TypeScript 索引统计: {:?}", stats);
        assert!(stats.files_scanned > 0, "应至少扫描到 1 个 TS 文件");
        assert!(stats.files_parsed > 0, "应至少成功解析 1 个 TS 文件");
        assert!(stats.symbols_found > 0, "应至少提取到 1 个符号");
        assert!(
            stats.failed_files.is_empty(),
            "不应有解析失败的文件: {:?}",
            stats.failed_files
        );

        // 验证搜索
        let results = indexer
            .tantivy
            .search_by_name("User", 10)
            .expect("搜索 User 失败");
        assert!(!results.is_empty(), "搜索 User 应返回结果");

        let results = indexer
            .tantivy
            .search_by_name("authenticate", 10)
            .expect("搜索 authenticate 失败");
        assert!(!results.is_empty(), "搜索 authenticate 应返回结果");
    }

    #[test]
    fn test_index_typescript_finds_class_and_interface() {
        let root = project_root();
        let fixture_dir = root.join("tests").join("fixtures").join("ts_sample");

        let (_tmp, tantivy_dir, sled_dir) = temp_index_dir("ts_class");
        let tantivy = TantivyIndex::open_or_create(&tantivy_dir).expect("创建索引失败");
        let sled = SledStore::open(&sled_dir).expect("创建存储失败");

        let registry = typescript_registry();
        let mut indexer = FullIndexer::new(&fixture_dir, tantivy, sled, registry);

        let stats = indexer.run().expect("索引应成功");
        assert!(stats.symbols_found > 0, "应有符号");

        // 搜索类
        let class_results = indexer
            .tantivy
            .search_by_name("User", 10)
            .expect("搜索 User 失败");
        assert!(!class_results.is_empty(), "应找到 User 类");

        // 搜索接口
        let iface_results = indexer
            .tantivy
            .search_by_name("AuthService", 10)
            .expect("搜索 AuthService 失败");
        assert!(!iface_results.is_empty(), "应找到 AuthService 接口");
    }

    // ========================================================================
    // Java 项目索引测试
    // ========================================================================

    #[test]
    fn test_index_java_project() {
        let root = project_root();
        let fixture_dir = root.join("tests").join("fixtures").join("java_sample");
        assert!(
            fixture_dir.exists(),
            "java_sample fixture 目录应存在: {}",
            fixture_dir.display()
        );

        let (_tmp, tantivy_dir, sled_dir) = temp_index_dir("java");
        let tantivy = TantivyIndex::open_or_create(&tantivy_dir).expect("创建索引失败");
        let sled = SledStore::open(&sled_dir).expect("创建存储失败");

        let registry = java_registry();
        let mut indexer = FullIndexer::new(&fixture_dir, tantivy, sled, registry);

        let stats = indexer.run().expect("Java 全量索引应成功");

        println!("Java 索引统计: {:?}", stats);
        assert!(stats.files_scanned > 0, "应至少扫描到 1 个 Java 文件");
        assert!(stats.files_parsed > 0, "应至少成功解析 1 个 Java 文件");
        assert!(stats.symbols_found > 0, "应至少提取到 1 个符号");
        assert!(
            stats.failed_files.is_empty(),
            "不应有解析失败的文件: {:?}",
            stats.failed_files
        );

        // 验证搜索
        let results = indexer
            .tantivy
            .search_by_name("User", 10)
            .expect("搜索 User 失败");
        assert!(!results.is_empty(), "搜索 User 应返回结果");

        let results = indexer
            .tantivy
            .search_by_name("authenticate", 10)
            .expect("搜索 authenticate 失败");
        assert!(!results.is_empty(), "搜索 authenticate 应返回结果");
    }

    #[test]
    fn test_index_java_finds_class_and_interface() {
        let root = project_root();
        let fixture_dir = root.join("tests").join("fixtures").join("java_sample");

        let (_tmp, tantivy_dir, sled_dir) = temp_index_dir("java_class");
        let tantivy = TantivyIndex::open_or_create(&tantivy_dir).expect("创建索引失败");
        let sled = SledStore::open(&sled_dir).expect("创建存储失败");

        let registry = java_registry();
        let mut indexer = FullIndexer::new(&fixture_dir, tantivy, sled, registry);

        let stats = indexer.run().expect("索引应成功");
        assert!(stats.symbols_found > 0, "应有符号");

        // 搜索类
        let class_results = indexer
            .tantivy
            .search_by_name("User", 10)
            .expect("搜索 User 失败");
        assert!(!class_results.is_empty(), "应找到 User 类");

        // 搜索接口
        let iface_results = indexer
            .tantivy
            .search_by_name("AuthService", 10)
            .expect("搜索 AuthService 失败");
        assert!(!iface_results.is_empty(), "应找到 AuthService 接口");
    }

    // ========================================================================
    // 空项目测试
    // ========================================================================

    #[test]
    fn test_index_empty_dir() {
        let (_tmp, tantivy_dir, sled_dir) = temp_index_dir("empty");
        let tantivy = TantivyIndex::open_or_create(&tantivy_dir).expect("创建索引失败");
        let sled = SledStore::open(&sled_dir).expect("创建存储失败");

        let registry = rust_registry();
        // 使用临时目录下的空子目录
        let empty_dir = _tmp.path().join("empty_src");
        std::fs::create_dir_all(&empty_dir).expect("创建空目录失败");

        let mut indexer = FullIndexer::new(&empty_dir, tantivy, sled, registry);
        let stats = indexer.run().expect("空目录索引应成功（不报错）");

        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.files_parsed, 0);
        assert_eq!(stats.symbols_found, 0);
    }
}
