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

    // Look at the first several lines for signature-like things
    let head: String = content.lines().take(12).collect::<Vec<_>>().join("\n").to_lowercase();

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
