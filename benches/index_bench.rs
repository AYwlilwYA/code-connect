use criterion::{black_box, Criterion, criterion_group, criterion_main};

fn bench_index_small_project(c: &mut Criterion) {
    c.bench_function("index rust sample", |b| {
        b.iter(|| {
            // TODO: 实际索引性能基准
            black_box(());
        })
    });
}

criterion_group!(benches, bench_index_small_project);
criterion_main!(benches);
