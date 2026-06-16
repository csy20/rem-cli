//! In-project text search (`/find` command).
//! Walks the project tree with `walkdir`, reads each file with a size cap,
//! and returns every line matching the query. Skips hidden/build/lock dirs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ignore::WalkBuilder;

const DEFAULT_MAX_FILE_BYTES: u64 = 64 * 1024;
const DEFAULT_MAX_RESULTS: usize = 500;
const DEFAULT_MAX_DEPTH: usize = 8;

/// A single hit in a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub path: PathBuf,
    pub line_no: usize,
    pub column: usize,
    pub line: String,
}

/// Knobs for `find_matches`. All fields have sensible defaults; pass
/// `FindOptions::default()` to use them.
#[derive(Debug, Clone)]
pub struct FindOptions {
    pub max_results: usize,
    pub max_file_bytes: u64,
    pub max_depth: usize,
    pub case_sensitive: bool,
    pub use_regex: bool,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            max_results: DEFAULT_MAX_RESULTS,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_depth: DEFAULT_MAX_DEPTH,
            case_sensitive: true,
            use_regex: false,
        }
    }
}

/// Result of a `/find` call — matches plus simple summary fields the
/// REPL can render (`handle_find` in `main.rs`).
#[derive(Debug, Clone)]
pub struct FindReport {
    pub matches: Vec<Match>,
    pub files_scanned: usize,
    pub files_skipped: usize,
    pub elapsed_ms: u128,
    pub truncated: bool,
}

/// Directory or file names that should never be descended into. These
/// are excluded at the `walkdir` iterator level so the search stays
/// fast and the output stays focused on source.
pub const SKIP_NAMES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".cache",
    ".rem",
    "__pycache__",
    ".venv",
    "venv",
];

/// File suffixes that are always skipped even outside the above dirs.
pub const SKIP_SUFFIXES: &[&str] = &[
    ".min.js", ".min.css", ".lock", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".pdf",
    ".zip", ".tar", ".gz", ".bz2", ".xz", ".7z", ".mp3", ".mp4", ".mov", ".woff", ".woff2", ".ttf",
    ".otf", ".eot",
];

/// Checks whether a directory name should be skipped during traversal.
pub fn should_skip_dir(name: &str) -> bool {
    SKIP_NAMES.contains(&name)
}

/// Checks whether a file name should be skipped (minified assets, binaries, etc.).
pub fn should_skip_file(name: &str) -> bool {
    let lower_bytes = name.as_bytes();
    let len = lower_bytes.len();
    // Fast path: check common minified extensions using byte comparison
    if len > 7 {
        let end = &lower_bytes[len.saturating_sub(7)..];
        if end.eq_ignore_ascii_case(b".min.js") || end.eq_ignore_ascii_case(b".min.css") {
            return true;
        }
    }
    SKIP_SUFFIXES.iter().any(|suf| {
        let sufb = suf.as_bytes();
        len >= sufb.len() && lower_bytes[len - sufb.len()..].eq_ignore_ascii_case(sufb)
    })
}

/// Walk `root` and return every line whose contents contain `query`.
///
/// By default `query` is matched as a plain substring — no regex metacharacters
/// are interpreted. Set `opts.use_regex = true` to enable regex matching.
/// An empty `query` is treated as "no matches" rather
/// than matching every line, which would be useless.
pub fn find_matches(root: &Path, query: &str, opts: &FindOptions) -> FindReport {
    let start = Instant::now();
    let mut report = FindReport {
        matches: Vec::new(),
        files_scanned: 0,
        files_skipped: 0,
        elapsed_ms: 0,
        truncated: false,
    };

    if query.is_empty() {
        report.elapsed_ms = start.elapsed().as_millis();
        return report;
    }

    let needle: Vec<u8> = if opts.case_sensitive {
        query.as_bytes().to_vec()
    } else {
        query.to_lowercase().into_bytes()
    };

    let regex_pattern: Option<regex::Regex> = if opts.use_regex {
        let re_query = if opts.case_sensitive {
            query.to_string()
        } else {
            format!("(?i){}", query)
        };
        regex::Regex::new(&re_query).ok()
    } else {
        None
    };

    let walker = WalkBuilder::new(root)
        .max_depth(Some(opts.max_depth))
        .follow_links(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(move |e| {
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = e.file_name().to_str() {
                    if should_skip_dir(name) {
                        return false;
                    }
                }
            }
            if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if let Some(name) = e.file_name().to_str() {
                    if should_skip_file(name) {
                        return false;
                    }
                }
            }
            true
        })
        .build();

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => {
                report.files_skipped += 1;
                continue;
            }
        };
        if should_skip_file(name) {
            report.files_skipped += 1;
            continue;
        }

        let size = match fs::metadata(path) {
            Ok(m) => m.len(),
            Err(_) => {
                report.files_skipped += 1;
                continue;
            }
        };
        if size == 0 || size > opts.max_file_bytes {
            report.files_skipped += 1;
            continue;
        }

        let contents = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => {
                report.files_skipped += 1;
                continue;
            }
        };
        report.files_scanned += 1;

        for (idx, raw_line) in contents.lines().enumerate() {
            let line_no = idx + 1;

            if let Some(ref re) = regex_pattern {
                if re.is_match(raw_line) {
                    for cap in re.find_iter(raw_line) {
                        let column = byte_to_column(&raw_line.as_bytes()[..cap.start()]);
                        report.matches.push(Match {
                            path: path.to_path_buf(),
                            line_no,
                            column: column + 1,
                            line: raw_line.to_string(),
                        });
                        if report.matches.len() >= opts.max_results {
                            report.truncated = true;
                            report.elapsed_ms = start.elapsed().as_millis();
                            return report;
                        }
                    }
                }
            } else {
                let mut search_from = 0usize;
                macro_rules! search_in {
                    ($haystack:expr) => {{
                        while let Some(pos) = find_subslice(&$haystack[search_from..], &needle) {
                            let column = byte_to_column(&$haystack[..search_from + pos]);
                            let line = raw_line.to_string();
                            report.matches.push(Match {
                                path: path.to_path_buf(),
                                line_no,
                                column: column + 1,
                                line,
                            });
                            if report.matches.len() >= opts.max_results {
                                report.truncated = true;
                                report.elapsed_ms = start.elapsed().as_millis();
                                return report;
                            }
                            search_from += pos + needle.len();
                            if search_from > $haystack.len() {
                                break;
                            }
                        }
                    }};
                }
                if opts.case_sensitive {
                    let haystack = raw_line.as_bytes();
                    search_in!(haystack);
                } else {
                    let lowered = raw_line.to_lowercase();
                    let haystack = lowered.as_bytes();
                    search_in!(haystack);
                }
            }
        }
    }

    report.elapsed_ms = start.elapsed().as_millis();
    report
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn byte_to_column(prefix: &[u8]) -> usize {
    prefix.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn make_tree() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "rem-find-test-{}-{}",
            std::process::id(),
            chrono_like_nanos()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        fs::write(
            base.join("index.html"),
            "<!doctype html>\n<html>\n  <body>handle_lint target</body>\n</html>\n",
        )
        .unwrap();
        fs::write(
            base.join("style.css"),
            "body { color: red; }\n/* handle_lint in css */\n",
        )
        .unwrap();
        fs::write(
            base.join("script.js"),
            "function handle_lint() { return 1; }\nconst color = 'red';\n",
        )
        .unwrap();
        fs::write(
            base.join("main.rs"),
            "fn handle_lint() {}\nfn other() { println!(\"hi\"); }\n",
        )
        .unwrap();
        fs::write(base.join("empty.txt"), "").unwrap();
        fs::write(base.join("binary.bin"), [0u8, 1, 2, 3, 0, 0]).unwrap();

        fs::create_dir_all(base.join("node_modules/lib")).unwrap();
        fs::write(base.join("node_modules/lib/x.js"), "handle_lint here too\n").unwrap();
        fs::create_dir_all(base.join("target")).unwrap();
        fs::write(base.join("target/ignore.rs"), "handle_lint in target\n").unwrap();
        fs::create_dir_all(base.join(".git")).unwrap();
        fs::write(base.join(".git/HEAD"), "handle_lint in git\n").unwrap();

        base
    }

    fn chrono_like_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    #[test]
    fn finds_substring_across_files() {
        let root = make_tree();
        let opts = FindOptions::default();
        let report = find_matches(&root, "handle_lint", &opts);
        let rels: Vec<String> = report
            .matches
            .iter()
            .map(|m| {
                m.path
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert!(rels.iter().any(|p| p == "index.html"));
        assert!(rels.iter().any(|p| p == "style.css"));
        assert!(rels.iter().any(|p| p == "script.js"));
        assert!(rels.iter().any(|p| p == "main.rs"));
        assert!(!rels.iter().any(|p| p.starts_with("node_modules/")));
        assert!(!rels.iter().any(|p| p.starts_with("target/")));
        assert!(!rels.iter().any(|p| p.starts_with(".git/")));
    }

    #[test]
    fn line_and_column_are_one_indexed() {
        let root = make_tree();
        let opts = FindOptions::default();
        let report = find_matches(&root, "color: red", &opts);
        assert!(!report.matches.is_empty());
        let m = &report.matches[0];
        assert!(m.line_no >= 1);
        assert!(m.column >= 1);
        assert!(m.line.contains("color: red"));
    }

    #[test]
    fn empty_query_returns_no_matches() {
        let root = make_tree();
        let report = find_matches(&root, "", &FindOptions::default());
        assert!(report.matches.is_empty());
        assert!(!report.truncated);
    }

    #[test]
    fn max_results_caps_and_marks_truncated() {
        let root = make_tree();
        let opts = FindOptions {
            max_results: 2,
            ..Default::default()
        };
        let report = find_matches(&root, "handle_lint", &opts);
        assert_eq!(report.matches.len(), 2);
        assert!(report.truncated);
    }

    #[test]
    fn case_insensitive_matches_variants() {
        let root = make_tree();
        let opts = FindOptions {
            case_sensitive: false,
            ..Default::default()
        };
        let report = find_matches(&root, "HANDLE_LINT", &opts);
        assert!(report.matches.iter().any(|m| m.path.ends_with("script.js")));
    }

    #[test]
    fn skips_empty_and_oversize_files() {
        let root = make_tree();
        let opts = FindOptions {
            max_file_bytes: 4,
            ..Default::default()
        };
        let report = find_matches(&root, "handle_lint", &opts);
        for m in &report.matches {
            let rel = m
                .path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            assert_ne!(rel, "empty.txt");
        }
    }

    #[test]
    fn no_match_yields_empty_report() {
        let root = make_tree();
        let report = find_matches(&root, "__definitely_not_present__", &FindOptions::default());
        assert!(report.matches.is_empty());
        assert!(!report.truncated);
    }

    #[test]
    fn max_depth_limits_recursion() {
        let deep_root = std::env::temp_dir().join(format!(
            "rem-find-deep-{}-{}",
            std::process::id(),
            chrono_like_nanos()
        ));
        let _ = fs::remove_dir_all(&deep_root);
        let deep = deep_root.join("a/b/c/d/e/f");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("leaf.txt"), "needle_in_deep_leaf\n").unwrap();

        let shallow = FindOptions {
            max_depth: 3,
            ..Default::default()
        };
        let report = find_matches(&deep_root, "needle_in_deep_leaf", &shallow);
        assert!(
            report.matches.is_empty(),
            "shallow depth should not reach deeply nested leaf"
        );

        let deep_opt = FindOptions {
            max_depth: 12,
            ..Default::default()
        };
        let report = find_matches(&deep_root, "needle_in_deep_leaf", &deep_opt);
        assert_eq!(report.matches.len(), 1);

        let _ = fs::remove_dir_all(&deep_root);
    }

    #[test]
    fn regex_mode_matches_pattern() {
        let root = make_tree();
        let opts = FindOptions {
            use_regex: true,
            ..Default::default()
        };
        let report = find_matches(&root, r"handle_\w+", &opts);
        assert!(report.matches.len() >= 4);
        assert!(report.matches.iter().any(|m| m.path.ends_with("main.rs")));
    }

    #[test]
    fn regex_mode_with_case_insensitive() {
        let root = make_tree();
        let opts = FindOptions {
            use_regex: true,
            case_sensitive: false,
            ..Default::default()
        };
        let report = find_matches(&root, r"HANDLE_\w+", &opts);
        assert!(report.matches.len() >= 4);
    }

    #[test]
    fn find_subslice_basics() {
        assert_eq!(find_subslice(b"hello world", b"world"), Some(6));
        assert_eq!(find_subslice(b"hello world", b"planet"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
        assert_eq!(find_subslice(b"", b"x"), None);
    }

    #[test]
    fn byte_to_column_counts_chars_not_bytes() {
        let prefix = "héllo ";
        assert_eq!(byte_to_column(prefix.as_bytes()), 6);
    }
}
