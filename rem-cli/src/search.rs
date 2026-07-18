//! Web search with configurable backends (DuckDuckGo, Google, Bing).
//! When an API key is configured for Google or Bing, uses the proper search API;
//! otherwise falls back to DuckDuckGo HTML scraping.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use crate::ui;
use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};

/// Time-to-live for cached search results.
const SEARCH_CACHE_TTL_SECS: u64 = 300;
/// Maximum number of cached search results before evicting oldest entries.
const SEARCH_CACHE_MAX_ENTRIES: usize = 256;

/// Combined cache entry with insertion order for LRU-like eviction.
/// Results are wrapped in Arc to avoid deep cloning on cache hits.
struct CacheEntry {
    results: Arc<Vec<SearchResult>>,
    cached_at: Instant,
}

/// In-memory cache for web search results keyed by normalized query.
/// Uses a single Mutex to avoid dual-lock contention and simplify eviction.
static SEARCH_CACHE: LazyLock<Mutex<SearchCacheInner>> = LazyLock::new(|| {
    Mutex::new(SearchCacheInner {
        entries: HashMap::new(),
        order: Vec::new(),
    })
});

struct SearchCacheInner {
    entries: HashMap<String, CacheEntry>,
    order: Vec<String>,
}

impl SearchCacheInner {
    fn get(&mut self, key: &str) -> Option<Vec<SearchResult>> {
        // First check if expired and remove
        let is_expired = self
            .entries
            .get(key)
            .is_some_and(|e| e.cached_at.elapsed().as_secs() >= SEARCH_CACHE_TTL_SECS);
        if is_expired {
            self.entries.remove(key);
            self.order.retain(|k| k != key);
            return None;
        }
        // Return results from cache hit (cheap Arc clone)
        if let Some(entry) = self.entries.get(key) {
            self.order.retain(|k| k != key);
            self.order.push(key.to_string());
            return Some(Arc::clone(&entry.results).as_ref().clone());
        }
        None
    }

    fn insert(&mut self, key: String, results: Vec<SearchResult>) {
        // Evict oldest if at capacity
        if !self.entries.contains_key(&key) && self.entries.len() >= SEARCH_CACHE_MAX_ENTRIES {
            if let Some(oldest) = self.order.first().cloned() {
                self.entries.remove(&oldest);
                self.order.retain(|k| k != &oldest);
            }
        }
        self.entries.insert(
            key.clone(),
            CacheEntry {
                results: Arc::new(results),
                cached_at: Instant::now(),
            },
        );
        self.order.retain(|k| k != &key);
        self.order.push(key);
    }

    fn purge_expired(&mut self) {
        let expired: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.cached_at.elapsed().as_secs() >= SEARCH_CACHE_TTL_SECS)
            .map(|(k, _)| k.clone())
            .collect();
        for k in &expired {
            self.entries.remove(k);
            self.order.retain(|key| key != k);
        }
    }
}

/// A single web search result with title, snippet, and URL.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub snippet: String,
    pub url: String,
}

/// Search provider configuration.
#[derive(Debug, Clone)]
pub enum SearchProvider {
    Google { api_key: String, cse_id: String },
    Bing { api_key: String },
}

/// Performs a web search using the configured provider, with automatic fallback.
/// Results are cached in-memory for SEARCH_CACHE_TTL_SECS to avoid redundant network calls.
pub(crate) async fn perform_web_search(
    client: &Client,
    query: &str,
    provider: Option<&SearchProvider>,
) -> Result<Vec<SearchResult>> {
    let cache_key = query.trim().to_lowercase();

    // Check cache first
    {
        let mut cache = SEARCH_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(results) = cache.get(&cache_key) {
            return Ok(results);
        }
        // Purge expired entries opportunistically
        cache.purge_expired();
    }

    // Perform the search
    let results = perform_web_search_uncached(client, query, provider).await?;

    // Store in cache with LRU-like eviction (single lock)
    if let Ok(mut cache) = SEARCH_CACHE.lock() {
        cache.insert(cache_key, results.clone());
    }

    Ok(results)
}

async fn perform_web_search_uncached(
    client: &Client,
    query: &str,
    provider: Option<&SearchProvider>,
) -> Result<Vec<SearchResult>> {
    let primary = match provider {
        Some(SearchProvider::Google { api_key, cse_id }) => Some(search_google(client, query, api_key, cse_id).await),
        Some(SearchProvider::Bing { api_key }) => Some(search_bing(client, query, api_key).await),
        None => None,
    };

    match primary {
        Some(Ok(results)) => Ok(results),
        Some(Err(primary_err)) => {
            tracing::warn!("primary search failed, falling back to DDG: {primary_err}");
            search_ddg(client, query).await
        }
        None => search_ddg(client, query).await,
    }
}

async fn search_google(client: &Client, query: &str, api_key: &str, cse_id: &str) -> Result<Vec<SearchResult>> {
    let resp = client
        .get("https://www.googleapis.com/customsearch/v1")
        .query(&[
            ("key", api_key),
            ("cx", cse_id),
            ("q", query),
            ("num", &crate::constants::SEARCH_MAX_RESULTS.to_string()),
        ])
        .send()
        .await
        .context("Google search request failed")?;
    let body: serde_json::Value = resp.json().await.context("failed to parse Google search response")?;
    let mut results = Vec::new();
    if let Some(items) = body["items"].as_array() {
        for item in items {
            let title = item["title"].as_str().unwrap_or("").to_string();
            let snippet = item["snippet"].as_str().unwrap_or("").to_string();
            let url = item["link"].as_str().unwrap_or("").to_string();
            if !title.is_empty() {
                results.push(SearchResult { title, snippet, url });
            }
        }
    }
    Ok(results)
}

async fn search_bing(client: &Client, query: &str, api_key: &str) -> Result<Vec<SearchResult>> {
    let resp = client
        .get("https://api.bing.microsoft.com/v7.0/search")
        .header("Ocp-Apim-Subscription-Key", api_key)
        .query(&[
            ("q", query),
            ("count", &crate::constants::SEARCH_MAX_RESULTS.to_string()),
        ])
        .send()
        .await
        .context("Bing search request failed")?;
    let body: serde_json::Value = resp.json().await.context("failed to parse Bing search response")?;
    let mut results = Vec::new();
    if let Some(web_pages) = body["webPages"]["value"].as_array() {
        for item in web_pages {
            let title = item["name"].as_str().unwrap_or("").to_string();
            let snippet = item["snippet"].as_str().unwrap_or("").to_string();
            let url = item["url"].as_str().unwrap_or("").to_string();
            if !title.is_empty() {
                results.push(SearchResult { title, snippet, url });
            }
        }
    }
    Ok(results)
}

async fn search_ddg(client: &Client, query: &str) -> Result<Vec<SearchResult>> {
    // Use DDG Lite — a simpler HTML endpoint that is much less likely to change
    // compared to the full html.duckduckgo.com/html/ page.
    let resp = client
        .post("https://lite.duckduckgo.com/lite/")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("User-Agent", "Mozilla/5.0 (compatible; rem-cli/0.4)")
        .body(format!("q={}", urlencoding_encode(query)))
        .send()
        .await
        .context("web search request failed")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("DuckDuckGo returned HTTP {}", resp.status()));
    }
    let html = resp.text().await.context("failed to read search response")?;
    Ok(parse_ddg_lite_html(&html))
}

/// RFC 3986 percent-encoding for search query strings.
/// Encodes all characters except unreserved ones (ALPHA, DIGIT, -, ., _, ~).
/// Uses application/x-www-form-urlencoded convention: space → +.
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push('+'),
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' => out.push(c),
            _ => {
                for b in c.to_string().bytes() {
                    out.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    out
}

/// Parses DDG Lite HTML responses.
/// DDG Lite has a simple table-based layout:
///   <tr class="result">
///     <td class="result-snippet">...</td>
///     <td><a rel="nofollow" href="...">title</a></td>
///   </tr>
fn parse_ddg_lite_html(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);

    static RESULT_ROW: std::sync::LazyLock<Option<Selector>> =
        std::sync::LazyLock::new(|| Selector::parse("tr.result").ok());
    static RESULT_LINK: std::sync::LazyLock<Option<Selector>> =
        std::sync::LazyLock::new(|| Selector::parse("a[rel='nofollow']").ok());
    static SNIPPET_CELL: std::sync::LazyLock<Option<Selector>> =
        std::sync::LazyLock::new(|| Selector::parse("td.result-snippet").ok());

    let row_sel = match RESULT_ROW.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let link_sel = match RESULT_LINK.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let snippet_sel = match SNIPPET_CELL.as_ref() {
        Some(s) => s,
        None => return Vec::new(),
    };

    let mut results = Vec::new();
    for row in document.select(row_sel).take(crate::constants::SEARCH_MAX_RESULTS) {
        let title = row
            .select(link_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let url = row
            .select(link_sel)
            .next()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("")
            .to_string();
        let snippet = row
            .select(snippet_sel)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        if !title.is_empty() || !snippet.is_empty() {
            results.push(SearchResult { title, snippet, url });
        }
    }
    results
}

/// Prints styled search results to the terminal.
pub fn print_search_results(results: &[SearchResult]) {
    let t = ui::theme::active();
    if results.is_empty() {
        println!("{}", ui::theme::paint_warning(&t, "  no results found"));
        return;
    }
    println!("{}", ui::theme::paint_rail_empty(&t));
    println!(
        "{} {}",
        ui::theme::paint(&t, "accent", "\u{258C}", true),
        ui::theme::paint_bright(&t, &format!("{} result(s) found", results.len())),
    );
    println!("{}", ui::theme::paint_rail_empty(&t));
    for (i, r) in results.iter().enumerate() {
        println!(
            "{} {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_bright(&t, &format!("{}. {}", i + 1, r.title))
        );
        println!(
            "{}   {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, &r.url)
        );
        if !r.snippet.is_empty() {
            println!("{}   {}", ui::theme::paint(&t, "accent", "\u{258C}", true), r.snippet);
        }
        println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
    }
}

/// Builds a SearchProvider from config fields.
pub(crate) fn provider_from_config(provider_name: &str, api_key: &str, search_cse_id: &str) -> Option<SearchProvider> {
    match provider_name.to_lowercase().as_str() {
        "google" if !api_key.is_empty() && !search_cse_id.is_empty() => Some(SearchProvider::Google {
            api_key: api_key.to_string(),
            cse_id: search_cse_id.to_string(),
        }),
        "bing" if !api_key.is_empty() => Some(SearchProvider::Bing {
            api_key: api_key.to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ddg_lite_html_returns_empty_for_empty_input() {
        let results = parse_ddg_lite_html("");
        assert!(results.is_empty());
    }

    #[test]
    fn parse_ddg_lite_html_returns_empty_for_no_matches() {
        let html = "<html><body>no results here</body></html>";
        let results = parse_ddg_lite_html(html);
        assert!(results.is_empty());
    }

    #[test]
    fn provider_from_config_ddg_returns_none() {
        assert!(provider_from_config("ddg", "", "").is_none());
    }

    #[test]
    fn provider_from_config_google_missing_key() {
        assert!(provider_from_config("google", "", "").is_none());
    }

    #[test]
    fn search_cache_inner_get_returns_none_for_missing() {
        let mut cache = SearchCacheInner {
            entries: HashMap::new(),
            order: Vec::new(),
        };
        assert!(cache.get("nonexistent").is_none());
    }

    #[test]
    fn search_cache_inner_insert_and_get() {
        let mut cache = SearchCacheInner {
            entries: HashMap::new(),
            order: Vec::new(),
        };
        let result = vec![SearchResult {
            title: "test".into(),
            snippet: "test".into(),
            url: "https://test.com".into(),
        }];
        cache.insert("key".to_string(), result.clone());
        let got = cache.get("key");
        assert!(got.is_some(), "should find cached entry");
        assert_eq!(got.unwrap().len(), 1);
    }

    #[test]
    fn search_cache_inner_evicts_oldest_when_full() {
        let mut cache = SearchCacheInner {
            entries: HashMap::new(),
            order: Vec::new(),
        };
        let result = vec![SearchResult {
            title: "test".into(),
            snippet: "test".into(),
            url: "https://test.com".into(),
        }];
        // Fill to max
        for i in 0..SEARCH_CACHE_MAX_ENTRIES {
            cache.insert(format!("key{}", i), result.clone());
        }
        // Insert one more
        cache.insert("overflow".to_string(), result);
        assert_eq!(cache.entries.len(), SEARCH_CACHE_MAX_ENTRIES);
        // "key0" should have been evicted (oldest)
        assert!(!cache.entries.contains_key("key0"), "oldest entry should be evicted");
        assert!(cache.entries.contains_key("overflow"), "new entry should be present");
    }

    #[test]
    fn purge_expired_entries_removes_stale_data() {
        let mut cache = SearchCacheInner {
            entries: HashMap::new(),
            order: Vec::new(),
        };
        let result = vec![SearchResult {
            title: "x".into(),
            snippet: "x".into(),
            url: "https://x.com".into(),
        }];
        // Insert a fresh entry
        cache.insert("fresh".to_string(), result.clone());
        // Insert an entry with expired timestamp
        cache.entries.insert(
            "stale".to_string(),
            CacheEntry {
                results: Arc::new(result),
                cached_at: Instant::now() - std::time::Duration::from_secs(SEARCH_CACHE_TTL_SECS + 1),
            },
        );
        cache.order.push("stale".to_string());

        cache.purge_expired();

        assert!(cache.entries.contains_key("fresh"), "fresh entry should remain");
        assert!(!cache.entries.contains_key("stale"), "stale entry should be purged");
    }
}
