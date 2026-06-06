// ── tests/find.rs ──
//
// Integration test for the `/find` slash command. Mirrors the
// `tests/intent_parsing.rs` style: pulls `find.rs` in via `#[path]`
// so we can exercise it without exposing the module's types as
// `pub` from the binary crate.

#[path = "../src/find.rs"]
mod find;

use find::{find_matches, FindOptions};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_tree(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let base = std::env::temp_dir().join(format!(
        "rem-find-int-{}-{}-{}",
        suffix,
        std::process::id(),
        nanos
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();

    fs::write(
        base.join("index.html"),
        "<!doctype html>\n<html>\n  <body>handleneedle body</body>\n</html>\n",
    )
    .unwrap();
    fs::write(
        base.join("style.css"),
        "body { color: red; }\n/* handleneedle in css */\n",
    )
    .unwrap();
    fs::write(
        base.join("script.js"),
        "function handleneedle() { return 1; }\n",
    )
    .unwrap();

    fs::create_dir_all(base.join("node_modules/lib")).unwrap();
    fs::write(base.join("node_modules/lib/x.js"), "handleneedle here\n").unwrap();
    fs::create_dir_all(base.join("target")).unwrap();
    fs::write(base.join("target/ignore.rs"), "handleneedle in target\n").unwrap();
    fs::create_dir_all(base.join(".git")).unwrap();
    fs::write(base.join(".git/HEAD"), "handleneedle in git\n").unwrap();

    base
}

fn rel(path: &std::path::Path, root: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/")
}

#[test]
fn integration_finds_source_files_only() {
    let root = temp_tree("source-only");
    let report = find_matches(&root, "handleneedle", &FindOptions::default());
    let rels: Vec<String> = report.matches.iter().map(|m| rel(&m.path, &root)).collect();

    assert!(rels.iter().any(|p| p == "index.html"));
    assert!(rels.iter().any(|p| p == "style.css"));
    assert!(rels.iter().any(|p| p == "script.js"));
    assert!(!rels.iter().any(|p| p.starts_with("node_modules/")));
    assert!(!rels.iter().any(|p| p.starts_with("target/")));
    assert!(!rels.iter().any(|p| p.starts_with(".git/")));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn integration_line_and_column_are_correct() {
    let root = temp_tree("line-col");
    let report = find_matches(&root, "color: red", &FindOptions::default());
    assert!(!report.matches.is_empty(), "should match the css rule");
    let m = &report.matches[0];
    assert!(m.line.contains("color: red"));
    assert_eq!(m.line_no, 1, "color: red is on line 1 of style.css");
    assert!(m.column >= 1);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn integration_max_results_truncates() {
    let root = temp_tree("cap");
    let mut opts = FindOptions::default();
    opts.max_results = 1;
    let report = find_matches(&root, "handleneedle", &opts);
    assert_eq!(report.matches.len(), 1);
    assert!(report.truncated);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn integration_empty_query_yields_nothing() {
    let root = temp_tree("empty");
    let report = find_matches(&root, "", &FindOptions::default());
    assert!(report.matches.is_empty());
    assert!(!report.truncated);
    assert_eq!(report.files_scanned, 0);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn integration_summary_counts_files() {
    let root = temp_tree("summary");
    let report = find_matches(&root, "handleneedle", &FindOptions::default());
    let unique: std::collections::BTreeSet<String> =
        report.matches.iter().map(|m| rel(&m.path, &root)).collect();
    // Should be 3 source files: index.html, style.css, script.js
    assert_eq!(unique.len(), 3);

    let _ = fs::remove_dir_all(&root);
}
