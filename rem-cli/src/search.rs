//! Web search with configurable backends (DuckDuckGo, Google, Bing).
//! When an API key is configured for Google or Bing, uses the proper search API;
//! otherwise falls back to DuckDuckGo HTML scraping.

use std::sync::LazyLock;

use crate::ui;
use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};

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
/// Tries the configured provider first, then falls back to DuckDuckGo on failure.
pub(crate) async fn perform_web_search(
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
    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "Mozilla/5.0 (compatible; rem-cli/0.4)")
        .send()
        .await
        .context("web search request failed")?;
    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("DuckDuckGo returned HTTP {}", resp.status()));
    }
    let html = resp.text().await.context("failed to read search response")?;
    Ok(parse_ddg_html(&html))
}

/// Simple percent-decoding for URL-encoded strings (no external dep needed).
fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                out.push(byte as char);
            } else {
                out.push('%');
                out.push_str(&hex);
            }
        } else if c == '+' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Extracts the actual destination URL from a DuckDuckGo redirect URL.
/// DDG wraps result links in `//duckduckgo.com/l/?uddg=<encoded_url>&rut=...`
fn resolve_ddg_url(href: &str) -> String {
    if href.contains("uddg=") {
        static RE_UDDG: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"[?&]uddg=([^&]+)").expect("invalid uddg regex"));
        if let Some(caps) = RE_UDDG.captures(href) {
            if let Some(encoded) = caps.get(1) {
                return url_decode(encoded.as_str());
            }
        }
    }
    href.to_string()
}

fn ddg_selectors() -> (&'static Selector, &'static Selector, &'static Selector) {
    static RESULT: LazyLock<Selector> = LazyLock::new(|| Selector::parse(".result").expect("invalid selector"));
    static TITLE: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse(".result__title a, .result__a").expect("invalid selector"));
    static SNIPPET: LazyLock<Selector> =
        LazyLock::new(|| Selector::parse(".result__snippet").expect("invalid selector"));
    (&RESULT, &TITLE, &SNIPPET)
}

fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let document = Html::parse_document(html);

    let (result_selector, title_selector, snippet_selector) = ddg_selectors();
    let result_selector = Some(result_selector);
    let title_selector = Some(title_selector);
    let snippet_selector = Some(snippet_selector);

    let result_elements = match result_selector {
        Some(sel) => document.select(sel).collect::<Vec<_>>(),
        None => return Vec::new(),
    };

    let mut results = Vec::new();
    for element in result_elements.iter().take(crate::constants::SEARCH_MAX_RESULTS) {
        let title = title_selector
            .as_ref()
            .and_then(|sel| element.select(sel).next())
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let url = title_selector
            .as_ref()
            .and_then(|sel| element.select(sel).next())
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("")
            .to_string();
        let url = resolve_ddg_url(&url);

        let snippet = snippet_selector
            .as_ref()
            .and_then(|sel| element.select(sel).next())
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
    fn parse_ddg_html_returns_empty_for_empty_input() {
        let results = parse_ddg_html("");
        assert!(results.is_empty());
    }

    #[test]
    fn parse_ddg_html_returns_empty_for_no_matches() {
        let html = "<html><body>no results here</body></html>";
        let results = parse_ddg_html(html);
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
