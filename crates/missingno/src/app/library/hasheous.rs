use sha1::{Digest, Sha1};

const BASE_URL: &str = "https://hasheous.org/api/v1";

#[derive(Debug, Clone)]
pub struct GameInfo {
    pub name: String,
    pub platform: Option<String>,
    pub description: Option<String>,
    pub cover_art: Option<Vec<u8>>,
    pub year: Option<String>,
    pub publisher: Option<String>,
    pub wikipedia_url: Option<String>,
    pub igdb_url: Option<String>,
}

pub fn rom_sha1(rom: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(rom);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>()
}

pub fn lookup(sha1: &str) -> Result<Option<GameInfo>, String> {
    let url = format!("{BASE_URL}/Lookup/ByHash/sha1/{sha1}");

    let response = match ureq::get(&url)
        .header("Accept", "application/json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(format!("Hasheous request failed: {e}")),
    };

    let body_str = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("Failed to read Hasheous response: {e}"))?;

    let body: serde_json::Value = serde_json::from_str(&body_str)
        .map_err(|e| format!("Failed to parse Hasheous response: {e}"))?;

    let name = body["name"].as_str().unwrap_or_default().to_string();
    if name.is_empty() {
        return Ok(None);
    }

    let platform = body["platform"]["name"].as_str().map(String::from);
    let publisher = body["publisher"]["name"].as_str().map(String::from);

    let mut logo_hash = None;

    if let Some(attributes) = body["attributes"].as_array() {
        for attr in attributes {
            let attr_name = attr["attributeName"].as_str().unwrap_or_default();
            let attr_type = attr["attributeType"].as_str().unwrap_or_default();

            if attr_name == "Logo" && attr_type == "ImageId" {
                logo_hash = attr["value"].as_str().map(String::from);
            }
        }
    }

    // Year from signature (singular)
    let year = body["signature"]["game"]["year"]
        .as_str()
        .filter(|y| !y.is_empty())
        .map(String::from);

    // Extract links from metadata
    let mut wikipedia_url = None;
    let mut igdb_url = None;
    if let Some(metadata) = body["metadata"].as_array() {
        for entry in metadata {
            if entry["status"].as_str() != Some("Mapped") {
                continue;
            }
            let link = entry["link"].as_str().filter(|l| !l.is_empty());
            match entry["source"].as_str() {
                Some("Wikipedia") => wikipedia_url = link.map(String::from),
                Some("IGDB") => igdb_url = link.map(String::from),
                _ => {}
            }
        }
    }

    // Fetch cover art if available
    let cover_art = logo_hash.and_then(|hash| {
        let url = format!("{BASE_URL}/images/{hash}");
        eprintln!("[hasheous] Fetching image: {url}");
        let result = fetch_image(&url);
        eprintln!(
            "[hasheous] Image: {} bytes",
            result.as_ref().map(|b| b.len()).unwrap_or(0)
        );
        result
    });

    Ok(Some(GameInfo {
        name,
        platform,
        description: None,
        cover_art,
        year,
        publisher,
        wikipedia_url,
        igdb_url,
    }))
}

fn fetch_image(url: &str) -> Option<Vec<u8>> {
    let response = ureq::get(url).call().ok()?;
    response.into_body().read_to_vec().ok()
}
