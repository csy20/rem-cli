//! End-to-end benchmarks for the `rem` CLI.
//! Runs the actual compiled binary to measure real-world performance.
//! Uses Criterion for statistical analysis.
//!
//! Library-code microbenchmarks (BM25, tokenization, session serialization)
//! are written as `#[bench]` tests inside the relevant source modules.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use criterion::{black_box, criterion_group, criterion_main, Criterion};

/// Path to the compiled `rem` binary.
fn rem_binary() -> &'static str {
    env!("CARGO_BIN_EXE_rem")
}

fn unique_dir(prefix: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("rem-bench-{prefix}-{pid}-{ts}-{n}"))
}

fn bench_help(c: &mut Criterion) {
    c.bench_function("cli_help", |b| {
        b.iter(|| {
            let output = Command::new(rem_binary())
                .arg("--help")
                .output()
                .expect("failed to run rem --help");
            black_box(output.status.success());
        });
    });
}

fn bench_new_bare(c: &mut Criterion) {
    c.bench_function("new_bare", |b| {
        b.iter_with_setup(
            || unique_dir("new-bare"),
            |root| {
                let output = Command::new(rem_binary())
                    .args(["new", "./test-bare", "--project-type", "bare"])
                    .current_dir(&root)
                    .output()
                    .expect("failed to run rem new");
                black_box(output.status.success());
                let _ = std::fs::remove_dir_all(&root);
            },
        );
    });
}

fn bench_new_rust(c: &mut Criterion) {
    c.bench_function("new_rust", |b| {
        b.iter_with_setup(
            || unique_dir("new-rust"),
            |root| {
                let output = Command::new(rem_binary())
                    .args(["new", "./test-rust", "--project-type", "rust"])
                    .current_dir(&root)
                    .output()
                    .expect("failed to run rem new");
                black_box(output.status.success());
                let _ = std::fs::remove_dir_all(&root);
            },
        );
    });
}

fn bench_index_dry_run(c: &mut Criterion) {
    c.bench_function("index_dry_run", |b| {
        b.iter_with_setup(
            || {
                let root = unique_dir("index");
                std::fs::create_dir_all(root.join("src")).unwrap();
                std::fs::write(root.join("src/main.rs"), "fn main() { println!(\"hello\"); }\n").unwrap();
                std::fs::write(root.join("src/lib.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }\n").unwrap();
                std::fs::write(root.join("README.md"), "# Test Project\n").unwrap();
                root
            },
            |root| {
                let output = Command::new(rem_binary())
                    .args(["index", "--dry-run", root.to_str().unwrap()])
                    .output()
                    .expect("failed to run rem index --dry-run");
                black_box(output.status.success());
                let _ = std::fs::remove_dir_all(&root);
            },
        );
    });
}

fn bench_theme_list(c: &mut Criterion) {
    c.bench_function("theme_list", |b| {
        b.iter(|| {
            let output = Command::new(rem_binary())
                .arg("theme")
                .output()
                .expect("failed to run rem theme");
            black_box(output.status.success());
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(50).measurement_time(std::time::Duration::from_secs(10));
    targets = bench_help, bench_new_bare, bench_new_rust, bench_index_dry_run, bench_theme_list
}

criterion_main!(benches);
