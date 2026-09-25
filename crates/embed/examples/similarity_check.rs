//! 嵌入能力实测：确定性 + 判别性相似度
//!
//! 用法：
//!   cargo run -p codeconnect-embed --example similarity_check [模型目录]
//!
//! 判别性测试的要点：查询串与目标符号名**没有任何共同子串**，
//! 若相似度仍能区分「语义相近」与「语义无关」，说明走的是真语义而非词法。
//!
//! 关键词：相似度 判别性 语义 词法 余弦 确定性

use codeconnect_embed::{ModelSource, cosine_similarity, default_model_dir, load, load_source, models_root};

/// 一组判别性用例：查询 + [(候选文本, 是否语义相关)]
struct Case {
    label: &'static str,
    query: &'static str,
    candidates: &'static [(&'static str, bool)],
}

const CASES: &[Case] = &[
    Case {
        label: "中文自然语言 → 英文符号（无共同子串）",
        query: "计算两个时间点相差多少秒",
        candidates: &[
            ("elapsed_seconds", true),
            ("duration_between", true),
            ("parse_json", false),
            ("render_html_page", false),
        ],
    },
    Case {
        label: "英文自然语言 → 英文符号",
        query: "read a json file and parse it into an object",
        candidates: &[
            ("parse_json", true),
            ("load_config_from_disk", true),
            ("elapsed_seconds", false),
            ("trace_callers", false),
        ],
    },
    Case {
        label: "中文自然语言 → 带签名的符号",
        query: "把一段文本切分成词",
        candidates: &[
            ("fn tokenize(text: &str) -> Vec<String>", true),
            ("fn split_words(input: &str) -> Vec<Token>", true),
            ("fn elapsed_seconds(a: Instant, b: Instant) -> f64", false),
            ("fn render_html_page(config: &Config) -> String", false),
        ],
    },
];

/// 演示「模型可选 / 可指定」的三种配置形态，并打印各自的原始错误分类
fn config_probe(real_dir: &std::path::Path) {
    println!("=== 配置形态（模型可选、可指定） ===");
    let root = models_root();
    println!("默认根目录: {}", root.display());

    for (label, raw) in [
        ("未配置（None）", None),
        ("未配置（空串）", Some("")),
        ("按名指定（未安装的名字）", Some("not-installed-model")),
        ("按路径指定（真实目录）", Some(real_dir.to_str().unwrap())),
    ] {
        let src = ModelSource::from_config(raw);
        match load_source(&src, &root) {
            Ok(e) => println!(
                "  {label:<26} → is_configured={} kind=Ok 已加载(dim={})",
                src.is_configured(),
                e.dim()
            ),
            Err(e) => println!(
                "  {label:<26} → is_configured={} kind={:?}\n     {}",
                src.is_configured(),
                e.kind(),
                e.to_string().replace('\n', "\n     ")
            ),
        }
    }
    println!();
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_model_dir);

    config_probe(&dir);

    println!("模型目录: {}", dir.display());
    let embedder = match load(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("加载失败：{e}");
            std::process::exit(1);
        }
    };
    println!("模型名  : {}", embedder.model_name());
    println!("向量维度: {}", embedder.dim());
    println!();

    let mut total_relevant: Vec<f32> = Vec::new();
    let mut total_irrelevant: Vec<f32> = Vec::new();

    for case in CASES {
        let texts: Vec<String> = std::iter::once(case.query.to_string())
            .chain(case.candidates.iter().map(|(t, _)| (*t).to_string()))
            .collect();
        let vectors = embedder.embed(&texts).expect("嵌入失败");
        let q = &vectors[0];

        println!("=== {} ===", case.label);
        println!("查询: {}", case.query);
        for (i, (text, relevant)) in case.candidates.iter().enumerate() {
            let score = cosine_similarity(q, &vectors[i + 1]);
            println!(
                "  {:.4}  [{}] {}",
                score,
                if *relevant { "相关" } else { "无关" },
                text
            );
            if *relevant {
                total_relevant.push(score);
            } else {
                total_irrelevant.push(score);
            }
        }
        let max_rel = case
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, (_, r))| *r)
            .map(|(i, _)| cosine_similarity(q, &vectors[i + 1]))
            .fold(f32::MIN, f32::max);
        let max_irr = case
            .candidates
            .iter()
            .enumerate()
            .filter(|(_, (_, r))| !*r)
            .map(|(i, _)| cosine_similarity(q, &vectors[i + 1]))
            .fold(f32::MIN, f32::max);
        println!(
            "  -> 最低相关 {:.4} vs 最高无关 {:.4}，间隔 {:+.4}",
            max_rel,
            max_irr,
            max_rel - max_irr
        );
        println!();
    }

    // 确定性：同一文本两次嵌入必须逐位一致
    let probe = vec!["determinism probe 确定性探测".to_string()];
    let a = embedder.embed(&probe).unwrap();
    let b = embedder.embed(&probe).unwrap();
    let same = a[0].len() == b[0].len()
        && a[0]
            .iter()
            .zip(b[0].iter())
            .all(|(x, y)| x.to_bits() == y.to_bits());
    println!("确定性（同文本两次嵌入逐位一致）: {same}");
    println!(
        "  首向量前 5 维: {:?}",
        &a[0][..5.min(a[0].len())]
    );

    // 不同文本必须给出不同向量（排除「恒定输出」这种假象）
    let two = embedder
        .embed(&["parse json".to_string(), "render html".to_string()])
        .unwrap();
    println!(
        "不同文本的余弦（应明显 < 1）: {:.4}",
        cosine_similarity(&two[0], &two[1])
    );

    let avg = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    println!();
    println!(
        "总览：相关对均值 {:.4}（n={}） / 无关对均值 {:.4}（n={}）",
        avg(&total_relevant),
        total_relevant.len(),
        avg(&total_irrelevant),
        total_irrelevant.len()
    );
}
