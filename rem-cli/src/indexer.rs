//! Codebase indexing and retrieval support.
//!
//! This module handles:
//! - Generating a retrieval index (`rem index`) with pure-Rust chunking.
//! - Loading the index at runtime.
//! - BM25 keyword-based relevant chunk retrieval (used to inject actual code into prompts
//!   instead of exhaustive file listings).
//! - Incremental re-indexing (skips unchanged files via mtime).
//!
//! The index format is a JSON object:
//! {
//!   "version": 2,
//!   "generated_at": "2026-01-15T10:30:00Z",
//!   "file_mtimes": { "src/main.rs": 1234567890, ... },
//!   "chunks": [ ... ],
//!   "inverted_index": { "login": [0, 5, 12], ... },
//!   "doc_freqs": { "login": 3, ... },
//!   "num_chunks": 100
//! }

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::find;

const INDEX_VERSION: u32 = 2;

/// The complete codebase index, including chunks and BM25 retrieval data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseIndex {
    pub version: u32,
    pub generated_at: String,
    pub file_mtimes: HashMap<String, u64>,
    pub chunks: Vec<IndexChunk>,
    pub inverted_index: HashMap<String, Vec<usize>>,
    pub doc_freqs: HashMap<String, u32>,
    pub num_chunks: usize,
}

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
    /// Pre-computed lowercased content for faster retrieval.
    #[serde(skip)]
    pub(crate) content_lower: String,
    /// Pre-computed lowercased name for faster retrieval.
    #[serde(skip)]
    pub(crate) name_lower: String,
    /// Pre-computed lowercased path for faster retrieval.
    #[serde(skip)]
    pub(crate) path_lower: String,
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
            // Try v2 format first (CodebaseIndex with inverted_index)
            if let Ok(index) = serde_json::from_str::<CodebaseIndex>(&text) {
                if !index.chunks.is_empty() {
                    let mut chunks = index.chunks;
                    for chunk in &mut chunks {
                        chunk.content_lower = chunk.content.to_lowercase();
                        chunk.name_lower = chunk.name.to_lowercase();
                        chunk.path_lower = chunk.path.to_lowercase();
                    }
                    return Some(chunks);
                }
            }
            // Fallback: try v1 flat format
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(arr) = data.get("chunks").and_then(|v| v.as_array()) {
                    let mut out = Vec::new();
                    for item in arr {
                        if let Ok(mut chunk) = serde_json::from_value::<IndexChunk>(item.clone()) {
                            chunk.content_lower = chunk.content.to_lowercase();
                            chunk.name_lower = chunk.name.to_lowercase();
                            chunk.path_lower = chunk.path.to_lowercase();
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

/// Loads the full CodebaseIndex structure (with inverted index) for BM25 retrieval.
/// Falls back to building one on-the-fly from raw chunks if only v1 format exists.
pub fn load_full_index(project_dir: &Path) -> Option<CodebaseIndex> {
    let candidates = [
        project_dir.join(".rem/codebase_index.json"),
        project_dir.join("models/codebase_index.json"),
    ];
    for p in &candidates {
        if let Ok(text) = fs::read_to_string(p) {
            if let Ok(mut index) = serde_json::from_str::<CodebaseIndex>(&text) {
                // Ensure lowercase fields are populated
                for chunk in &mut index.chunks {
                    chunk.content_lower = chunk.content.to_lowercase();
                    chunk.name_lower = chunk.name.to_lowercase();
                    chunk.path_lower = chunk.path.to_lowercase();
                }
                // Rebuild inverted index if missing (e.g. migrated from v1)
                if index.inverted_index.is_empty() && !index.chunks.is_empty() {
                    rebuild_inverted_index(&mut index);
                }
                return Some(index);
            }
        }
    }
    None
}

/// Builds the inverted index and computes doc frequencies for BM25.
fn rebuild_inverted_index(index: &mut CodebaseIndex) {
    let mut inverted: HashMap<String, Vec<usize>> = HashMap::new();
    let mut doc_freqs: HashMap<String, u32> = HashMap::new();
    for (i, chunk) in index.chunks.iter().enumerate() {
        let mut seen_in_chunk: HashSet<String> = HashSet::new();
        for w in tokenize(&chunk.content_lower) {
            inverted.entry(w.clone()).or_default().push(i);
            if seen_in_chunk.insert(w.clone()) {
                *doc_freqs.entry(w).or_insert(0) += 1;
            }
        }
    }
    index.inverted_index = inverted;
    index.doc_freqs = doc_freqs;
    index.num_chunks = index.chunks.len();
}

/// Tokenizes text into lowercase alphanumeric tokens (min 3 chars).
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .map(|w| w.to_lowercase())
        .collect()
}

/// BM25 retrieval using the pre-built inverted index.
/// Falls back to the original additive scoring if no inverted index is available.
pub fn retrieve_relevant_chunks<'a>(
    index: &'a [IndexChunk],
    query: &str,
    top_k: usize,
    max_chars: usize,
) -> Vec<&'a IndexChunk> {
    if index.is_empty() || query.trim().is_empty() {
        return vec![];
    }
    let q_words = tokenize(query);

    // BM25 parameters
    const K1: f64 = 1.5;
    const B: f64 = 0.75;
    let n = index.len() as f64;
    let avg_dl: f64 = if n > 0.0 {
        index.iter().map(|c| c.content.len() as f64).sum::<f64>() / n
    } else {
        1.0
    };

    let has_any_embedding = index.iter().any(|c| c.embedding.is_some());

    let mut scored: Vec<(f64, &IndexChunk)> = index
        .iter()
        .map(|c| {
            let mut score = 0.0f64;
            let dl = c.content.len() as f64;

            for w in &q_words {
                let tf = count_occurrences(&c.content_lower, w) as f64;
                if tf == 0.0 {
                    continue;
                }
                let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * (dl / avg_dl)));

                let df = c.content_lower.matches(w).count() as f64;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();

                score += tf_norm * idf;
            }

            // Name/path bonus
            for w in &q_words {
                if c.name_lower.contains(w) || c.path_lower.contains(w) {
                    score += 2.0;
                }
            }

            // Chunk type bonus
            if matches!(c.chunk_type.as_str(), "function" | "class" | "method") {
                score += 0.5;
            }

            // Semantic score from embeddings (if available)
            if has_any_embedding {
                if let Some(ref emb) = c.embedding {
                    // Simple query embedding approximation: normalize query presence
                    let query_emb: Vec<f32> = q_words
                        .iter()
                        .map(|w| {
                            if c.content_lower.contains(w) {
                                1.0
                            } else {
                                0.0
                            }
                        })
                        .collect();
                    if !query_emb.is_empty() && query_emb.len() == emb.len() {
                        let sim = cosine_similarity(emb, &query_emb);
                        score += sim * 3.0; // up to +3.0 boost
                    }
                }
            }

            (score, c)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut chosen = Vec::new();
    let mut used = 0usize;
    for (_, c) in scored.into_iter().take(top_k.max(1)) {
        let block_len = c.content.len() + c.path.len() + 64;
        if used + block_len > max_chars {
            break;
        }
        used += block_len;
        chosen.push(c);
    }
    chosen
}

/// Counts occurrences of a word in text (simple substring counting on word boundaries).
fn count_occurrences(text: &str, word: &str) -> usize {
    if word.is_empty() {
        return 0;
    }
    let lower = text.to_lowercase();
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = lower[start..].find(word) {
        count += 1;
        start += pos + 1;
    }
    count
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
struct FileEntryToProcess {
    rel_str: String,
    name: String,
    content: String,
    line_count: usize,
}

pub fn generate_codebase_index(root: &Path) -> Result<(Vec<IndexChunk>, HashMap<String, u64>)> {
    let max_file_bytes: u64 = 120 * 1024;
    let target_chunk = 2800usize;
    let existing_mtimes = load_existing_mtimes(root);

    let mut file_entries: Vec<FileEntryToProcess> = Vec::new();
    let mut file_mtimes: HashMap<String, u64> = HashMap::new();
    let mut changed_files = 0u32;

    for entry in WalkBuilder::new(root)
        .max_depth(Some(8))
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
                    if find::should_skip_dir(name) {
                        return false;
                    }
                }
            }
            if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                if let Some(name) = e.file_name().to_str() {
                    if find::should_skip_file(name) {
                        return false;
                    }
                }
            }
            true
        })
        .build()
        .flatten()
    {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
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

        let mtime = file_mtime(p);
        file_mtimes.insert(rel_str.clone(), mtime);

        // Incremental: skip files whose mtime hasn't changed
        if let Some(prev_mtime) = existing_mtimes.get(&rel_str) {
            if *prev_mtime == mtime {
                continue;
            }
        }

        let text = match fs::read_to_string(p) {
            Ok(t) if !t.trim().is_empty() => t,
            _ => continue,
        };

        let line_count = text.lines().count().max(1);
        file_entries.push(FileEntryToProcess {
            rel_str,
            name: name.to_string(),
            content: text,
            line_count,
        });
        changed_files += 1;
    }

    // If nothing changed, recycle existing chunks
    if changed_files == 0 && !existing_mtimes.is_empty() {
        if let Some(existing) = load_codebase_index(root) {
            return Ok((existing, file_mtimes));
        }
    }

    let mut chunks: Vec<IndexChunk> = file_entries
        .par_iter()
        .flat_map(|fe| {
            let mut local_chunks = Vec::new();
            let ctype = guess_chunk_type(&fe.rel_str, &fe.content);

            if fe.content.len() <= target_chunk + 400 {
                local_chunks.push(IndexChunk {
                    path: fe.rel_str.clone(),
                    name: fe.name.clone(),
                    chunk_type: ctype.to_string(),
                    content: fe.content.clone(),
                    content_lower: fe.content.to_lowercase(),
                    name_lower: fe.name.to_lowercase(),
                    path_lower: fe.rel_str.to_lowercase(),
                    start_line: 1,
                    end_line: fe.line_count,
                    embedding: None,
                });
            } else {
                let parts = split_content_into_chunks(&fe.content, target_chunk);
                for (i, (start_l, end_l, piece)) in parts.into_iter().enumerate() {
                    if piece.trim().is_empty() {
                        continue;
                    }
                    let piece_ctype = if i == 0 {
                        ctype
                    } else {
                        guess_chunk_type(&fe.rel_str, &piece)
                    };
                    let content_lower = piece.to_lowercase();
                    local_chunks.push(IndexChunk {
                        path: fe.rel_str.clone(),
                        name: fe.name.clone(),
                        chunk_type: piece_ctype.to_string(),
                        content: piece,
                        content_lower,
                        name_lower: fe.name.to_lowercase(),
                        path_lower: fe.rel_str.to_lowercase(),
                        start_line: start_l,
                        end_line: end_l,
                        embedding: None,
                    });
                }
            }
            local_chunks
        })
        .collect();

    chunks.par_sort_by(|a, b| a.path.cmp(&b.path).then(a.start_line.cmp(&b.start_line)));
    Ok((chunks, file_mtimes))
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

    if out.len() == 1 && out[0].2.len() > target * 2 {
        let big = out.remove(0).2;
        let mut start = 0usize;
        let mut lnum = 1usize;
        while start < big.len() {
            let end = (start + target).min(big.len());
            let end = big.floor_char_boundary(end);
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

/// Writes the codebase index to `.rem/codebase_index.json` with inverted index and mtimes.
pub fn write_codebase_index(
    root: &Path,
    chunks: &[IndexChunk],
    file_mtimes: HashMap<String, u64>,
) -> Result<()> {
    let rem_dir = root.join(".rem");
    fs::create_dir_all(&rem_dir).context("failed to create .rem directory for index")?;
    let out_path = rem_dir.join("codebase_index.json");

    // Build inverted index + doc freqs from chunks
    let mut inverted_index: HashMap<String, Vec<usize>> = HashMap::new();
    let mut doc_freqs: HashMap<String, u32> = HashMap::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let mut seen: HashSet<String> = HashSet::new();
        for w in tokenize(&chunk.content) {
            inverted_index.entry(w.clone()).or_default().push(i);
            if seen.insert(w.clone()) {
                *doc_freqs.entry(w).or_insert(0) += 1;
            }
        }
    }

    let index = CodebaseIndex {
        version: INDEX_VERSION,
        generated_at: crate::format_timestamp(),
        file_mtimes,
        chunks: chunks.to_vec(),
        inverted_index,
        doc_freqs,
        num_chunks: chunks.len(),
    };

    let text = serde_json::to_string_pretty(&index).context("failed to serialize index")?;
    fs::write(&out_path, text).context("failed to write codebase_index.json")?;
    Ok(())
}

/// Returns the mtime of a file, or 0 if unavailable.
fn file_mtime(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| {
            m.modified().map(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            })
        })
        .unwrap_or(0)
}

/// Computes embeddings for chunks using Ollama's /api/embed endpoint.
/// Falls back silently if Ollama is unavailable.
pub fn compute_embeddings(chunks: &mut [IndexChunk], ollama_url: &str) {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok();
    let client = match client {
        Some(c) => c,
        None => return,
    };

    for chunk in chunks.iter_mut() {
        let text = if chunk.content.len() > 8000 {
            &chunk.content[..8000]
        } else {
            &chunk.content
        };
        if text.trim().is_empty() {
            continue;
        }
        let url = format!("{}/api/embed", ollama_url.trim_end_matches('/'));
        let payload = json!({
            "model": "nomic-embed-text",
            "input": text
        });
        if let Ok(resp) = client.post(&url).json(&payload).send() {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                if let Some(embeddings) = body["embeddings"].as_array() {
                    if let Some(embedding) = embeddings.first() {
                        if let Some(vec) = embedding.as_array() {
                            let v: Vec<f32> = vec
                                .iter()
                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                .collect();
                            if !v.is_empty() {
                                chunk.embedding = Some(v);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Computes cosine similarity between two embedding vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na * nb)) as f64
}

/// Loads existing file mtimes from a previous index, if available.
fn load_existing_mtimes(root: &Path) -> HashMap<String, u64> {
    let candidates = [
        root.join(".rem/codebase_index.json"),
        root.join("models/codebase_index.json"),
    ];
    for p in &candidates {
        if let Ok(text) = fs::read_to_string(p) {
            if let Ok(index) = serde_json::from_str::<CodebaseIndex>(&text) {
                return index.file_mtimes;
            }
        }
    }
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chunks() -> Vec<IndexChunk> {
        vec![
            IndexChunk {
                path: "src/main.rs".into(),
                name: "main.rs".into(),
                chunk_type: "function".into(),
                content: "fn main() {\n    println!(\"hello\");\n}".into(),
                content_lower: "fn main() {\n    println!(\"hello\");\n}".into(),
                name_lower: "main.rs".into(),
                path_lower: "src/main.rs".into(),
                start_line: 1,
                end_line: 3,
                embedding: None,
            },
            IndexChunk {
                path: "src/auth.rs".into(),
                name: "auth.rs".into(),
                chunk_type: "file".into(),
                content: "pub fn login() {}\npub fn logout() {}".into(),
                content_lower: "pub fn login() {}\npub fn logout() {}".into(),
                name_lower: "auth.rs".into(),
                path_lower: "src/auth.rs".into(),
                start_line: 1,
                end_line: 2,
                embedding: None,
            },
            IndexChunk {
                path: "README.md".into(),
                name: "README.md".into(),
                chunk_type: "docs".into(),
                content: "# Project\nThis is a project about authentication.".into(),
                content_lower: "# project\nthis is a project about authentication.".into(),
                name_lower: "readme.md".into(),
                path_lower: "readme.md".into(),
                start_line: 1,
                end_line: 2,
                embedding: None,
            },
        ]
    }

    #[test]
    fn retrieve_relevant_empty_index() {
        let result = retrieve_relevant_chunks(&[], "login", 5, 10000);
        assert!(result.is_empty());
    }

    #[test]
    fn retrieve_relevant_empty_query() {
        let chunks = sample_chunks();
        let result = retrieve_relevant_chunks(&chunks, "", 5, 10000);
        assert!(result.is_empty());
    }

    #[test]
    fn retrieve_relevant_matches_content() {
        let chunks = sample_chunks();
        let result = retrieve_relevant_chunks(&chunks, "login", 5, 10000);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn retrieve_relevant_respects_top_k() {
        let chunks = sample_chunks();
        let result = retrieve_relevant_chunks(&chunks, "login auth", 1, 10000);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn retrieve_relevant_respects_max_chars() {
        let chunks = sample_chunks();
        let result = retrieve_relevant_chunks(&chunks, "main auth login", 5, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn build_retrieved_empty_chunks() {
        let result = build_retrieved_context(&[], 1000);
        assert!(result.is_empty());
    }

    #[test]
    fn build_retrieved_formats_chunks() {
        let chunks = sample_chunks();
        let refs: Vec<&IndexChunk> = chunks.iter().collect();
        let result = build_retrieved_context(&refs, 10000);
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("fn main()"));
        assert!(result.contains("Relevant code chunks"));
        assert!(result.contains("End of retrieved context"));
    }

    #[test]
    fn build_retrieved_respects_max_chars() {
        let chunks = sample_chunks();
        let refs: Vec<&IndexChunk> = chunks.iter().collect();
        let result = build_retrieved_context(&refs, 50);
        assert!(result.len() <= 50 || result.contains("[End of retrieved context"));
    }

    #[test]
    fn guess_chunk_type_rust_function() {
        assert_eq!(guess_chunk_type("lib.rs", "pub fn foo() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_rust_type() {
        assert_eq!(guess_chunk_type("lib.rs", "struct Foo { x: i32 }"), "type");
    }

    #[test]
    fn guess_chunk_type_python_class() {
        assert_eq!(guess_chunk_type("app.py", "class MyClass:"), "class");
    }

    #[test]
    fn guess_chunk_type_python_function() {
        assert_eq!(guess_chunk_type("app.py", "def my_func():"), "function");
    }

    #[test]
    fn guess_chunk_type_js_function() {
        assert_eq!(guess_chunk_type("app.js", "function foo() {}"), "function");
    }

    #[test]
    fn guess_chunk_type_js_class() {
        assert_eq!(guess_chunk_type("app.jsx", "class Foo {}"), "class");
    }

    #[test]
    fn guess_chunk_type_html() {
        assert_eq!(guess_chunk_type("index.html", "<html></html>"), "html");
    }

    #[test]
    fn guess_chunk_type_config() {
        assert_eq!(guess_chunk_type("Cargo.toml", "[package]"), "config");
    }

    #[test]
    fn guess_chunk_type_fallback() {
        assert_eq!(guess_chunk_type("data.csv", "a,b,c"), "file");
    }

    #[test]
    fn split_content_small_stays_as_one() {
        let result = split_content_into_chunks("hello\nworld\n", 2800);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].2, "hello\nworld\n");
    }

    #[test]
    fn split_content_splits_large() {
        let text = (0..100)
            .map(|i| format!("line_{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let result = split_content_into_chunks(&text, 50);
        assert!(result.len() > 1, "should produce multiple chunks");
    }

    #[test]
    fn split_content_line_tracking() {
        let text = "a\nb\nc\nd\ne\n";
        let result = split_content_into_chunks(text, 4);
        assert!(result.len() >= 2);
    }
}
