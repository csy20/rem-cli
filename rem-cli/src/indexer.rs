//! Codebase indexing and retrieval support.
//!
//! This module handles:
//! - Generating a retrieval index (`rem index`) with pure-Rust chunking.
//! - Loading the index at runtime.
//! - Keyword-based relevant chunk retrieval (used to inject actual code into prompts
//!   instead of exhaustive file listings).
//!
//! The index format is a simple JSON:
//! {
//!   "chunks": [
//!     {
//!       "path": "src/foo.rs",
//!       "name": "foo.rs",
//!       "chunk_type": "function" | "class" | "file" | "section" | ...,
//!       "content": "...",
//!       "start_line": 12,
//!       "end_line": 45,
//!       "embedding": null
//!     },
//!     ...
//!   ]
//! }
//!
//! Chunk types are best-effort and used to slightly boost scoring for functions/classes
//! in `retrieve_relevant_chunks`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::find;

/// Chunk of source code stored in the retrieval index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexChunk {
    pub path: String,
    pub name: String,
    #[serde(default, rename = "chunk_type")]
    pub chunk_type: String,
    pub content: String,
    #[serde(default)]
    pub start_line: usize,
    #[serde(default)]
    pub end_line: usize,
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
}

/// Try to load an index for the given project dir.
/// Conventional locations (in order):
///   <project>/.rem/codebase_index.json
///   <project>/models/codebase_index.json   (legacy)
/// Returns None if not present or unreadable.
pub fn load_codebase_index(project_dir: &Path) -> Option<Vec<IndexChunk>> {
    let candidates = [
        project_dir.join(".rem/codebase_index.json"),
        project_dir.join("models/codebase_index.json"),
    ];
    for p in &candidates {
        if let Ok(text) = fs::read_to_string(p) {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(arr) = data.get("chunks").and_then(|v| v.as_array()) {
                    let mut out = Vec::new();
                    for item in arr {
                        if let Ok(chunk) = serde_json::from_value::<IndexChunk>(item.clone()) {
                            out.push(chunk);
                        }
                    }
                    if !out.is_empty() {
                        return Some(out);
                    }
                }
            }
        }
    }
    None
}

/// Keyword-based retrieval (with light name/path bonus). Fast, no extra deps, works even
/// if embeddings are absent or we don't want to call an embedder for the query yet.
/// This is a huge scaling win vs. dumping every filename + size: we inject *actual code*
/// for chunks whose content matches the user query / task.
pub fn retrieve_relevant_chunks<'a>(
    index: &'a [IndexChunk],
    query: &str,
    top_k: usize,
    max_chars: usize,
) -> Vec<&'a IndexChunk> {
    if index.is_empty() || query.trim().is_empty() {
        return vec![];
    }
    let q = query.to_lowercase();
    let q_words: Vec<&str> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .collect();

    let mut scored: Vec<(i32, &IndexChunk)> = index
        .iter()
        .map(|c| {
            let mut score = 0i32;

            // Strong signal: words appear in the actual code content
            if c.content.len() < 20000 {
                let content_l = c.content.to_lowercase();
                for w in &q_words {
                    if content_l.contains(w) {
                        score += 10;
                    }
                }
            }
            // Bonus for name / path match (e.g. "auth" in auth.rs or user auth handler)
            if c.name.len() < 500 && c.path.len() < 500 {
                let name_l = c.name.to_lowercase();
                let path_l = c.path.to_lowercase();
                for w in &q_words {
                    if name_l.contains(w) || path_l.contains(w) {
                        score += 4;
                    }
                }
            }
            // Light recency / size bias not needed; prefer matches.

            // Extra if the chunk type is useful (function/class > generic file)
            if matches!(c.chunk_type.as_str(), "function" | "class" | "method") {
                score += 1;
            }

            (score, c)
        })
        .filter(|(s, _)| *s > 0)
        .collect();

    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));

    let mut chosen = Vec::new();
    let mut used = 0usize;
    for (_, c) in scored.into_iter().take(top_k.max(1)) {
        let block_len = c.content.len() + c.path.len() + 64; // rough header
        if used + block_len > max_chars {
            break;
        }
        used += block_len;
        chosen.push(c);
    }
    chosen
}

/// Build a compact "Relevant code from project (via index):" section for injection.
/// Called from the main prompt assembly when an index is present for the project.
pub fn build_retrieved_context(chunks: &[&IndexChunk], max_chars: usize) -> String {
    if chunks.is_empty() {
        return String::new();
    }
    let mut out = String::from("[Relevant code chunks from project index]:\n");
    let mut used = out.len();
    for c in chunks {
        let loc = if c.start_line > 0 && c.end_line > 0 {
            format!("{}:{}-{}", c.path, c.start_line, c.end_line)
        } else {
            c.path.clone()
        };
        let header = format!("### {} ({})\n", loc, c.chunk_type);
        let body = format!("{}\n\n", c.content);
        let add = header.len() + body.len();
        if used + add > max_chars {
            break;
        }
        out.push_str(&header);
        out.push_str(&body);
        used += add;
    }
    if out.len() > 30 {
        out.push_str(
            "[End of retrieved context — use @path for more specific files if needed]\n\n",
        );
    }
    out
}

// ── Generation (the `rem index` implementation) ─────────────────────────────

/// Walk + chunk a project into IndexChunk entries (matches the shape expected by
/// load_codebase_index / retrieve_relevant_chunks / build_retrieved_context).
pub fn generate_codebase_index(root: &Path) -> Result<Vec<IndexChunk>> {
    let mut chunks: Vec<IndexChunk> = Vec::new();
    let max_file_bytes: u64 = 120 * 1024;
    let target_chunk = 2800usize;

    for entry in WalkDir::new(root)
        .max_depth(8)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if let Some(name) = e.file_name().to_str() {
                if e.file_type().is_dir() && find::should_skip_dir(name) {
                    return false;
                }
                if e.file_type().is_file() && find::should_skip_file(name) {
                    return false;
                }
            }
            true
        })
    {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let p = entry.path();
        let Ok(rel) = p.strip_prefix(root) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().to_string();
        if rel_str.is_empty() || rel_str.starts_with('.') {
            continue;
        }

        // Extra guard for obvious backup/generated files
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".rs.bk") || name.contains(".lock") || name.ends_with(".bin") {
            continue;
        }

        let meta = match p.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() > max_file_bytes {
            continue;
        }

        let text = match fs::read_to_string(p) {
            Ok(t) if !t.trim().is_empty() => t,
            _ => continue, // non-utf8 or empty → skip
        };

        let line_count = text.lines().count().max(1);

        let ctype = guess_chunk_type(&rel_str, &text);

        if text.len() <= target_chunk + 400 {
            chunks.push(IndexChunk {
                path: rel_str.clone(),
                name: p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| rel_str.clone()),
                chunk_type: ctype.to_string(),
                content: text,
                start_line: 1,
                end_line: line_count,
                embedding: None,
            });
        } else {
            let parts = split_content_into_chunks(&text, target_chunk);
            for (i, (start_l, end_l, piece)) in parts.into_iter().enumerate() {
                if piece.trim().is_empty() {
                    continue;
                }
                let piece_ctype = if i == 0 {
                    ctype
                } else {
                    // For interior sections, try to detect a strong symbol at the start of this piece
                    guess_chunk_type(&rel_str, &piece)
                };
                chunks.push(IndexChunk {
                    path: rel_str.clone(),
                    name: p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| rel_str.clone()),
                    chunk_type: piece_ctype.to_string(),
                    content: piece,
                    start_line: start_l,
                    end_line: end_l,
                    embedding: None,
                });
            }
        }
    }

    chunks.sort_by(|a, b| a.path.cmp(&b.path).then(a.start_line.cmp(&b.start_line)));
    Ok(chunks)
}

fn split_content_into_chunks(text: &str, target: usize) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut buf = String::with_capacity(target + 256);
    let mut cur_start_line = 1usize;
    let mut cur_line = 1usize;

    for line in text.lines() {
        let line_len = line.len() + 1;
        if buf.len() + line_len > target && !buf.trim().is_empty() {
            let end_l = cur_line.saturating_sub(1).max(cur_start_line);
            out.push((cur_start_line, end_l, buf.clone()));
            buf.clear();
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

    // Force split giant single chunk (rare, e.g. minified or one huge paragraph)
    if out.len() == 1 && out[0].2.len() > target * 2 {
        let big = out.remove(0).2;
        let mut start = 0usize;
        let mut lnum = 1usize;
        while start < big.len() {
            let end = (start + target).min(big.len());
            let piece = &big[start..end];
            let piece_lines = piece.lines().count().max(1);
            out.push((lnum, lnum + piece_lines - 1, piece.to_string()));
            lnum += piece_lines;
            start = end;
            if start < big.len() && big.as_bytes().get(start) == Some(&b'\n') {
                start += 1;
            }
        }
    }
    out
}

/// Best-effort classification of a chunk for scoring bonuses in retrieval.
/// The retriever already gives +1 to "function" | "class" | "method".
fn guess_chunk_type(rel_path: &str, content: &str) -> &'static str {
    let ext = Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Look at the first several lines for signature-like things
    let head: String = content
        .lines()
        .take(12)
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

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

/// Writes the codebase index to `.rem/codebase_index.json`.
pub fn write_codebase_index(root: &Path, chunks: &[IndexChunk]) -> Result<()> {
    let rem_dir = root.join(".rem");
    fs::create_dir_all(&rem_dir).context("failed to create .rem directory for index")?;
    let out_path = rem_dir.join("codebase_index.json");
    let payload = serde_json::json!({ "chunks": chunks });
    let text = serde_json::to_string_pretty(&payload).context("failed to serialize index")?;
    fs::write(&out_path, text).context("failed to write codebase_index.json")?;
    Ok(())
}
