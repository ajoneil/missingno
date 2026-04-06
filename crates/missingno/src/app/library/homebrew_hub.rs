//! HTTP client for downloading homebrew assets (cover images, ROMs)
//! from GitHub. Used by the homebrew browser and download handler.

const USER_AGENT: &str = concat!("missingno/", env!("CARGO_PKG_VERSION"));

/// Simple HTTP client for downloading files.
pub struct HomebrewHubClient;

impl HomebrewHubClient {
    pub fn new() -> Self {
        Self
    }

    /// Download a file by URL. Returns the raw bytes.
    pub fn download_image(&self, url: &str) -> Result<Vec<u8>, String> {
        let response = ureq::get(url)
            .header("User-Agent", USER_AGENT)
            .call()
            .map_err(|e| format!("Download failed: {e}"))?;

        response
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("Failed to read data: {e}"))
    }
}
