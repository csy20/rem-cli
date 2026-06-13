//! Web search via DuckDuckGo HTML API.
//! Performs searches by scraping DuckDuckGo's HTML results page and
//! parsing titles, snippets, and URLs from the response.

use crate::ui;
use anyhow::{Context, Result};
use regex::Regex;
use reqwest::Client;
use std::sync::LazyLock;

static RE_SEARCH_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="result__a"[^>]*href="([^"]*)"[^>]*>([^<]*)</a>"#)
        .expect("invalid regex literal")
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

/// Performs a web search via DuckDuckGo HTML API.
pub(crate) async fn perform_web_search(client: &Client, query: &str) -> Result<Vec<SearchResult>> {
    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header("User-Agent", "rem-cli/0.2")
        .send()
        .await
        .context("web search request failed")?;
    let html = resp
        .text()
        .await
        .context("failed to read search response")?;
    Ok(parse_ddg_html(&html))
}

fn parse_ddg_html(html: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut remaining = html;
    while results.len() < 8 {
        if let Some(cap) = RE_SEARCH_TITLE.captures(remaining) {
            let url = cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let title = cap
                .get(2)
                .map(|m| strip_html(m.as_str()))
                .unwrap_or_default();
            let snippet_pos = cap.get(0).map(|m| m.end()).unwrap_or(0);
            let after_title = &remaining[snippet_pos..];
            let snippet = RE_SEARCH_SNIPPET
                .captures(after_title)
                .and_then(|c| c.get(1))
                .map(|m| strip_html(m.as_str()).trim().to_string())
                .unwrap_or_default();
            if !title.is_empty() {
                results.push(SearchResult {
                    title,
                    snippet,
                    url,
                });
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
    let trimmed = out.trim().to_string();
    trimmed
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
            println!(
                "{}   {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                r.snippet
            );
        }
        println!("{}", ui::theme::paint(&t, "accent", "\u{258C}", true));
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
}
