//! 端到端验证：索引后 `search_symbol detail=full` 能拿到**非空**的
//! `signature` 与 `doc_comment`（spec 2026-09-25「问题 1」验收标准 1）。
//!
//! 7 个语言各自的 fixture 来自 `crates/parser/tests/fixtures/doc/`，
//! 每个语言单独建索引、单独断言 —— 7 种注释形态差异很大，不做「改完一起测」。
//!
//! 索引落在 `target/e2e-signature-doc/`（项目内，属 cargo 构建产物目录）。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use codeconnect_index::full_indexer::FullIndexer;
use codeconnect_index::query_engine::QueryEngine;
use codeconnect_index::sled_store::SledStore;
use codeconnect_index::tantivy_index::{CallEdgeIndex, TantivyIndex};
use codeconnect_mcp::schemas::SearchSymbolParams;
use codeconnect_mcp::tools::{handle_search_symbol, ToolRegistry};
use codeconnect_parser::factory::ParserRegistry;

/// 一个语言的验证用例
struct Case {
    language: &'static str,
    fixture: &'static str,
    target: &'static str,
    /// 「带参数函数 + 文档注释」的符号名
    documented: &'static str,
    expect_signature: &'static str,
    expect_doc: &'static str,
    /// 注释与声明之间隔了空行的符号
    gap: &'static str,
    /// 注释与声明之间隔了别的声明的符号
    neighbor: &'static str,
}

const CASES: &[Case] = &[
    Case {
        language: "rust",
        fixture: "sample.rs",
        target: "sample.rs",
        documented: "add",
        expect_signature: "pub fn add(a: i32, b: i32) -> i32",
        expect_doc: "计算两数之和。 第二行说明。 # 示例",
        gap: "gap",
        neighbor: "neighbor",
    },
    Case {
        language: "java",
        fixture: "Sample.java",
        target: "Sample.java",
        documented: "add",
        expect_signature: "public int add(int a, int b)",
        expect_doc: "两数之和。 @param a 第一个数 @param b 第二个数 @return 和",
        gap: "gap",
        neighbor: "neighbor",
    },
    Case {
        language: "typescript",
        fixture: "sample.ts",
        target: "sample.ts",
        documented: "topAdd",
        expect_signature: "function topAdd(a: number, b: number): number",
        expect_doc: "顶层函数文档",
        gap: "gap",
        neighbor: "neighbor",
    },
    Case {
        language: "javascript",
        fixture: "sample.js",
        target: "sample.js",
        documented: "topAdd",
        expect_signature: "function topAdd(a, b)",
        expect_doc: "顶层函数文档",
        gap: "gap",
        neighbor: "neighbor",
    },
    Case {
        language: "c",
        fixture: "sample.c",
        target: "sample.c",
        documented: "add",
        expect_signature: "int add(int a, int b)",
        expect_doc: "两数之和。 @param a 第一个数 @param b 第二个数",
        gap: "gap",
        neighbor: "neighbor",
    },
    Case {
        language: "cpp",
        fixture: "sample.cpp",
        target: "sample.cpp",
        documented: "add",
        expect_signature: "int add(int a, int b)",
        expect_doc: "自由函数文档",
        gap: "gap",
        neighbor: "neighbor",
    },
    Case {
        language: "csharp",
        fixture: "Sample.cs",
        target: "Sample.cs",
        documented: "Add",
        expect_signature: "public int Add(int a, int b)",
        expect_doc: "<summary> 两数之和。 </summary>",
        gap: "Gap",
        neighbor: "Neighbor",
    },
];

fn workspace_target() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("e2e-signature-doc")
}

fn parser_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("parser")
        .join("tests")
        .join("fixtures")
        .join("doc")
}

/// 注册全部 7 个语言的解析器
fn registry_with_all_languages() -> Arc<ParserRegistry> {
    let mut registry = ParserRegistry::new();
    registry.register(Arc::new(codeconnect_parser::rust::RustParser::new()));
    registry.register(Arc::new(codeconnect_parser::java::JavaParser::new()));
    registry.register(Arc::new(
        codeconnect_parser::typescript::TypeScriptParser::new(),
    ));
    registry.register(Arc::new(
        codeconnect_parser::javascript::JavaScriptParser::new(),
    ));
    registry.register(Arc::new(codeconnect_parser::c::CParser::new()));
    registry.register(Arc::new(codeconnect_parser::cpp::CppParser::new()));
    registry.register(Arc::new(codeconnect_parser::csharp::CSharpParser::new()));
    Arc::new(registry)
}

/// 两个测试都会建索引；并发跑会争抢同一个 scratch 目录，串行化
static INDEX_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 索引一个只含单个 fixture 文件的临时项目，返回可查询的注册表
fn index_case(case: &Case) -> ToolRegistry {
    let _guard = INDEX_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let scratch = workspace_target().join(case.language);
    let project = scratch.join("project");
    let data = scratch.join("data");
    // 清掉上次运行的残留，保证索引是新建的
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&project).expect("创建临时项目目录失败");

    let src = parser_fixture_dir().join(case.fixture);
    let dst = project.join(case.target);
    std::fs::copy(&src, &dst)
        .unwrap_or_else(|e| panic!("复制 fixture {} 失败: {}", src.display(), e));

    let tantivy = Arc::new(
        TantivyIndex::open_or_create(&data.join("tantivy")).expect("创建符号索引失败"),
    );
    let edges = Arc::new(
        CallEdgeIndex::open_or_create(&data.join("edges")).expect("创建调用边索引失败"),
    );
    let sled = Arc::new(SledStore::open(&data.join("sled")).expect("创建 sled 失败"));

    let indexer = FullIndexer::new(
        &project,
        tantivy.clone(),
        edges,
        sled.clone(),
        registry_with_all_languages(),
    );
    let stats = indexer.run().expect("全量索引失败");
    assert!(
        stats.files_parsed >= 1 && stats.symbols_found >= 1,
        "{} 应至少索引 1 个文件且产出符号，实得 files_parsed={} symbols_found={}",
        case.language,
        stats.files_parsed,
        stats.symbols_found
    );

    let mut registry = ToolRegistry::new();
    registry.query_engine = Some(Arc::new(QueryEngine::from_arc(tantivy.clone(), sled.clone())));
    registry.tantivy = Some(tantivy);
    registry.sled = Some(sled);
    registry.project_root = Some(project);
    registry
}

/// 走 MCP 的 `search_symbol`，`detail=full`
fn search_full(registry: &ToolRegistry, language: &str, query: &str) -> serde_json::Value {
    handle_search_symbol(
        registry,
        SearchSymbolParams {
            query: query.to_string(),
            kind: None,
            language: Some(language.to_string()),
            limit: 20,
            detail: "full".to_string(),
            // 文本真值需要 sled 里的已索引文件表，与本次断言无关，关掉减少噪音
            text_truth: false,
        },
    )
    .data
    .unwrap_or_else(|| panic!("{} 搜索 {} 的响应没有 data", language, query))
}

fn find_symbol(data: &serde_json::Value, name: &str) -> serde_json::Value {
    data.as_array()
        .unwrap_or_else(|| panic!("响应应为数组，实得: {}", data))
        .iter()
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("未找到符号 {}，实得: {}", name, data))
        .clone()
}

/// 验收标准 1/2/4：7 个语言索引后 `detail=full` 都拿到非空且单行的 signature / doc_comment
#[test]
fn test_search_symbol_full_returns_signature_and_doc_for_all_languages() {
    let mut report = String::new();
    for case in CASES {
        let registry = index_case(case);
        let sym = find_symbol(
            &search_full(&registry, case.language, case.documented),
            case.documented,
        );

        let signature = sym["signature"].as_str().unwrap_or("");
        let doc = sym["doc_comment"].as_str().unwrap_or("");
        report.push_str(&format!(
            "\n[{}] {} -> signature={:?} doc_comment={:?}",
            case.language, case.documented, signature, doc
        ));

        assert!(
            !signature.is_empty(),
            "{} 的 signature 不应为空（detail=full 未透传？）",
            case.language
        );
        assert!(
            !doc.is_empty(),
            "{} 的 doc_comment 不应为空（detail=full 未透传？）",
            case.language
        );
        assert!(
            !signature.contains('\n') && !signature.contains('{'),
            "{} 的签名应单行且不含函数体，实得: {:?}",
            case.language,
            signature
        );
        assert_eq!(signature, case.expect_signature, "{} 签名不符", case.language);
        assert_eq!(doc, case.expect_doc, "{} 文档注释不符", case.language);
    }
    println!("{}", report);
}

/// 验收标准 3：注释与声明之间隔空行 / 隔别的声明时，索引里的 `doc_comment` 必须是空
#[test]
fn test_search_symbol_full_has_no_doc_for_gap_and_neighbor() {
    let mut report = String::new();
    for case in CASES {
        let registry = index_case(case);

        for (name, why) in [
            (case.gap, "注释与声明之间隔了空行"),
            (case.neighbor, "注释与声明之间隔了别的声明"),
        ] {
            let sym = find_symbol(&search_full(&registry, case.language, name), name);
            let doc = sym["doc_comment"].as_str();
            report.push_str(&format!(
                "\n[{}] {:<9} ({}) -> doc_comment={:?}",
                case.language, name, why, doc
            ));
            assert!(
                doc.is_none_or(|d| d.is_empty()),
                "{} 的 {} {}，doc_comment 必须为空，实得: {:?}",
                case.language,
                name,
                why,
                doc
            );
        }
    }
    println!("{}", report);
}
