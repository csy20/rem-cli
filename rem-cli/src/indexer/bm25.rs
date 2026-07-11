use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

use super::{CodebaseIndex, IndexChunk};

/// Tokenizes text into lowercase alphanumeric tokens (min 2 chars).
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    let estimated = text.len() / 20;
    let mut tokens = Vec::with_capacity(estimated.max(16));
    tokens.extend(
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 1)
            .map(|w| w.to_lowercase()),
    );
    tokens
}

/// Builds the inverted index and computes document frequencies from chunks.
pub(crate) fn build_inverted_index(
    chunks: &[IndexChunk],
    doc_freqs: &mut HashMap<String, u32>,
) -> HashMap<String, Vec<usize>> {
    let mut inverted: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let mut seen_in_chunk: HashSet<String> = HashSet::new();
        for w in tokenize(&chunk.content_lower) {
            if seen_in_chunk.insert(w.clone()) {
                *doc_freqs.entry(w.clone()).or_insert(0) += 1;
                inverted.entry(w).or_default().push(i);
            }
        }
    }
    inverted
}

/// BM25 retrieval using the pre-built inverted index for O(log n) candidate lookup
/// instead of scanning all chunks. Name/path/type bonuses are still checked on all
/// chunks for completeness.
pub fn retrieve_relevant_chunks<'a>(
    index: &'a CodebaseIndex,
    query: &str,
    top_k: usize,
    max_chars: usize,
) -> Vec<&'a IndexChunk> {
    if index.chunks.is_empty() || query.trim().is_empty() {
        return vec![];
    }
    let q_words = tokenize(query);
    if q_words.is_empty() {
        return vec![];
    }

    // Use inverted index to find candidate chunks (ones containing at least one query term)
    let mut candidate_indices: HashSet<usize> = HashSet::new();
    for w in &q_words {
        if let Some(ids) = index.inverted_index.get(w) {
            for &id in ids {
                candidate_indices.insert(id);
            }
        }
    }

    const K1: f64 = 1.5;
    const B: f64 = 0.75;
    let n = index.chunks.len() as f64;
    let avg_dl = if index.avg_dl > 0.0 { index.avg_dl } else { 1.0 };

    // Parallel BM25 scoring on candidate chunks (those containing query terms in content)
    let candidates: Vec<usize> = candidate_indices
        .iter()
        .copied()
        .filter(|&idx| idx < index.chunks.len())
        .collect();
    let mut scored: Vec<(f64, &IndexChunk)> = candidates
        .par_iter()
        .filter_map(|&idx| {
            let c = &index.chunks[idx];
            let mut score = 0.0f64;
            let dl = c.content.len() as f64;

            // Build token frequency map once per chunk (avoids O(q_words × tokens) scan)
            let token_counts: std::collections::HashMap<&str, usize> = {
                let mut map = std::collections::HashMap::new();
                for t in c
                    .content_lower
                    .split(|ch: char| !ch.is_alphanumeric())
                    .filter(|t| t.len() > 1)
                {
                    *map.entry(t).or_insert(0) += 1;
                }
                map
            };
            let has_name_or_path_match = q_words
                .iter()
                .any(|w| c.name_lower.contains(w) || c.path_lower.contains(w));

            for w in &q_words {
                let tf_val = token_counts.get(w.as_str()).copied().unwrap_or(0);
                if tf_val == 0 {
                    continue;
                }
                let tf = tf_val as f64;
                let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * (dl / avg_dl)));
                let df = index.doc_freqs.get(w).copied().unwrap_or(1) as f64;
                let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                score += tf_norm * idf;
            }

            // Name/path bonus (computed once, not per word)
            if has_name_or_path_match {
                score += 2.0;
            }

            // Chunk type bonus
            if matches!(c.chunk_type.as_str(), "function" | "class" | "method") {
                score += 0.5;
            }

            if score > 0.0 {
                Some((score, c))
            } else {
                None
            }
        })
        .collect();

    // Also scan all chunks for name/path matches that weren't content candidates
    for (idx, c) in index.chunks.iter().enumerate() {
        if candidate_indices.contains(&idx) {
            continue; // already scored
        }
        let has_name_or_path = q_words
            .iter()
            .any(|w| c.name_lower.contains(w) || c.path_lower.contains(w));
        let mut bonus = 0.0f64;
        if has_name_or_path {
            bonus += 2.0;
        }
        if matches!(c.chunk_type.as_str(), "function" | "class" | "method") {
            bonus += 0.5;
        }
        if bonus > 0.0 {
            scored.push((bonus, c));
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_empty() {
        let result = tokenize("");
        assert!(result.is_empty());
    }

    #[test]
    fn tokenize_short_words_filtered() {
        let result = tokenize("a b cd ef");
        assert_eq!(result, vec!["cd", "ef"]);
    }

    #[test]
    fn tokenize_lowercase() {
        let result = tokenize("Hello WORLD");
        assert!(result.contains(&"hello".to_string()));
        assert!(result.contains(&"world".to_string()));
    }

    #[test]
    fn tokenize_with_underscores() {
        let result = tokenize("fn_123 bar_456");
        assert_eq!(result, vec!["fn", "123", "bar", "456"]);
    }

    #[test]
    fn tokenize_unicode() {
        let result = tokenize("café résumé");
        assert!(result.contains(&"café".to_string()));
        assert!(result.contains(&"résumé".to_string()));
    }

    #[test]
    fn tokenize_special_chars() {
        let result = tokenize("hello.world!foo-bar");
        assert_eq!(result, vec!["hello", "world", "foo", "bar"]);
    }

    #[test]
    fn build_inverted_index_empty() {
        let chunks = vec![];
        let mut doc_freqs = HashMap::new();
        let inverted = build_inverted_index(&chunks, &mut doc_freqs);
        assert!(inverted.is_empty());
        assert!(doc_freqs.is_empty());
    }

    #[test]
    fn build_inverted_index_single_chunk() {
        let chunks = vec![IndexChunk {
            path: "src/lib.rs".into(),
            name: "lib.rs".into(),
            chunk_type: "function".into(),
            content: "fn hello() {}".into(),
            content_lower: "fn hello() {}".into(),
            name_lower: "lib.rs".into(),
            path_lower: "src/lib.rs".into(),
            start_line: 1,
            end_line: 1,
            embedding: None,
        }];
        let mut doc_freqs = HashMap::new();
        let inverted = build_inverted_index(&chunks, &mut doc_freqs);
        assert!(inverted.contains_key("hello"));
        assert!(inverted.contains_key("fn"));
        assert_eq!(doc_freqs.get("hello"), Some(&1));
    }

    #[test]
    fn build_inverted_index_doc_freq() {
        let chunks = vec![
            IndexChunk {
                path: "a.rs".into(),
                name: "a.rs".into(),
                chunk_type: "file".into(),
                content: "hello world".into(),
                content_lower: "hello world".into(),
                name_lower: "a.rs".into(),
                path_lower: "a.rs".into(),
                start_line: 1,
                end_line: 1,
                embedding: None,
            },
            IndexChunk {
                path: "b.rs".into(),
                name: "b.rs".into(),
                chunk_type: "file".into(),
                content: "hello there".into(),
                content_lower: "hello there".into(),
                name_lower: "b.rs".into(),
                path_lower: "b.rs".into(),
                start_line: 1,
                end_line: 1,
                embedding: None,
            },
        ];
        let mut doc_freqs = HashMap::new();
        let inverted = build_inverted_index(&chunks, &mut doc_freqs);
        assert_eq!(doc_freqs.get("hello"), Some(&2));
        assert_eq!(doc_freqs.get("world"), Some(&1));
        assert_eq!(doc_freqs.get("there"), Some(&1));
        assert_eq!(inverted.get("hello").unwrap().len(), 2);
    }

    #[test]
    fn tokenize_null_bytes_ignored() {
        let result = tokenize("hello\0world");
        assert!(result.contains(&"hello".to_string()));
        assert!(result.contains(&"world".to_string()));
    }

    #[test]
    fn tokenize_numbers_kept() {
        let result = tokenize("abc123 def456");
        assert!(result.contains(&"abc123".to_string()));
        assert!(result.contains(&"def456".to_string()));
    }

    #[test]
    fn tokenize_single_chars_filtered() {
        let result = tokenize("x y z abc");
        assert_eq!(result, vec!["abc"]);
    }

    fn make_index(chunks: Vec<IndexChunk>) -> CodebaseIndex {
        let mut doc_freqs = HashMap::new();
        let inverted_index = build_inverted_index(&chunks, &mut doc_freqs);
        let num_chunks = chunks.len();
        let avg_dl = if num_chunks > 0 {
            chunks.iter().map(|c| c.content.len() as f64).sum::<f64>() / num_chunks as f64
        } else {
            1.0
        };
        CodebaseIndex {
            version: 2,
            generated_at: String::new(),
            file_mtimes: HashMap::new(),
            chunks,
            inverted_index,
            doc_freqs,
            num_chunks,
            avg_dl,
        }
    }

    #[test]
    fn retrieve_relevant_empty_chunks_list() {
        let index = make_index(vec![]);
        let result = retrieve_relevant_chunks(&index, "hello", 5, 1000);
        assert!(result.is_empty());
    }

    #[test]
    fn retrieve_relevant_empty_query() {
        let chunk = IndexChunk {
            path: "test.rs".into(),
            name: "test.rs".into(),
            chunk_type: "file".into(),
            content: "hello world".into(),
            content_lower: "hello world".into(),
            name_lower: "test.rs".into(),
            path_lower: "test.rs".into(),
            start_line: 1,
            end_line: 1,
            embedding: None,
        };
        let index = make_index(vec![chunk]);
        let result = retrieve_relevant_chunks(&index, "", 5, 1000);
        assert!(result.is_empty());
    }

    #[test]
    fn retrieve_relevant_query_does_not_match_name_or_path() {
        let chunk = IndexChunk {
            path: "test.rs".into(),
            name: "test.rs".into(),
            chunk_type: "file".into(),
            content: "some code here".into(),
            content_lower: "some code here".into(),
            name_lower: "test.rs".into(),
            path_lower: "test.rs".into(),
            start_line: 1,
            end_line: 1,
            embedding: None,
        };
        let index = make_index(vec![chunk]);
        let result = retrieve_relevant_chunks(&index, "nonexistent_token", 5, 1000);
        assert!(result.is_empty(), "unrelated query should return no results");
    }
}
