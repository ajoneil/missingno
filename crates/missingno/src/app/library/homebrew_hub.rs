//! Client for the gbdev Homebrew Hub API (https://hh.gbdev.io).
//!
//! Self-contained — no UI dependencies. Designed to be called from
//! background threads via `smol::unblock`.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::Instant,
};

use serde::Deserialize;

const API_BASE: &str = "https://hh3.gbdev.io/api";
const USER_AGENT: &str = concat!("missingno/", env!("CARGO_PKG_VERSION"));
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com/gbdev/database/master/entries";

/// How long cached search results stay valid.
const CACHE_TTL_SECS: u64 = 300; // 5 minutes

// ── Public types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResults {
    pub results: usize,
    pub page_total: usize,
    pub page_current: usize,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub developer: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub typetag: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub screenshots: Vec<String>,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(rename = "gameWebsite", default)]
    pub game_website: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
}

impl Entry {
    /// URL for a screenshot image.
    pub fn screenshot_url(&self, filename: &str) -> String {
        format!("{GITHUB_RAW_BASE}/{}/{filename}", self.slug)
    }

    /// URL for the cover image (if "cover.png" is in screenshots).
    pub fn cover_url(&self) -> Option<String> {
        if self.screenshots.iter().any(|s| s == "cover.png") {
            Some(self.screenshot_url("cover.png"))
        } else {
            self.screenshots.first().map(|s| self.screenshot_url(s))
        }
    }

    /// The primary playable ROM file, if any.
    pub fn playable_file(&self) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.playable.unwrap_or(false))
            .or_else(|| self.files.first())
    }

    /// URL to download a file.
    pub fn file_url(&self, filename: &str) -> String {
        format!("{GITHUB_RAW_BASE}/{}/{filename}", self.slug)
    }

    /// The game's website (tries gameWebsite first, then website).
    pub fn url(&self) -> Option<&str> {
        self.game_website
            .as_deref()
            .or(self.website.as_deref())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileEntry {
    pub filename: String,
    #[serde(default)]
    pub playable: Option<bool>,
    #[serde(default)]
    pub default: Option<bool>,
}

// ── Search parameters ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchQuery {
    pub platform: Option<String>,
    pub typetag: Option<String>,
    pub title: Option<String>,
    pub tags: Option<String>,
    pub page: usize,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            platform: None,
            typetag: None,
            title: None,
            tags: None,
            page: 1,
        }
    }
}

impl SearchQuery {
    pub fn gb() -> Self {
        Self {
            platform: Some("GB".to_string()),
            ..Default::default()
        }
    }

    pub fn gbc() -> Self {
        Self {
            platform: Some("GBC".to_string()),
            ..Default::default()
        }
    }

    pub fn with_title(mut self, title: &str) -> Self {
        if title.is_empty() {
            self.title = None;
        } else {
            self.title = Some(title.to_string());
        }
        self
    }

    pub fn with_typetag(mut self, typetag: &str) -> Self {
        self.typetag = Some(typetag.to_string());
        self
    }

    pub fn with_page(mut self, page: usize) -> Self {
        self.page = page;
        self
    }
}

// ── Client ────────────────────────────────────────────────────────────

/// Homebrew Hub API client with in-memory cache.
pub struct HomebrewHubClient {
    cache: Mutex<HashMap<SearchQuery, CachedResult>>,
}

struct CachedResult {
    results: SearchResults,
    fetched_at: Instant,
}

impl HomebrewHubClient {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Search the Homebrew Hub. Returns cached results if fresh enough.
    /// Safe to call from a background thread.
    pub fn search(&self, query: &SearchQuery) -> Result<SearchResults, String> {
        // Check cache
        if let Some(cached) = self.get_cached(query) {
            return Ok(cached);
        }

        // Build URL
        let mut url = format!("{API_BASE}/search?page={}&page_elements=10", query.page);
        if let Some(platform) = &query.platform {
            url.push_str(&format!("&platform={platform}"));
        }
        if let Some(typetag) = &query.typetag {
            url.push_str(&format!("&typetag={typetag}"));
        }
        if let Some(title) = &query.title {
            url.push_str(&format!("&title={}", urlencoding(title)));
        }
        if let Some(tags) = &query.tags {
            url.push_str(&format!("&tags={}", urlencoding(tags)));
        }
        url.push_str("&order_by=title&sort=asc");

        let response = ureq::get(&url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| format!("Homebrew Hub request failed: {e}"))?;

        let body = response
            .into_body()
            .read_to_string()
            .map_err(|e| format!("Failed to read response: {e}"))?;

        let results: SearchResults =
            serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {e}"))?;

        // Cache the result
        self.put_cached(query.clone(), &results);

        Ok(results)
    }

    /// Fetch a single entry by slug.
    pub fn entry(&self, slug: &str) -> Result<Entry, String> {
        let url = format!("{API_BASE}/entry/{slug}.json");

        let response = ureq::get(&url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/json")
            .call()
            .map_err(|e| format!("Homebrew Hub request failed: {e}"))?;

        let body = response
            .into_body()
            .read_to_string()
            .map_err(|e| format!("Failed to read response: {e}"))?;

        serde_json::from_str(&body).map_err(|e| format!("Failed to parse entry: {e}"))
    }

    /// Download a ROM file. Returns the raw bytes.
    pub fn download_rom(&self, entry: &Entry) -> Result<Vec<u8>, String> {
        let file = entry
            .playable_file()
            .ok_or_else(|| "No playable file found".to_string())?;

        let url = entry.file_url(&file.filename);

        let response = ureq::get(&url)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("Download failed: {e}"))?;

        response
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("Failed to read ROM data: {e}"))
    }

    /// Download a cover/screenshot image. Returns the raw bytes.
    pub fn download_image(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(url)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("Image download failed: {e}"))?;

        response
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("Failed to read image data: {e}"))
    }

    fn get_cached(&self, query: &SearchQuery) -> Option<SearchResults> {
        let cache = self.cache.lock().ok()?;
        let entry = cache.get(query)?;
        if entry.fetched_at.elapsed().as_secs() < CACHE_TTL_SECS {
            Some(entry.results.clone())
        } else {
            None
        }
    }

    fn put_cached(&self, query: SearchQuery, results: &SearchResults) {
        if let Ok(mut cache) = self.cache.lock() {
            // Evict old entries to prevent unbounded growth
            if cache.len() > 100 {
                cache.retain(|_, v| v.fetched_at.elapsed().as_secs() < CACHE_TTL_SECS);
            }
            cache.insert(
                query,
                CachedResult {
                    results: results.clone(),
                    fetched_at: Instant::now(),
                },
            );
        }
    }
}

/// Minimal URL encoding for query parameters.
fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encoding() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("test&foo=bar"), "test%26foo%3Dbar");
    }

    #[test]
    fn search_query_default() {
        let q = SearchQuery::default();
        assert_eq!(q.page, 1);
        assert!(q.platform.is_none());
    }
}
