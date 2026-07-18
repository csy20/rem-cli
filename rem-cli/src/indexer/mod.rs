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
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use ignore::WalkBuilder;
use memmap2::Mmap;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::find;

mod bm25;
mod chunking;
pub(crate) use bm25::build_inverted_index;
pub use bm25::retrieve_relevant_chunks;
pub(crate) use chunking::{guess_chunk_type, split_content_into_chunks};

const INDEX_VERSION: u32 = 2;

/// The complete codebase index, including chunks and BM25 retrieval data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodebaseIndex {
    pub version: u32,
    pub generated_at: String,
    pub file_mtimes: HashMap<String, u64>,
    pub chunks: Vec<IndexChunk>,
    pub inverted_index: HashMap<String, Vec<usize>>,
    pub doc_freqs: HashMap<String, u32>,
    pub num_chunks: usize,
    /// Pre-computed average chunk length for BM25 scoring (avoids O(n) recomputation).
    #[serde(default)]
    pub avg_dl: f64,
}

/// Chunk of source code stored in the retrieval index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Pre-computed token → frequency map for BM25 scoring,
    /// avoiding O(chunks × query_terms) re-tokenization at query time.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub(crate) token_counts: std::collections::HashMap<String, usize>,
}

/// Try to load an index for the given project dir, returning the full `CodebaseIndex`
/// (with inverted_index, doc_freqs, and pre-lowercased chunk fields).
/// Conventional locations (in order):
///   <project>/.rem/codebase_index.msgpack  (fastest — MessagePack format)
///   <project>/.rem/codebase_index.json.gz  (compressed JSON)
///   <project>/.rem/codebase_index.json      (plain JSON)
///   <project>/models/codebase_index.json   (legacy)
/// Returns None if not present or unreadable.
pub fn load_codebase_index(project_dir: &Path) -> Option<CodebaseIndex> {
    // Fast path: MessagePack format (preferred — 60-80% smaller, zero-copy capable)
    let msgpack_path = project_dir.join(".rem/codebase_index.msgpack");
    if let Ok(file) = fs::File::open(&msgpack_path) {
        let mmap = unsafe {
            // SAFETY: The mapped file is read-only and not written to during
            // the lifetime of the mapping. The data is validated immediately
            // via rmp_serde::from_slice, which handles malformed input.
            Mmap::map(&file).ok()
        };
        if let Some(mmap) = mmap {
            match rmp_serde::from_slice::<CodebaseIndex>(&mmap) {
                Ok(index) if !index.chunks.is_empty() => return Some(index),
                _ => {
                    // Deserialize from reader fallback if mmap path fails
                    let mut reader = std::io::BufReader::new(file);
                    if let Ok(index) = rmp_serde::from_read::<&mut _, CodebaseIndex>(&mut reader) {
                        if !index.chunks.is_empty() {
                            return Some(index);
                        }
                    }
                }
            }
        }
    }

    // Try compressed format (.json.gz), then plain JSON
    let gz_path = project_dir.join(".rem/codebase_index.json.gz");
    if let Ok(file) = fs::File::open(&gz_path) {
        let mut decoder = GzDecoder::new(file);
        let mut text = String::new();
        if decoder.read_to_string(&mut text).is_ok() {
            if let Ok(index) = serde_json::from_str::<CodebaseIndex>(&text) {
                if !index.chunks.is_empty() {
                    return Some(index);
                }
            }
        }
    }

    // Fallback: plain JSON with memmap for zero-copy reads
    let candidates = [
        project_dir.join(".rem/codebase_index.json"),
        project_dir.join("models/codebase_index.json"),
    ];
    for p in &candidates {
        let text = match try_mmap_read(p) {
            Some(t) => t,
            None => continue,
        };
        let parsed_v2 = serde_json::from_str::<CodebaseIndex>(&text);
        // Try v2 format first (CodebaseIndex with inverted_index)
        if let Ok(index) = parsed_v2 {
            if !index.chunks.is_empty() {
                return Some(index);
            }
        } else if fs::metadata(p).map(|m| m.len()).unwrap_or(0) > 0 {
            // File exists and has content but failed to parse — likely corrupted
            tracing::warn!(
                "index file {} appears corrupted (failed to parse as v2 or v1), consider regenerating with `rem index`",
                p.display()
            );
        }
        // Fallback: try v1 flat format
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(arr) = data.get("chunks").and_then(|v| v.as_array()) {
                let mut chunks = Vec::new();
                for item in arr {
                    if let Ok(chunk) = serde_json::from_value::<IndexChunk>(item.clone()) {
                        // v1 format — build_inverted_index uses content directly (tokenize lowercases)
                        chunks.push(chunk);
                    }
                }
                if !chunks.is_empty() {
                    let num_chunks = chunks.len();
                    let mut doc_freqs = HashMap::new();
                    let inverted_index = build_inverted_index(&chunks, &mut doc_freqs);
                    let avg_dl = chunks.iter().map(|c| c.content.len() as f64).sum::<f64>() / num_chunks as f64;
                    return Some(CodebaseIndex {
                        version: INDEX_VERSION,
                        generated_at: String::new(),
                        file_mtimes: HashMap::new(),
                        chunks,
                        inverted_index,
                        doc_freqs,
                        num_chunks,
                        avg_dl,
                    });
                }
            }
        }
    }
    None
}

/// Try to read a file into a String using memory-mapped I/O for zero-copy.
/// Falls back to `fs::read_to_string` if mmap is unavailable.
fn try_mmap_read(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    let len = metadata.len() as usize;
    if len == 0 {
        // Some indices are gzip-compressed — mmap won't help, fall through
        return fs::read_to_string(path).ok();
    }
    // Try mmap first for fast zero-copy reads
    if let Ok(mmap) = unsafe {
        // SAFETY: The mapped file is read-only and not written to during
        // the lifetime of the mapping. Data is validated via from_utf8
        // before use.
        Mmap::map(&file)
    } {
        std::str::from_utf8(&mmap).ok().map(|s| s.to_string())
    } else {
        fs::read_to_string(path).ok()
    }
}

/// Build a structured "Relevant code from project (via index):" section for injection.
/// Called from the main prompt assembly when an index is present for the project.
pub fn build_retrieved_context(chunks: &[&IndexChunk], max_chars: usize) -> String {
    if chunks.is_empty() {
        return String::new();
    }
    let header_prefix = "[Relevant code chunks from project index]:\n";
    let mut used = header_prefix.len();
    let mut out = String::with_capacity(max_chars.min(4096));
    out.push_str(header_prefix);
    const FOOTER: &str = "[End of retrieved context — use @path for more specific files if needed]\n\n";
    for c in chunks {
        let loc = if c.start_line > 0 && c.end_line > 0 {
            format!("{}:{}-{}", c.path, c.start_line, c.end_line)
        } else {
            c.path.clone()
        };
        let header_len = loc.len() + 7 + c.chunk_type.len(); // "### " + " (" + ")\n"
        if used + header_len + c.content.len() + 2 > max_chars {
            break;
        }
        out.push_str("### ");
        out.push_str(&loc);
        out.push_str(" (");
        out.push_str(&c.chunk_type);
        out.push_str(")\n");
        out.push_str(&c.content);
        out.push_str("\n\n");
        used += header_len + c.content.len() + 2;
    }
    if used > header_prefix.len() {
        out.push_str(FOOTER);
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
    let max_file_bytes: u64 = crate::constants::INDEX_MAX_FILE_BYTES;
    let target_chunk = crate::constants::INDEX_TARGET_CHUNK_BYTES;
    let existing_mtimes = load_mtimes(root);

    let mut file_mtimes: HashMap<String, u64> = HashMap::new();
    let mut changed_files = 0u32;
    let mut scanned_count = 0u32;
    let scan_start = Instant::now();

    // Phase 1: Walk directory tree (sequential) — collect paths and metadata
    struct WalkEntry {
        rel_str: String,
        name: String,
        path: PathBuf,
    }

    let mut walk_entries: Vec<WalkEntry> = Vec::new();

    for entry in WalkBuilder::new(root)
        .max_depth(Some(crate::constants::INDEX_MAX_DEPTH))
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

        scanned_count += 1;
        if scanned_count.is_multiple_of(100) && std::io::stderr().is_terminal() {
            let changed = changed_files;
            let elapsed = scan_start.elapsed();
            eprint!(
                "\r  \u{1F50D} {} files scanned ({} new/changed, {}.{:02}s)",
                scanned_count,
                changed,
                elapsed.as_secs(),
                elapsed.subsec_millis() / 10
            );
            let _ = std::io::stderr().flush();
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

        changed_files += 1;
        walk_entries.push(WalkEntry {
            rel_str,
            name: name.to_string(),
            path: p.to_path_buf(),
        });
    }

    // Clear progress line
    if scanned_count >= 100 && std::io::stderr().is_terminal() {
        eprint!("\r\x1b[2K");
        let _ = std::io::stderr().flush();
    }

    // If nothing changed, recycle existing chunks
    if changed_files == 0 && !existing_mtimes.is_empty() {
        if let Some(existing) = load_codebase_index(root) {
            return Ok((existing.chunks, file_mtimes));
        }
    }

    // Phase 2: Read files and chunk them (parallel only for large sets)
    let process_entry = |we: &WalkEntry| -> Option<FileEntryToProcess> {
        let text = std::fs::read_to_string(&we.path)
            .ok()
            .filter(|t| !t.trim().is_empty())?;
        let line_count = text.lines().count().max(1);
        Some(FileEntryToProcess {
            rel_str: we.rel_str.clone(),
            name: we.name.clone(),
            content: text,
            line_count,
        })
    };
    let file_entries: Vec<FileEntryToProcess> = if walk_entries.len() > 100 {
        walk_entries.par_iter().filter_map(&process_entry).collect()
    } else {
        walk_entries.iter().filter_map(process_entry).collect()
    };

    let num_files = file_entries.len();
    let chunk_start = Instant::now();
    if num_files > 0 && std::io::stderr().is_terminal() {
        eprint!("\r  \u{2699} chunking {} files...", num_files);
        let _ = std::io::stderr().flush();
    }
    let mut new_chunks: Vec<IndexChunk> = if file_entries.len() > 100 {
        file_entries
            .into_par_iter()
            .flat_map(|fe| chunk_file_entry(fe, target_chunk))
            .collect()
    } else {
        file_entries
            .into_iter()
            .flat_map(|fe| chunk_file_entry(fe, target_chunk))
            .collect()
    };
    if num_files > 0 && std::io::stderr().is_terminal() {
        let chunk_elapsed = chunk_start.elapsed();
        if new_chunks.len() > num_files {
            eprint!(
                "\r  \u{2705} chunked into {} chunks from {} files ({}.{:02}s)\n",
                new_chunks.len(),
                num_files,
                chunk_elapsed.as_secs(),
                chunk_elapsed.subsec_millis() / 10
            );
        } else {
            eprint!(
                "\r  \u{2705} {} files chunked ({}.{:02}s)\n",
                num_files,
                chunk_elapsed.as_secs(),
                chunk_elapsed.subsec_millis() / 10
            );
        }
        let _ = std::io::stderr().flush();
    }

    // Incremental merge: preserve unchanged chunks from the old index
    let changed_paths: HashSet<&str> = walk_entries.iter().map(|e| e.rel_str.as_str()).collect();
    if !existing_mtimes.is_empty() {
        if let Some(old) = load_codebase_index(root) {
            let unchanged: Vec<IndexChunk> = old
                .chunks
                .into_iter()
                .filter(|c| {
                    // Keep chunks for files that (a) still exist in the current walk
                    // and (b) were not re-processed (mtime unchanged)
                    file_mtimes.contains_key(&c.path) && !changed_paths.contains(c.path.as_str())
                })
                .collect();
            // Dedup: avoid duplicate chunks when a file's chunk content overlaps
            let existing_keys: HashSet<(String, usize)> =
                new_chunks.iter().map(|c| (c.path.clone(), c.start_line)).collect();
            let unique_unchanged: Vec<IndexChunk> = unchanged
                .into_iter()
                .filter(|c| !existing_keys.contains(&(c.path.clone(), c.start_line)))
                .collect();
            // Prepend unchanged chunks so they appear before new ones
            new_chunks.splice(0..0, unique_unchanged);
        }
    }

    new_chunks.par_sort_by(|a, b| a.path.cmp(&b.path).then(a.start_line.cmp(&b.start_line)));
    Ok((new_chunks, file_mtimes))
}

fn chunk_file_entry(fe: FileEntryToProcess, target_chunk: usize) -> Vec<IndexChunk> {
    let mut local_chunks = Vec::new();
    let ctype = guess_chunk_type(&fe.rel_str, &fe.content);
    let tokenize_for_chunk = |content: &str| -> std::collections::HashMap<String, usize> {
        let mut map = std::collections::HashMap::new();
        for t in bm25::tokenize(content) {
            *map.entry(t).or_insert(0) += 1;
        }
        map
    };

    if fe.content.len() <= target_chunk + 400 {
        local_chunks.push(IndexChunk {
            path: fe.rel_str.clone(),
            name: fe.name.clone(),
            chunk_type: ctype.to_string(),
            content: fe.content.clone(),
            start_line: 1,
            end_line: fe.line_count,
            token_counts: tokenize_for_chunk(&fe.content),
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
            let token_counts = tokenize_for_chunk(&piece);
            local_chunks.push(IndexChunk {
                path: fe.rel_str.clone(),
                name: fe.name.clone(),
                chunk_type: piece_ctype.to_string(),
                content: piece,
                start_line: start_l,
                end_line: end_l,
                token_counts,
            });
        }
    }
    local_chunks
}

/// Writes the codebase index to `.rem/codebase_index.json` with inverted index and mtimes.
pub fn write_codebase_index(root: &Path, chunks: Vec<IndexChunk>, file_mtimes: HashMap<String, u64>) -> Result<()> {
    let rem_dir = root.join(".rem");
    fs::create_dir_all(&rem_dir).context("failed to create .rem directory for index")?;
    let out_path = rem_dir.join("codebase_index.json");

    // Build inverted index + doc freqs from chunks using pre-lowercased content
    let mut doc_freqs: HashMap<String, u32> = HashMap::new();
    let inverted_index = build_inverted_index(&chunks, &mut doc_freqs);

    let num_chunks = chunks.len();
    let avg_dl = if num_chunks > 0 {
        chunks.iter().map(|c| c.content.len() as f64).sum::<f64>() / num_chunks as f64
    } else {
        1.0
    };
    let index = CodebaseIndex {
        version: INDEX_VERSION,
        generated_at: crate::format_timestamp(),
        file_mtimes,
        chunks,
        inverted_index,
        doc_freqs,
        num_chunks,
        avg_dl,
    };

    // Save mtimes to a separate small file for fast loading without parsing full index
    save_mtimes(root, &index.file_mtimes);

    // Serialize to bytes once, share across format writers
    let json_bytes = serde_json::to_vec_pretty(&index).context("failed to serialize index")?;
    let tmp_path = rem_dir.join("codebase_index.json.tmp");
    let out_path_clone = out_path.clone();
    let msgpack_rem_dir = rem_dir.clone();

    // Write all three formats in parallel for faster index generation
    std::thread::scope(|s| {
        // JSON (primary format) — write directly from bytes
        s.spawn(|| {
            let _ = fs::write(&tmp_path, &json_bytes);
            let _ = fs::rename(&tmp_path, &out_path_clone);
        });

        // Gzip compressed JSON
        s.spawn(|| {
            let gz_path = msgpack_rem_dir.join("codebase_index.json.gz");
            if let Ok(file) = fs::File::create(&gz_path) {
                let mut encoder = GzEncoder::new(file, Compression::default());
                if let Err(e) = encoder.write_all(&json_bytes) {
                    tracing::warn!("failed to write compressed index: {e}");
                }
            }
        });

        // MessagePack (fastest load path)
        s.spawn(|| {
            let msgpack_path = msgpack_rem_dir.join("codebase_index.msgpack");
            match rmp_serde::to_vec(&index) {
                Ok(bytes) => {
                    if let Err(e) = fs::write(&msgpack_path, &bytes) {
                        tracing::warn!("failed to write msgpack index: {e}");
                    }
                }
                Err(e) => tracing::warn!("failed to serialize msgpack index: {e}"),
            }
        });
    });
    Ok(())
}

/// Saves file mtimes to a separate small JSON file for fast loading
/// without parsing the full codebase index.
fn save_mtimes(root: &Path, mtimes: &HashMap<String, u64>) {
    let path = root.join(".rem/mtimes.json");
    if let Ok(json) = serde_json::to_string(mtimes) {
        let _ = fs::write(&path, &json);
    }
}

/// Loads file mtimes from the separate mtimes file if available,
/// falling back to parsing the full codebase index.
fn load_mtimes(root: &Path) -> HashMap<String, u64> {
    let path = root.join(".rem/mtimes.json");
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(mtimes) = serde_json::from_str::<HashMap<String, u64>>(&text) {
            // Detect second-precision values and convert to milliseconds
            let is_seconds = mtimes.values().all(|v| *v < 10_000_000_000);
            if is_seconds {
                return mtimes.into_iter().map(|(k, v)| (k, v * 1000)).collect();
            }
            return mtimes;
        }
    }
    load_existing_mtimes(root)
}

/// Returns the mtime of a file in milliseconds since epoch, or 0 if unavailable.
/// Millisecond precision avoids false-positive changes in rapid save scenarios
/// (e.g., file watcher or `rem index` called twice within one second).
fn file_mtime(path: &Path) -> u64 {
    path.metadata()
        .and_then(|m| {
            m.modified().map(|t| {
                t.duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0) as u64
            })
        })
        .unwrap_or(0)
}

/// Loads existing file mtimes from a previous index, if available.
/// Automatically converts second-precision values (from older index versions)
/// to millisecond precision for correct comparison.
fn load_existing_mtimes(root: &Path) -> HashMap<String, u64> {
    let candidates = [
        root.join(".rem/codebase_index.json"),
        root.join("models/codebase_index.json"),
    ];
    for p in &candidates {
        if p.exists() {
            if let Ok(text) = fs::read_to_string(p) {
                if let Ok(index) = serde_json::from_str::<CodebaseIndex>(&text) {
                    let mtimes = index.file_mtimes;
                    // Detect second-precision values (< 10B, timestamp in seconds before ~2286)
                    // and convert to millisecond precision
                    let is_seconds = mtimes.values().all(|v| *v < 10_000_000_000);
                    if is_seconds {
                        return mtimes.into_iter().map(|(k, v)| (k, v * 1000)).collect();
                    }
                    return mtimes;
                }
            }
        }
    }
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::bm25::tokenize;

    fn sample_index() -> CodebaseIndex {
        let chunks = vec![
            IndexChunk {
                path: "src/main.rs".into(),
                name: "main.rs".into(),
                chunk_type: "function".into(),
                content: "fn main() {\n    println!(\"hello\");\n}".into(),
                start_line: 1,
                end_line: 3,
                token_counts: HashMap::new(),
            },
            IndexChunk {
                path: "src/auth.rs".into(),
                name: "auth.rs".into(),
                chunk_type: "file".into(),
                content: "pub fn login() {}\npub fn logout() {}".into(),
                start_line: 1,
                end_line: 2,
                token_counts: HashMap::new(),
            },
            IndexChunk {
                path: "README.md".into(),
                name: "README.md".into(),
                chunk_type: "docs".into(),
                content: "# Project\nThis is a project about authentication.".into(),
                start_line: 1,
                end_line: 2,
                token_counts: HashMap::new(),
            },
        ];
        let mut doc_freqs = HashMap::new();
        let inverted_index = build_inverted_index(&chunks, &mut doc_freqs);
        CodebaseIndex {
            version: INDEX_VERSION,
            generated_at: String::new(),
            file_mtimes: HashMap::new(),
            chunks,
            inverted_index,
            doc_freqs,
            num_chunks: 3,
            avg_dl: 0.0,
        }
    }

    #[test]
    fn retrieve_relevant_empty_index() {
        let empty = CodebaseIndex {
            version: INDEX_VERSION,
            generated_at: String::new(),
            file_mtimes: HashMap::new(),
            chunks: vec![],
            inverted_index: HashMap::new(),
            doc_freqs: HashMap::new(),
            num_chunks: 0,
            avg_dl: 0.0,
        };
        let result = retrieve_relevant_chunks(&empty, "login", 5, 10000);
        assert!(result.is_empty());
    }

    #[test]
    fn retrieve_relevant_empty_query() {
        let index = sample_index();
        let result = retrieve_relevant_chunks(&index, "", 5, 10000);
        assert!(result.is_empty());
    }

    #[test]
    fn retrieve_relevant_matches_content() {
        let index = sample_index();
        let result = retrieve_relevant_chunks(&index, "login", 5, 10000);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn retrieve_relevant_respects_top_k() {
        let index = sample_index();
        let result = retrieve_relevant_chunks(&index, "login auth", 1, 10000);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn retrieve_relevant_respects_max_chars() {
        let index = sample_index();
        let result = retrieve_relevant_chunks(&index, "main auth login", 5, 10);
        assert!(result.is_empty());
    }

    #[test]
    fn build_retrieved_empty_chunks() {
        let result = build_retrieved_context(&[], 1000);
        assert!(result.is_empty());
    }

    #[test]
    fn build_retrieved_formats_chunks() {
        let index = sample_index();
        let refs: Vec<&IndexChunk> = index.chunks.iter().collect();
        let result = build_retrieved_context(&refs, 10000);
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("fn main()"));
        assert!(result.contains("Relevant code chunks"));
        assert!(result.contains("End of retrieved context"));
    }

    #[test]
    fn build_retrieved_respects_max_chars() {
        let index = sample_index();
        let refs: Vec<&IndexChunk> = index.chunks.iter().collect();
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
        let text = (0..100).map(|i| format!("line_{}", i)).collect::<Vec<_>>().join("\n");
        let result = split_content_into_chunks(&text, 50);
        assert!(result.len() > 1, "should produce multiple chunks");
    }

    #[test]
    fn split_content_line_tracking() {
        let text = "a\nb\nc\nd\ne\n";
        let result = split_content_into_chunks(text, 4);
        assert!(result.len() >= 2);
    }

    // ── Quick benchmarks (timing-based, runs with cargo test) ──────────

    #[test]
    fn bench_tokenize_large_text() {
        let text = (0..1000)
            .map(|i| format!("word_{} fn_login_authenticate_validate_token_{}", i, i))
            .collect::<Vec<_>>()
            .join(" ");
        let start = std::time::Instant::now();
        let tokens = tokenize(&text);
        let elapsed = start.elapsed();
        assert!(tokens.len() > 1000, "should produce many tokens");
        assert!(
            elapsed.as_millis() < 200,
            "tokenizing 1000 words took {}ms (expected <200ms)",
            elapsed.as_millis()
        );
    }

    #[test]
    fn bench_split_content_large_file() {
        let text = (0..10_000)
            .map(|i| format!("line_{}: some content here", i))
            .collect::<Vec<_>>()
            .join("\n");
        let start = std::time::Instant::now();
        let chunks = split_content_into_chunks(&text, 200);
        let elapsed = start.elapsed();
        assert!(chunks.len() > 1, "should split 10k lines into chunks");
        assert!(
            elapsed.as_millis() < 1000,
            "splitting 10k lines took {}ms (expected <1000ms)",
            elapsed.as_millis()
        );
    }

    #[test]
    fn incremental_index_merge_unchanged_chunks() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rem-incr-idx-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("stable.rs"), "fn stable() {}").unwrap();
        fs::write(root.join("changing.rs"), "fn old_version() {}").unwrap();
        fs::write(root.join("README.md"), "# Initial").unwrap();

        // First index — all files
        let (chunks1, mtimes1) = generate_codebase_index(&root).unwrap();
        write_codebase_index(&root, chunks1, mtimes1).unwrap();
        let stable_count = load_codebase_index(&root).unwrap().chunks.len();

        // Second index — no changes, should recycle
        let (chunks2, _) = generate_codebase_index(&root).unwrap();
        assert_eq!(chunks2.len(), stable_count, "unchanged index should recycle all chunks");

        // Modify one file
        fs::write(root.join("changing.rs"), "fn new_version() {}").unwrap();
        // Also create a new file
        fs::write(root.join("new.rs"), "fn new() {}").unwrap();

        // Third index — only changed/new files re-processed, stable ones merged
        let (chunks3, mtimes3) = generate_codebase_index(&root).unwrap();
        assert!(
            chunks3.len() > stable_count,
            "should have more chunks after adding a file"
        );
        let paths3: HashSet<&str> = chunks3.iter().map(|c| c.path.as_str()).collect();
        assert!(paths3.contains("stable.rs"), "stable.rs must still be present");
        assert!(paths3.contains("new.rs"), "new.rs must be present");
        assert!(paths3.contains("changing.rs"), "changing.rs must be present");
        // The content of changing.rs should reflect the new version
        assert!(chunks3
            .iter()
            .any(|c| c.path == "changing.rs" && c.content.contains("new_version")));

        // Write and reload — verify persistency
        write_codebase_index(&root, chunks3, mtimes3).unwrap();
        let loaded = load_codebase_index(&root).unwrap();
        assert!(loaded.chunks.iter().any(|c| c.path == "new.rs"));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn full_index_cycle_walk_build_retrieve() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rem-idx-cycle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("main.rs"), "fn hello() { println!(\"hello world\"); }").unwrap();
        fs::write(root.join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();
        fs::write(root.join("README.md"), "# Test Project").unwrap();
        // Ignored dirs should be excluded
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), "ignored").unwrap();

        // Generate index (walks + chunks)
        let (chunks, mtimes) = generate_codebase_index(&root).unwrap();
        assert!(chunks.len() >= 3, "should produce at least 3 chunks");

        // Write index
        write_codebase_index(&root, chunks.clone(), mtimes).unwrap();
        assert!(root.join(".rem/codebase_index.json").exists());

        // Load index back
        let loaded = load_codebase_index(&root).unwrap();
        assert_eq!(loaded.chunks.len(), chunks.len());

        // Retrieve relevant
        let retrieved = retrieve_relevant_chunks(&loaded, "hello", 5, 2000);
        assert!(!retrieved.is_empty(), "should find 'hello'-related chunks");

        // Cleanup
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn build_retrieved_footer_not_duplicated() {
        let index = sample_index();
        let refs: Vec<&IndexChunk> = index.chunks.iter().collect();
        let result = build_retrieved_context(&refs, 10000);
        let footer = "[End of retrieved context";
        let first = result.find(footer);
        let last = result.rfind(footer);
        assert_eq!(first, last, "footer should appear exactly once");
    }

    #[test]
    fn build_retrieved_max_chars_boundary() {
        let index = sample_index();
        let refs: Vec<&IndexChunk> = index.chunks.iter().collect();
        // Exactly enough for one chunk + footer
        let result = build_retrieved_context(&refs, 150);
        assert!(result.contains("Relevant code chunks"));
        assert!(result.contains("[End of retrieved context"));
    }

    #[test]
    fn build_retrieved_mixed_chunk_types() {
        let chunks = [
            IndexChunk {
                path: "src/main.rs".into(),
                name: "main.rs".into(),
                chunk_type: "function".into(),
                content: "fn main() {}".into(),
                start_line: 1,
                end_line: 1,
                token_counts: HashMap::new(),
            },
            IndexChunk {
                path: "docs/README.md".into(),
                name: "README.md".into(),
                chunk_type: "docs".into(),
                content: "# Documentation".into(),
                start_line: 1,
                end_line: 1,
                token_counts: HashMap::new(),
            },
        ];
        let refs: Vec<&IndexChunk> = chunks.iter().collect();
        let result = build_retrieved_context(&refs, 10000);
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("docs/README.md"));
        assert!(result.contains("(function)"));
        assert!(result.contains("(docs)"));
    }

    #[test]
    fn build_retrieved_empty_input_edge_cases() {
        assert_eq!(build_retrieved_context(&[], 0), "");
        assert_eq!(build_retrieved_context(&[], 1000), "");
    }

    #[test]
    fn save_and_load_mtimes_roundtrip() {
        use std::fs;
        let root = std::env::temp_dir().join(format!("rem-mtimes-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".rem")).unwrap();

        let mut mtimes = HashMap::new();
        mtimes.insert("src/main.rs".to_string(), 1234567890123u64);
        mtimes.insert("src/lib.rs".to_string(), 1234567890456u64);

        save_mtimes(&root, &mtimes);
        assert!(root.join(".rem/mtimes.json").exists());

        let loaded = load_mtimes(&root);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("src/main.rs"), Some(&1234567890123u64));
        assert_eq!(loaded.get("src/lib.rs"), Some(&1234567890456u64));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn load_mtimes_falls_back_to_index_when_no_separate_file() {
        let root = std::env::temp_dir().join(format!("rem-mtimes-fallback-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // No .rem/mtimes.json exists — should return empty rather than panic
        let loaded = load_mtimes(&root);
        assert!(loaded.is_empty());
    }
}
