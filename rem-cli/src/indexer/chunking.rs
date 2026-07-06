use std::path::Path;

pub(crate) fn split_content_into_chunks(text: &str, target: usize) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut buf = String::with_capacity(target + 256);
    let mut cur_start_line = 1usize;
    let mut cur_line = 1usize;

    for line in text.lines() {
        let line_len = line.len() + 1;
        if buf.len() + line_len > target && !buf.trim().is_empty() {
            let end_l = cur_line.saturating_sub(1).max(cur_start_line);
            out.push((cur_start_line, end_l, std::mem::take(&mut buf)));
            cur_start_line = cur_line;
        }
        buf.push_str(line);
        buf.push('\n');
        cur_line += 1;
    }
    if !buf.trim().is_empty() {
        let end_l = (cur_line - 1).max(cur_start_line);
        out.push((cur_start_line, end_l, buf));
    }

    if out.len() == 1 && out[0].2.len() > target * 2 {
        let big = out.remove(0).2;
        let mut start = 0usize;
        let mut lnum = 1usize;
        while start < big.len() {
            let mut end = (start + target).min(big.len());
            end = big.floor_char_boundary(end);
            // Adjust end to nearest newline boundary to avoid mid-line split
            if end < big.len() {
                if let Some(newline_pos) = big[start..end].rfind('\n') {
                    end = start + newline_pos + 1;
                }
            }
            let piece = big[start..end].to_string();
            let piece_lines = piece.lines().count().max(1);
            out.push((lnum, lnum + piece_lines - 1, piece));
            lnum += piece_lines;
            start = end;
        }
    }
    out
}

/// Best-effort classification of a chunk for scoring bonuses in retrieval.
/// The retriever already gives +1 to "function" | "class" | "method".
pub(crate) fn guess_chunk_type(rel_path: &str, content: &str) -> &'static str {
    let ext = Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Scan up to 50 lines for signature-like things (files with many imports
    // at the top need deeper scanning to find the actual code definition)
    let head: String = content.lines().take(50).collect::<Vec<_>>().join("\n").to_lowercase();

    match ext.as_str() {
        "rs" => {
            if head.contains("fn ") || head.contains("pub fn ") || head.contains("pub async fn ") {
                "function"
            } else if head.contains("struct ")
                || head.contains("enum ")
                || head.contains("trait ")
                || head.contains("type ")
            {
                "type"
            } else if head.contains("mod ") || head.contains("pub mod ") {
                "module"
            } else if head.contains("impl ") {
                "impl"
            } else {
                "file"
            }
        }
        "py" | "pyi" => {
            if head.contains("class ") {
                "class"
            } else if head.contains("def ") || head.contains("async def ") {
                "function"
            } else {
                "file"
            }
        }
        "js" | "jsx" | "mjs" | "cjs" => {
            if head.contains("class ") {
                "class"
            } else if head.contains("function ")
                || head.contains("=>")
                || head.contains("const ")
                || head.contains("let ")
            {
                "function"
            } else {
                "file"
            }
        }
        "ts" | "tsx" => {
            if head.contains("class ") || head.contains("interface ") {
                "class"
            } else if head.contains("function ") || head.contains("=>") || head.contains("const ") {
                "function"
            } else {
                "file"
            }
        }
        "go" => {
            if head.contains("func ") {
                "function"
            } else {
                "file"
            }
        }
        "java" | "kt" | "scala" => {
            if head.contains("class ") || head.contains("interface ") || head.contains("object ") {
                "class"
            } else if head.contains("fun ")
                || head.contains("public ")
                || head.contains("private ")
                || head.contains("def ")
            {
                "function"
            } else {
                "file"
            }
        }
        "html" | "htm" => "html",
        "css" | "scss" | "less" => "css",
        "md" | "markdown" => "docs",
        "toml" | "yaml" | "yml" | "json" => "config",
        _ => "file",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guess_chunk_type_ts_function() {
        assert_eq!(guess_chunk_type("app.ts", "function foo() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_ts_class() {
        assert_eq!(guess_chunk_type("app.ts", "class Foo {}"), "class");
    }

    #[test]
    fn guess_chunk_type_ts_interface() {
        assert_eq!(guess_chunk_type("app.ts", "interface Bar {}"), "class");
    }

    #[test]
    fn guess_chunk_type_tsx_class() {
        assert_eq!(guess_chunk_type("app.tsx", "class Component {}"), "class");
    }

    #[test]
    fn guess_chunk_type_tsx_arrow() {
        assert_eq!(guess_chunk_type("app.tsx", "const Comp => {}"), "function");
    }

    #[test]
    fn guess_chunk_type_go_function() {
        assert_eq!(guess_chunk_type("main.go", "func main() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_go_file() {
        assert_eq!(guess_chunk_type("main.go", "package main"), "file");
    }

    #[test]
    fn guess_chunk_type_java_class() {
        assert_eq!(guess_chunk_type("Main.java", "class Main {}"), "class");
    }

    #[test]
    fn guess_chunk_type_java_interface() {
        assert_eq!(guess_chunk_type("Main.java", "interface Foo {}"), "class");
    }

    #[test]
    fn guess_chunk_type_java_method() {
        assert_eq!(guess_chunk_type("Util.java", "public void run() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_kt_class() {
        assert_eq!(guess_chunk_type("App.kt", "class App {}"), "class");
    }

    #[test]
    fn guess_chunk_type_kt_fun() {
        assert_eq!(guess_chunk_type("Util.kt", "fun foo() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_scala_class() {
        assert_eq!(guess_chunk_type("App.scala", "class App {}"), "class");
    }

    #[test]
    fn guess_chunk_type_scala_def() {
        assert_eq!(guess_chunk_type("Util.scala", "def foo() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_htm() {
        assert_eq!(guess_chunk_type("index.htm", "<html></html>"), "html");
    }

    #[test]
    fn guess_chunk_type_css() {
        assert_eq!(guess_chunk_type("style.css", "body {}"), "css");
    }

    #[test]
    fn guess_chunk_type_scss() {
        assert_eq!(guess_chunk_type("style.scss", "$color: red;"), "css");
    }

    #[test]
    fn guess_chunk_type_less() {
        assert_eq!(guess_chunk_type("style.less", "@color: red;"), "css");
    }

    #[test]
    fn guess_chunk_type_md() {
        assert_eq!(guess_chunk_type("README.md", "# Title"), "docs");
    }

    #[test]
    fn guess_chunk_type_yaml() {
        assert_eq!(guess_chunk_type("config.yaml", "key: value"), "config");
    }

    #[test]
    fn guess_chunk_type_yml() {
        assert_eq!(guess_chunk_type("config.yml", "key: value"), "config");
    }

    #[test]
    fn guess_chunk_type_json() {
        assert_eq!(guess_chunk_type("package.json", "{}"), "config");
    }

    #[test]
    fn guess_chunk_type_pyi() {
        assert_eq!(guess_chunk_type("types.pyi", "class Foo: ..."), "class");
    }

    #[test]
    fn guess_chunk_type_mjs() {
        assert_eq!(guess_chunk_type("app.mjs", "function foo() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_cjs() {
        assert_eq!(guess_chunk_type("app.cjs", "function foo() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_unknown() {
        assert_eq!(guess_chunk_type("data.csv", "a,b,c"), "file");
    }

    #[test]
    fn guess_chunk_type_empty_content() {
        assert_eq!(guess_chunk_type("lib.rs", ""), "file");
    }

    #[test]
    fn split_content_empty() {
        let result = split_content_into_chunks("", 100);
        assert!(result.is_empty());
    }

    #[test]
    fn split_content_single_line() {
        let result = split_content_into_chunks("hello world", 100);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].2, "hello world\n");
    }

    #[test]
    fn split_content_fits_in_one_chunk() {
        let result = split_content_into_chunks("a\nb\nc\n", 100);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 1);
        assert_eq!(result[0].1, 3);
    }

    #[test]
    fn split_content_overlarge_single_chunk() {
        let text = "x".repeat(500);
        let result = split_content_into_chunks(&text, 100);
        assert!(result.len() > 1, "should split overlarge chunk");
        for (_, _, chunk) in &result {
            assert!(chunk.len() <= 200, "each chunk should be at most target*2");
        }
    }

    #[test]
    fn split_content_line_tracking_multiple_chunks() {
        let text = (1..=10).map(|i| format!("line_{}", i)).collect::<Vec<_>>().join("\n");
        let result = split_content_into_chunks(&text, 10);
        assert!(result.len() > 1);
        let mut prev_end = 0usize;
        for (start, end, _) in &result {
            assert!(*start >= prev_end);
            assert!(*start <= *end);
            prev_end = *end + 1;
        }
    }
}
