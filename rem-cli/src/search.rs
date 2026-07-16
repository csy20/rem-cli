//! Web search with configurable backends (DuckDuckGo, Google, Bing).
//! When an API key is configured for Google or Bing, uses the proper search API;
//! otherwise falls back to DuckDuckGo HTML scraping.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::Instant;

use crate::ui;
use anyhow::{Context, Result};
use reqwest::Client;
use scraper::{Html, Selector};

/// Time-to-live for cached search results.
const SEARCH_CACHE_TTL_SECS: u64 = 300;

type SearchCache = HashMap<String, (Instant, Vec<SearchResult>)>;

/// In-memory cache for web search results keyed by normalized query.
/// Each entry stores the results and the time they were cached.
static SEARCH_CACHE: LazyLock<Mutex<SearchCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

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
        let cache = SEARCH_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_at, cached_results)) = cache.get(&cache_key) {
            if cached_at.elapsed().as_secs() < SEARCH_CACHE_TTL_SECS {
                return Ok(cached_results.clone());
            }
        }
    }

    // Perform the search
    let results = perform_web_search_uncached(client, query, provider).await?;

    // Store in cache
    if let Ok(mut cache) = SEARCH_CACHE.lock() {
        cache.insert(cache_key, (Instant::now(), results.clone()));
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

/// Minimal URL encoding for search queries (spaces → +, keep most chars).
fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push('+'),
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
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

    static RESULT_ROW: std::sync::LazyLock<Selector> =
        std::sync::LazyLock::new(|| Selector::parse("tr.result").expect("invalid selector"));
    static RESULT_LINK: std::sync::LazyLock<Selector> =
        std::sync::LazyLock::new(|| Selector::parse("a[rel='nofollow']").expect("invalid selector"));
    static SNIPPET_CELL: std::sync::LazyLock<Selector> =
        std::sync::LazyLock::new(|| Selector::parse("td.result-snippet").expect("invalid selector"));

    let mut results = Vec::new();
    for row in document.select(&RESULT_ROW).take(crate::constants::SEARCH_MAX_RESULTS) {
        let title = row
            .select(&RESULT_LINK)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let url = row
            .select(&RESULT_LINK)
            .next()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("")
            .to_string();
        let snippet = row
            .select(&SNIPPET_CELL)
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
}
