//! Criterion benchmarks for the indexer module (tokenize, chunking, retrieval).
//! Run with: `cargo bench`

use criterion::{black_box, criterion_group, criterion_main, Criterion};

// The indexer functions are pub(crate), so we can't import them directly.
// These benchmarks test the crate binary via the CLI.
// For micro-benchmarks, inline the core logic here.

fn bench_tokenize(c: &mut Criterion) {
    let text = (0..1000)
        .map(|i| format!("word_{} fn_login_authenticate_validate_token_{}", i, i))
        .collect::<Vec<_>>()
        .join(" ");

    c.bench_function("tokenize_1000_words", |b| {
        b.iter(|| {
            let lower = black_box(&text).to_lowercase();
            let _tokens: Vec<&str> = lower
                .split(|ch: char| !ch.is_alphanumeric())
                .filter(|s| !s.is_empty() && s.len() >= 2)
                .collect();
        })
    });
}

fn bench_split_content(c: &mut Criterion) {
    let text = (0..5000)
        .map(|i| format!("line_{}: some content here", i))
        .collect::<Vec<_>>()
        .join("\n");

    c.bench_function("split_5000_lines", |b| {
        b.iter(|| {
            let lines: Vec<&str> = black_box(&text).split('\n').collect();
            let _chunks: Vec<String> = lines.chunks(200).map(|chunk| chunk.join("\n")).collect();
        })
    });
}

criterion_group!(benches, bench_tokenize, bench_split_content);
criterion_main!(benches);
