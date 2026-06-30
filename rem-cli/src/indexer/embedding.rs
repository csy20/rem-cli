use futures_util::future::join_all;

use super::IndexChunk;

/// Computes embeddings for chunks using Ollama's /api/embed endpoint.
/// Uses async reqwest with batched concurrent requests for large chunk sets.
/// Falls back silently if Ollama is unavailable.
pub async fn compute_embeddings(chunks: &mut [IndexChunk], ollama_url: &str) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .ok();
    let client = match client {
        Some(c) => c,
        None => {
            tracing::warn!("failed to build HTTP client for embeddings (Ollama unavailable?)");
            return;
        }
    };

    let url = format!("{}/api/embed", ollama_url.trim_end_matches('/'));
    let batch_size = 10;
    for batch in chunks.chunks_mut(batch_size) {
        let mut futures = Vec::with_capacity(batch.len());
        for chunk in batch.iter() {
            let byte_len = chunk.content.len();
            let cutoff = if byte_len > 8000 {
                (0..=8000)
                    .rev()
                    .find(|&i| chunk.content.is_char_boundary(i))
                    .unwrap_or(0)
            } else {
                byte_len
            };
            let text = if byte_len > cutoff {
                chunk.content[..cutoff].to_string()
            } else {
                chunk.content.clone()
            };
            let payload = serde_json::json!({
                "model": "nomic-embed-text",
                "input": text
            });
            let req = client.post(&url).json(&payload).send();
            futures.push(async move {
                if text.trim().is_empty() {
                    return None;
                }
                let resp = req.await.ok()?;
                let body: serde_json::Value = resp.json().await.ok()?;
                let embeddings = body.get("embeddings")?.as_array()?;
                let embedding = embeddings.first()?;
                let vec: Vec<f32> = embedding
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_f64().map(|f| f as f32))
                    .collect();
                if vec.is_empty() {
                    None
                } else {
                    Some(vec)
                }
            });
        }
        let results: Vec<Option<Vec<f32>>> = join_all(futures).await;
        for (chunk, result) in batch.iter_mut().zip(results) {
            if let Some(emb) = result {
                chunk.embedding = Some(emb);
            }
        }
    }
}
