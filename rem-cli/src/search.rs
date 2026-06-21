//! Web search with configurable backends (DuckDuckGo, Google, Bing).
//! When an API key is configured for Google or Bing, uses the proper search API;
//! otherwise falls back to DuckDuckGo HTML scraping.

use crate::ui;
use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use std::sync::LazyLock;

static RE_SEARCH_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="result__a"[^>]*href="([^"]*)"[^>]*>([^<]*)</a>"#).expect("invalid regex literal")
});
static RE_SEARCH_SNIPPET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="result__snippet"[^>]*>([^<]*(?:<[^/>][^>]*>[^<]*</[^>]+>)*[^<]*)</a>"#)
        .expect("invalid regex literal")
});

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

/// Performs a web search using the configured provider.
pub(crate) async fn perform_web_search(
    client: &Client,
    query: &str,
    provider: Option<&SearchProvider>,
) -> Result<Vec<SearchResult>> {
    match provider {
        Some(SearchProvider::Google { api_key, cse_id }) => search_google(client, query, api_key, cse_id).await,
        Some(SearchProvider::Bing { api_key }) => search_bing(client, query, api_key).await,
        _ => search_ddg(client, query).await,
    }
}

async fn search_google(client: &Client, query: &str, api_key: &str, cse_id: &str) -> Result<Vec<SearchResult>> {
    let resp = client
        .get("https://www.googleapis.com/customsearch/v1")
        .query(&[("key", api_key), ("cx", cse_id), ("q", query), ("num", "8")])
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
        .query(&[("q", query), ("count", "8")])
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
        .header("User-Agent", "rem-cli/0.2")
        .send()
        .await
        .context("web search request failed")?;
    let html = resp.text().await.context("failed to read search response")?;
    Ok(parse_ddg_html(&html))
}

fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut remaining = html;
    while results.len() < 8 {
        if let Some(cap) = RE_SEARCH_TITLE.captures(remaining) {
            let url = cap.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
            let title = cap.get(2).map(|m| strip_html(m.as_str())).unwrap_or_default();
            let snippet_pos = cap.get(0).map(|m| m.end()).unwrap_or(0);
            let after_title = &remaining[snippet_pos..];
            let snippet = RE_SEARCH_SNIPPET
                .captures(after_title)
                .and_then(|c| c.get(1))
                .map(|m| strip_html(m.as_str()).trim().to_string())
                .unwrap_or_default();
            if !title.is_empty() {
                results.push(SearchResult { title, snippet, url });
            }
            let advance = cap.get(0).map(|m| m.end()).unwrap_or(1);
            if advance >= remaining.len() {
                break;
            }
            remaining = &remaining[advance..];
        } else {
            break;
        }
    }
    results
}

fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                while let Some(&next) = chars.peek() {
                    if next == '>' {
                        chars.next();
                        break;
                    }
                    chars.next();
                }
            }
            '&' => {
                let mut entity = String::with_capacity(8);
                entity.push('&');
                for ch in chars.by_ref() {
                    entity.push(ch);
                    if ch == ';' {
                        break;
                    }
                }
                match entity.as_str() {
                    "&amp;" => out.push('&'),
                    "&lt;" => out.push('<'),
                    "&gt;" => out.push('>'),
                    "&quot;" => out.push('"'),
                    "&#x27;" => out.push('\''),
                    _ => out.push_str(&entity),
                }
            }
            _ => out.push(c),
        }
    }
    out.trim().to_string()
}

/// Prints styled search results to the terminal.
pub fn print_search_results(results: &[SearchResult]) {
    let t = ui::theme::active();
    if results.is_empty() {
        println!("{}", ui::theme::paint_warning(&t, "  no results found"));
        return;
    }
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
    fn strip_html_removes_tags() {
        assert_eq!(strip_html("<b>hello</b> world"), "hello world");
    }

    #[test]
    fn strip_html_decodes_entities() {
        assert_eq!(strip_html("&amp;lt;test&amp;gt;"), "&lt;test&gt;");
    }

    #[test]
    fn strip_html_handles_empty() {
        assert_eq!(strip_html(""), "");
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
