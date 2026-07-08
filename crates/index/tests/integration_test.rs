//! 全量索引 + 搜索端到端集成测试
//!
//! 测试完整流程：解析项目目录 → 构建符号索引 →
//! 写入 tantivy + sled 持久化存储 → 全文搜索验证
//!
//! 覆盖 Rust、TypeScript、Java 三种语言的索引和查询。

use std::path::PathBuf;
use std::sync::Arc;

use codeconnect_index::full_indexer::FullIndexer;
use codeconnect_index::query_engine::QueryEngine;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};
use codeconnect_parser::factory::ParserRegistry;
use codeconnect_parser::java::JavaParser;
use codeconnect_parser::rust::RustParser;
use codeconnect_parser::typescript::TypeScriptParser;

/// 获取测试 fixtures 目录的绝对路径
fn fixtures_dir() -> PathBuf {
    // 从 crate 根目录向上找到项目根目录
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/index → 项目根目录
    let project_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("无法找到项目根目录");
    project_root.join("tests").join("fixtures").join("rust_sample")
}

#[test]
fn test_full_index_and_search_rust_sample() {
    // =====================================================================
    // 第 1 步：创建临时目录存放 tantivy/sled 存储
    // =====================================================================
    let tmp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let tantivy_dir = tmp_dir.path().join("tantivy");
    let tantivy_edges_dir = tmp_dir.path().join("tantivy_edges");
    let sled_dir = tmp_dir.path().join("sled");

    // =====================================================================
    // 第 2 步：注册 RustParser → 创建 FullIndexer
    // =====================================================================
    let mut registry = ParserRegistry::new();
    registry.register(Arc::new(RustParser::new()));

    let tantivy = Arc::new(TantivyIndex::open_or_create(&tantivy_dir)
        .expect("创建 tantivy 索引失败"));
    let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(&tantivy_edges_dir)
        .expect("创建调用边索引失败"));
    let sled = Arc::new(SledStore::open(&sled_dir)
        .expect("打开 sled 存储失败"));

    let project_root = fixtures_dir();
    let indexer = FullIndexer::new(
        &project_root,
        tantivy,
        call_edge_index,
        sled,
        Arc::new(registry),
    );

    // =====================================================================
    // 第 3 步：运行全量索引
    // =====================================================================
    let stats = indexer.run().expect("全量索引运行失败");

    // 验证索引统计
    assert!(
        stats.files_scanned > 0,
        "应扫描到至少一个源文件"
    );
    assert!(
        stats.symbols_found >= 5,
        "应提取到至少 5 个符号，实际: {}",
        stats.symbols_found
    );
    assert!(
        stats.failed_files.is_empty(),
        "不应有解析失败的文件: {:?}",
        stats.failed_files
    );

    // 复制需要的统计值，然后显式丢弃 indexer（释放 tangivy 写入锁）
    let symbols_found = stats.symbols_found;
    drop(indexer);

    // =====================================================================
    // 第 4 步：创建查询引擎，搜索 "authenticate"
    // =====================================================================
    // 重新打开 tantivy 和 sled（indexer 已释放写入锁）
    let tantivy_for_query = TantivyIndex::open_or_create(&tantivy_dir)
        .expect("重新打开 tantivy 索引失败");
    let sled_for_query = SledStore::open(&sled_dir)
        .expect("重新打开 sled 存储失败");

    let query_engine = QueryEngine::new(tantivy_for_query, sled_for_query);

    // 搜索 "authenticate" —— 应该在 fixtures 中找到 AuthService::authenticate
    let search_results = query_engine
        .search_by_name("authenticate", None, None, 10)
        .expect("搜索 authenticate 失败");

    assert!(
        !search_results.is_empty(),
        "搜索 'authenticate' 应返回至少一个结果"
    );

    // 验证返回结果中有 authenticate 方法
    let auth_result = search_results
        .iter()
        .find(|r| r.name.contains("authenticate"));
    assert!(
        auth_result.is_some(),
        "搜索结果中应包含 'authenticate' 符号, 实际结果: {:?}",
        search_results.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    // =====================================================================
    // 第 5 步：通过 tantivy 读取符号验证
    // =====================================================================
    for result in &search_results {
        let symbol_opt = query_engine
            .get_symbol_by_id(&result.stable_id)
            .expect("从 tantivy 读取符号失败");

        assert!(
            symbol_opt.is_some(),
            "tantivy 中应存在符号 {}, ID: {}",
            result.name,
            result.stable_id
        );

        let symbol = symbol_opt.unwrap();
        assert_eq!(symbol.id, result.stable_id, "Symbol ID 应一致");
        assert_eq!(symbol.name, result.name, "符号名应一致");
    }

    // =====================================================================
    // 第 6 步：验证索引符号总数
    // =====================================================================
    let total = query_engine
        .total_symbols()
        .expect("查询符号总数失败");

    assert_eq!(
        total, symbols_found,
        "tantivy 中的文档数应与索引统计一致"
    );
}

#[test]
fn test_index_stats_has_all_fields() {
    // 验证 IndexStats 输出包含所有必要字段
    let project_root = fixtures_dir();
    let tmp_dir = tempfile::tempdir().expect("创建临时目录失败");

    let mut registry = ParserRegistry::new();
    registry.register(Arc::new(RustParser::new()));

    let tantivy = Arc::new(TantivyIndex::open_or_create(&tmp_dir.path().join("tantivy"))
        .expect("创建 tantivy 索引失败"));
    let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(&tmp_dir.path().join("tantivy_edges"))
        .expect("创建调用边索引失败"));
    let sled = Arc::new(SledStore::open(&tmp_dir.path().join("sled"))
        .expect("打开 sled 存储失败"));

    let indexer = FullIndexer::new(
        &project_root,
        tantivy,
        call_edge_index,
        sled,
        Arc::new(registry),
    );

    let stats = indexer.run().expect("全量索引运行失败");

    // 所有统计字段应为非负值
    assert!(stats.files_scanned > 0);
    assert!(stats.files_parsed > 0);
    assert!(stats.symbols_found > 0);
    // calls_found 和 imports_found 依赖于 scm queries 的具体匹配情况
    // 这两个字段允许为 0（queries 可能未匹配到调用/导入）
    assert!(stats.failed_files.is_empty());
}

// ============================================================================
// TypeScript 项目集成测试
// ============================================================================

/// 获取 TypeScript fixture 目录
fn ts_fixtures_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("无法找到项目根目录");
    project_root.join("tests").join("fixtures").join("ts_sample")
}

#[test]
fn test_full_index_and_search_typescript_sample() {
    let tmp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let tantivy_dir = tmp_dir.path().join("tantivy_ts");
    let tantivy_edges_dir = tmp_dir.path().join("tantivy_edges_ts");
    let sled_dir = tmp_dir.path().join("sled_ts");

    // 注册 TypeScriptParser → 创建 FullIndexer
    let mut registry = ParserRegistry::new();
    registry.register(Arc::new(TypeScriptParser::new()));

    let tantivy = Arc::new(TantivyIndex::open_or_create(&tantivy_dir)
        .expect("创建 tantivy 索引失败"));
    let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(&tantivy_edges_dir)
        .expect("创建调用边索引失败"));
    let sled = Arc::new(SledStore::open(&sled_dir)
        .expect("打开 sled 存储失败"));

    let project_root = ts_fixtures_dir();
    let indexer = FullIndexer::new(
        &project_root,
        tantivy,
        call_edge_index,
        sled,
        Arc::new(registry),
    );

    let stats = indexer.run().expect("TypeScript 全量索引运行失败");

    // 验证索引统计
    assert!(
        stats.files_scanned >= 2,
        "应至少扫描到 2 个 TS 文件，实际: {}",
        stats.files_scanned
    );
    assert!(
        stats.symbols_found >= 3,
        "应提取到至少 3 个符号，实际: {}",
        stats.symbols_found
    );
    assert!(
        stats.failed_files.is_empty(),
        "不应有解析失败的文件: {:?}",
        stats.failed_files
    );

    let symbols_found = stats.symbols_found;
    drop(indexer);

    // 查询验证
    let tantivy_for_query = TantivyIndex::open_or_create(&tantivy_dir)
        .expect("重新打开 tantivy 索引失败");
    let sled_for_query = SledStore::open(&sled_dir)
        .expect("重新打开 sled 存储失败");
    let query_engine = QueryEngine::new(tantivy_for_query, sled_for_query);

    // 搜索 "User" —— 应该在 fixtures 中找到 User 类
    let search_results = query_engine
        .search_by_name("User", None, None, 10)
        .expect("搜索 User 失败");
    assert!(
        !search_results.is_empty(),
        "搜索 'User' 应返回至少一个结果"
    );

    // 搜索 "authenticate" —— 方法
    let auth_results = query_engine
        .search_by_name("authenticate", None, None, 10)
        .expect("搜索 authenticate 失败");
    assert!(
        !auth_results.is_empty(),
        "搜索 'authenticate' 应返回至少一个结果"
    );

    // 验证索引符号总数
    let total = query_engine.total_symbols().expect("查询符号总数失败");
    assert_eq!(total, symbols_found, "tantivy 文档数应与统计一致");
}

#[test]
fn test_typescript_finds_class_and_interface() {
    let tmp_dir = tempfile::tempdir().expect("创建临时目录失败");

    let mut registry = ParserRegistry::new();
    registry.register(Arc::new(TypeScriptParser::new()));

    let tantivy = Arc::new(TantivyIndex::open_or_create(&tmp_dir.path().join("tantivy_ts2"))
        .expect("创建索引失败"));
    let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(&tmp_dir.path().join("tantivy_edges_ts2"))
        .expect("创建调用边索引失败"));
    let sled = Arc::new(SledStore::open(&tmp_dir.path().join("sled_ts2"))
        .expect("打开存储失败"));

    let indexer = FullIndexer::new(
        &ts_fixtures_dir(),
        tantivy,
        call_edge_index,
        sled,
        Arc::new(registry),
    );

    let stats = indexer.run().expect("索引应成功");
    assert!(stats.symbols_found > 0, "应有符号");
    drop(indexer);

    // 重新打开查询
    let tantivy = TantivyIndex::open_or_create(&tmp_dir.path().join("tantivy_ts2"))
        .expect("重新打开索引失败");
    let sled = SledStore::open(&tmp_dir.path().join("sled_ts2"))
        .expect("重新打开存储失败");
    let query_engine = QueryEngine::new(tantivy, sled);

    let class_results = query_engine
        .search_by_name("User", None, None, 10)
        .expect("搜索失败");
    assert!(!class_results.is_empty(), "应找到 User 类");

    let iface_results = query_engine
        .search_by_name("AuthService", None, None, 10)
        .expect("搜索失败");
    assert!(!iface_results.is_empty(), "应找到 AuthService 接口");
}

// ============================================================================
// Java 项目集成测试
// ============================================================================

/// 获取 Java fixture 目录
fn java_fixtures_dir() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("无法找到项目根目录");
    project_root.join("tests").join("fixtures").join("java_sample")
}

#[test]
fn test_full_index_and_search_java_sample() {
    let tmp_dir = tempfile::tempdir().expect("创建临时目录失败");
    let tantivy_dir = tmp_dir.path().join("tantivy_java");
    let tantivy_edges_dir = tmp_dir.path().join("tantivy_edges_java");
    let sled_dir = tmp_dir.path().join("sled_java");

    // 注册 JavaParser → 创建 FullIndexer
    let mut registry = ParserRegistry::new();
    registry.register(Arc::new(JavaParser::new()));

    let tantivy = Arc::new(TantivyIndex::open_or_create(&tantivy_dir)
        .expect("创建 tantivy 索引失败"));
    let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(&tantivy_edges_dir)
        .expect("创建调用边索引失败"));
    let sled = Arc::new(SledStore::open(&sled_dir)
        .expect("打开 sled 存储失败"));

    let project_root = java_fixtures_dir();
    let indexer = FullIndexer::new(
        &project_root,
        tantivy,
        call_edge_index,
        sled,
        Arc::new(registry),
    );

    let stats = indexer.run().expect("Java 全量索引运行失败");

    // 验证索引统计
    assert!(
        stats.files_scanned >= 3,
        "应至少扫描到 3 个 Java 文件，实际: {}",
        stats.files_scanned
    );
    assert!(
        stats.symbols_found >= 3,
        "应提取到至少 3 个符号，实际: {}",
        stats.symbols_found
    );
    assert!(
        stats.failed_files.is_empty(),
        "不应有解析失败的文件: {:?}",
        stats.failed_files
    );

    let symbols_found = stats.symbols_found;
    drop(indexer);

    // 查询验证
    let tantivy_for_query = TantivyIndex::open_or_create(&tantivy_dir)
        .expect("重新打开 tantivy 索引失败");
    let sled_for_query = SledStore::open(&sled_dir)
        .expect("重新打开 sled 存储失败");
    let query_engine = QueryEngine::new(tantivy_for_query, sled_for_query);

    // 搜索 "User" —— Java 类
    let search_results = query_engine
        .search_by_name("User", None, None, 10)
        .expect("搜索 User 失败");
    assert!(
        !search_results.is_empty(),
        "搜索 'User' 应返回至少一个结果"
    );

    // 搜索 "authenticate" —— Java 方法
    let auth_results = query_engine
        .search_by_name("authenticate", None, None, 10)
        .expect("搜索 authenticate 失败");
    assert!(
        !auth_results.is_empty(),
        "搜索 'authenticate' 应返回至少一个结果"
    );

    // 验证索引符号总数
    let total = query_engine.total_symbols().expect("查询符号总数失败");
    assert_eq!(total, symbols_found, "tantivy 文档数应与统计一致");
}

#[test]
fn test_java_finds_class_and_interface() {
    let tmp_dir = tempfile::tempdir().expect("创建临时目录失败");

    let mut registry = ParserRegistry::new();
    registry.register(Arc::new(JavaParser::new()));

    let tantivy = Arc::new(TantivyIndex::open_or_create(&tmp_dir.path().join("tantivy_java2"))
        .expect("创建索引失败"));
    let call_edge_index = Arc::new(CallEdgeIndex::open_or_create(&tmp_dir.path().join("tantivy_edges_java2"))
        .expect("创建调用边索引失败"));
    let sled = Arc::new(SledStore::open(&tmp_dir.path().join("sled_java2"))
        .expect("打开存储失败"));

    let indexer = FullIndexer::new(
        &java_fixtures_dir(),
        tantivy,
        call_edge_index,
        sled,
        Arc::new(registry),
    );

    let stats = indexer.run().expect("索引应成功");
    assert!(stats.symbols_found > 0, "应有符号");
    drop(indexer);

    // 重新打开查询
    let tantivy = TantivyIndex::open_or_create(&tmp_dir.path().join("tantivy_java2"))
        .expect("重新打开索引失败");
    let sled = SledStore::open(&tmp_dir.path().join("sled_java2"))
        .expect("重新打开存储失败");
    let query_engine = QueryEngine::new(tantivy, sled);

    let class_results = query_engine
        .search_by_name("User", None, None, 10)
        .expect("搜索失败");
    assert!(!class_results.is_empty(), "应找到 User 类");

    let iface_results = query_engine
        .search_by_name("AuthService", None, None, 10)
        .expect("搜索失败");
    assert!(!iface_results.is_empty(), "应找到 AuthService 接口");
}
