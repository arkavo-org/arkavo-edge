use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A single search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Cached search results with timestamp
struct CachedResults {
    results: Vec<SearchResult>,
    timestamp: Instant,
}

/// Web search tool using DuckDuckGo HTML endpoint
pub struct WebSearchTool {
    schema: ToolSchema,
    cache: Mutex<HashMap<String, CachedResults>>,
    rate_limit: Mutex<RateLimiter>,
}

/// Simple rate limiter
struct RateLimiter {
    request_count: u32,
    window_start: Instant,
    max_per_minute: u32,
}

impl RateLimiter {
    fn new(max_per_minute: u32) -> Self {
        Self {
            request_count: 0,
            window_start: Instant::now(),
            max_per_minute,
        }
    }

    fn check(&mut self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_start) > Duration::from_secs(60) {
            self.window_start = now;
            self.request_count = 0;
        }

        if self.request_count >= self.max_per_minute {
            return false;
        }

        self.request_count += 1;
        true
    }
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "web_search".to_string(),
                aliases: Some(vec![
                    "search".to_string(),
                    "lookup".to_string(),
                    "docs".to_string(),
                ]),
                description: "Search the web for documentation and information using DuckDuckGo. \
                    Results are cached for 15 minutes to reduce API calls."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum results to return (default: 5, max: 10)"
                        },
                        "site_filter": {
                            "type": "string",
                            "description": "Limit search to specific domain (e.g., 'docs.rs', 'stackoverflow.com')"
                        }
                    },
                    "required": ["query"]
                }),
            },
            cache: Mutex::new(HashMap::new()),
            rate_limit: Mutex::new(RateLimiter::new(30)),
        }
    }

    /// Build the search URL with query and optional site filter
    fn build_url(&self, query: &str, site_filter: Option<&str>) -> String {
        let full_query = if let Some(site) = site_filter {
            format!("site:{} {}", site, query)
        } else {
            query.to_string()
        };

        format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding::encode(&full_query)
        )
    }

    /// Parse search results from DuckDuckGo HTML response
    fn parse_results(&self, html: &str, max_results: usize) -> Vec<SearchResult> {
        let mut results = Vec::new();

        // DuckDuckGo HTML structure:
        // <a class="result__a" href="...">Title</a>
        // <a class="result__snippet">Snippet text...</a>

        // Simple HTML parsing without external dependency
        let mut pos = 0;
        while results.len() < max_results {
            // Find result link
            let link_start = match html[pos..].find("class=\"result__a\"") {
                Some(p) => pos + p,
                None => break,
            };

            // Find href
            let href_search_start = link_start.saturating_sub(20);
            let href_start = match html[href_search_start..link_start].rfind("href=\"") {
                Some(p) => href_search_start + p + 6,
                None => {
                    pos = link_start + 1;
                    continue;
                }
            };
            let href_end = match html[href_start..].find('"') {
                Some(p) => href_start + p,
                None => {
                    pos = link_start + 1;
                    continue;
                }
            };
            let url = &html[href_start..href_end];

            // Skip DuckDuckGo redirect URLs and extract actual URL
            let actual_url = if url.contains("uddg=") {
                // Extract URL from DuckDuckGo redirect
                url.split("uddg=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .map(|s| urlencoding::decode(s).unwrap_or_default().to_string())
                    .unwrap_or_else(|| url.to_string())
            } else {
                url.to_string()
            };

            // Find title (text between > and </a>)
            let title_start = match html[link_start..].find('>') {
                Some(p) => link_start + p + 1,
                None => {
                    pos = link_start + 1;
                    continue;
                }
            };
            let title_end = match html[title_start..].find("</a>") {
                Some(p) => title_start + p,
                None => {
                    pos = link_start + 1;
                    continue;
                }
            };
            let title = Self::strip_html_tags(&html[title_start..title_end]);

            // Find snippet
            let snippet_start = match html[title_end..].find("class=\"result__snippet\"") {
                Some(p) => {
                    let s = title_end + p;
                    match html[s..].find('>') {
                        Some(p2) => s + p2 + 1,
                        None => {
                            pos = title_end + 1;
                            continue;
                        }
                    }
                }
                None => {
                    pos = title_end + 1;
                    // Still add result with empty snippet
                    if !title.is_empty() && !actual_url.is_empty() {
                        results.push(SearchResult {
                            title: title.trim().to_string(),
                            url: actual_url,
                            snippet: String::new(),
                        });
                    }
                    continue;
                }
            };
            let snippet_end = match html[snippet_start..].find("</a>") {
                Some(p) => snippet_start + p,
                None => snippet_start + 200.min(html.len() - snippet_start),
            };
            let snippet = Self::strip_html_tags(&html[snippet_start..snippet_end]);

            if !title.is_empty() && !actual_url.is_empty() {
                results.push(SearchResult {
                    title: title.trim().to_string(),
                    url: actual_url,
                    snippet: snippet.trim().to_string(),
                });
            }

            pos = snippet_end;
        }

        results
    }

    /// Strip HTML tags from text
    fn strip_html_tags(html: &str) -> String {
        let mut result = String::new();
        let mut in_tag = false;

        for c in html.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => result.push(c),
                _ => {}
            }
        }

        // Decode common HTML entities
        result
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#x27;", "'")
            .replace("&apos;", "'")
            .replace("&nbsp;", " ")
    }

    /// Check cache for existing results
    fn get_cached(&self, cache_key: &str) -> Option<Vec<SearchResult>> {
        let cache = self.cache.lock().ok()?;
        let cached = cache.get(cache_key)?;

        // 15-minute TTL
        let results = if cached.timestamp.elapsed() < Duration::from_secs(15 * 60) {
            Some(cached.results.clone())
        } else {
            None
        };
        drop(cache);
        results
    }

    /// Store results in cache
    fn set_cached(&self, cache_key: String, results: Vec<SearchResult>) {
        if let Ok(mut cache) = self.cache.lock() {
            // Limit cache size to 100 entries
            if cache.len() >= 100 {
                // Remove oldest entries
                let oldest: Vec<String> = cache
                    .iter()
                    .filter(|(_, v)| v.timestamp.elapsed() > Duration::from_secs(10 * 60))
                    .map(|(k, _)| k.clone())
                    .collect();
                for key in oldest {
                    cache.remove(&key);
                }
            }

            cache.insert(
                cache_key,
                CachedResults {
                    results,
                    timestamp: Instant::now(),
                },
            );
        }
    }

    /// Perform the actual web search
    async fn search(
        &self,
        query: &str,
        site_filter: Option<&str>,
        max_results: usize,
    ) -> Result<(Vec<SearchResult>, bool)> {
        let cache_key = format!("{}:{}", site_filter.unwrap_or(""), query);

        // Check cache first
        if let Some(cached) = self.get_cached(&cache_key) {
            return Ok((cached.into_iter().take(max_results).collect(), true));
        }

        // Check rate limit
        {
            let mut rate_limit = self
                .rate_limit
                .lock()
                .map_err(|_| ToolError::Mcp("Rate limiter lock poisoned".to_string()))?;
            if !rate_limit.check() {
                return Err(ToolError::Mcp(
                    "Rate limit exceeded (30 requests/minute). Try again later.".to_string(),
                ));
            }
        }

        // Build URL and fetch
        let url = self.build_url(query, site_filter);

        // Use a simple HTTP client approach with tokio
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("Arkavo-Agent/1.0 (https://arkavo.com)")
            .build()
            .map_err(|e| ToolError::Mcp(format!("Failed to create HTTP client: {}", e)))?;

        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|e| ToolError::Mcp(format!("Search request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(ToolError::Mcp(format!(
                "Search returned status: {}",
                response.status()
            )));
        }

        let html = response
            .text()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to read response: {}", e)))?;

        // Parse results
        let results = self.parse_results(&html, max_results.max(10)); // Parse up to 10, return max_results

        // Cache results
        self.set_cached(cache_key, results.clone());

        Ok((results.into_iter().take(max_results).collect(), false))
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParams("Missing required 'query' parameter".to_string())
            })?;

        if query.trim().is_empty() {
            return Err(ToolError::InvalidParams(
                "Query cannot be empty".to_string(),
            ));
        }

        let max_results = params
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(5)
            .min(10);

        let site_filter = params.get("site_filter").and_then(|v| v.as_str());

        let (results, cached) = self.search(query, site_filter, max_results).await?;

        Ok(json!({
            "results": results,
            "query": query,
            "cached": cached,
            "result_count": results.len(),
            "site_filter": site_filter
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_schema() {
        let tool = WebSearchTool::new();
        let schema = tool.schema();

        assert_eq!(schema.name, "web_search");
        assert!(
            schema
                .aliases
                .as_ref()
                .unwrap()
                .contains(&"search".to_string())
        );
        assert!(schema.description.contains("DuckDuckGo"));
    }

    #[test]
    fn test_build_url() {
        let tool = WebSearchTool::new();

        let url = tool.build_url("rust async", None);
        assert!(url.contains("q=rust%20async"));

        let url_with_site = tool.build_url("tokio", Some("docs.rs"));
        assert!(url_with_site.contains("site%3Adocs.rs"));
        assert!(url_with_site.contains("tokio"));
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(
            WebSearchTool::strip_html_tags("<b>Hello</b> <i>World</i>"),
            "Hello World"
        );
        assert_eq!(WebSearchTool::strip_html_tags("&amp; &lt; &gt;"), "& < >");
        assert_eq!(
            WebSearchTool::strip_html_tags("No tags here"),
            "No tags here"
        );
    }

    #[test]
    fn test_parse_results_empty() {
        let tool = WebSearchTool::new();
        let results = tool.parse_results("<html><body>No results</body></html>", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rate_limiter() {
        let mut limiter = RateLimiter::new(3);

        assert!(limiter.check()); // 1
        assert!(limiter.check()); // 2
        assert!(limiter.check()); // 3
        assert!(!limiter.check()); // Should be blocked
    }

    #[test]
    fn test_cache() {
        let tool = WebSearchTool::new();

        // Initially empty
        assert!(tool.get_cached("test:query").is_none());

        // Add to cache
        tool.set_cached(
            "test:query".to_string(),
            vec![SearchResult {
                title: "Test".to_string(),
                url: "https://example.com".to_string(),
                snippet: "Test snippet".to_string(),
            }],
        );

        // Should be cached
        let cached = tool.get_cached("test:query");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_execute_empty_query() {
        let tool = WebSearchTool::new();
        let params = json!({
            "query": ""
        });

        let result = tool.execute(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_missing_query() {
        let tool = WebSearchTool::new();
        let params = json!({});

        let result = tool.execute(params).await;
        assert!(result.is_err());
    }
}
