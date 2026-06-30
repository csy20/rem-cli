use std::collections::{HashMap, HashSet};

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
            }
            inverted.entry(w).or_default().push(i);
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
    let avg_dl: f64 = if n > 0.0 {
        index.chunks.iter().map(|c| c.content.len() as f64).sum::<f64>() / n
    } else {
        1.0
    };

    let mut scored: Vec<(f64, &IndexChunk)> = Vec::with_capacity(candidate_indices.len() + 16);

    // BM25 scoring only on candidate chunks (those containing query terms in content)
    for &idx in &candidate_indices {
        let c = &index.chunks[idx];
        let mut score = 0.0f64;
        let dl = c.content.len() as f64;

        for w in &q_words {
            let tf_val = count_occurrences(&c.content_lower, w);
            if tf_val == 0 {
                continue;
            }
            let tf = tf_val as f64;
            let tf_norm = (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * (dl / avg_dl)));
            let df = index.doc_freqs.get(w).copied().unwrap_or(1) as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            score += tf_norm * idf;
        }

        // Name/path bonus for candidates
        for w in &q_words {
            if c.name_lower.contains(w) || c.path_lower.contains(w) {
                score += 2.0;
            }
        }

        // Chunk type bonus
        if matches!(c.chunk_type.as_str(), "function" | "class" | "method") {
            score += 0.5;
        }

        if score > 0.0 {
            scored.push((score, c));
        }
    }

    // Also scan all chunks for name/path matches that weren't content candidates
    for (idx, c) in index.chunks.iter().enumerate() {
        if candidate_indices.contains(&idx) {
            continue; // already scored
        }
        let mut bonus = 0.0f64;
        for w in &q_words {
            if c.name_lower.contains(w) || c.path_lower.contains(w) {
                bonus += 2.0;
            }
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

/// Counts occurrences of `word` as a standalone token in lowercased `text`.
/// Both `text` and `word` are expected to be pre-lowercased.
/// Uses the same tokenization as the indexer for consistency.
fn count_occurrences(text: &str, word: &str) -> usize {
    if word.is_empty() || text.is_empty() {
        return 0;
    }
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1 && *t == word)
        .count()
}
