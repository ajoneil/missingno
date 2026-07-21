//! Verification: fetch ROMs from sources, hash them, match against artifacts
//! and the user's local collection.

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
};

use sha1::{Digest, Sha1};

pub fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    response
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("read failed: {e}"))
}

const ROM_EXTENSIONS: [&str; 5] = ["gb", "gbc", "a26", "bin", "rom"];

const HASHEOUS: &str = "https://hasheous.org/api/v1";

/// What Hasheous knows about a dump: a hosted cover URL and mapped links.
#[derive(Clone, Debug, Default)]
pub struct HasheousHit {
    pub name: String,
    pub cover_url: Option<String>,
    pub wikipedia_url: Option<String>,
}

pub fn hasheous_lookup(sha1: &str) -> Result<Option<HasheousHit>, String> {
    let url = format!("{HASHEOUS}/Lookup/ByHash/sha1/{sha1}");
    let response = match ureq::get(&url)
        .header(
            "User-Agent",
            concat!("missingno-curator/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/json")
        .call()
    {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(404)) => return Ok(None),
        Err(e) => return Err(format!("Hasheous request failed: {e}")),
    };
    let text = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("read failed: {e}"))?;
    let body: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse failed: {e}"))?;
    let name = body["name"].as_str().unwrap_or_default().to_owned();
    if name.is_empty() {
        return Ok(None);
    }
    let mut hit = HasheousHit {
        name,
        ..Default::default()
    };
    if let Some(attributes) = body["attributes"].as_array() {
        for attr in attributes {
            if attr["attributeName"].as_str() == Some("Logo")
                && attr["attributeType"].as_str() == Some("ImageId")
                && let Some(hash) = attr["value"].as_str()
            {
                hit.cover_url = Some(format!("{HASHEOUS}/images/{hash}"));
            }
        }
    }
    if let Some(metadata) = body["metadata"].as_array() {
        for entry in metadata {
            if entry["status"].as_str() == Some("Mapped")
                && entry["source"].as_str() == Some("Wikipedia")
                && let Some(link) = entry["link"].as_str().filter(|l| !l.is_empty())
            {
                hit.wikipedia_url = Some(link.to_owned());
            }
        }
    }
    Ok(Some(hit))
}

/// sha1 → path index over the user's ROM directory.
#[derive(Default, Debug)]
pub struct RomIndex {
    pub by_sha1: HashMap<String, PathBuf>,
    pub scanned: usize,
}

impl RomIndex {
    pub fn scan(dir: &Path) -> io::Result<Self> {
        let mut index = Self::default();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase());
                if !extension.is_some_and(|e| ROM_EXTENSIONS.contains(&e.as_str())) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                index.by_sha1.insert(sha1_hex(&bytes), path);
                index.scanned += 1;
            }
        }
        Ok(index)
    }
}

/// Synchronous smoke-test: recognize → construct → produce frames → screenshot.
pub struct BootShot {
    pub frames_run: usize,
    pub frames_seen: usize,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn boot_check(
    filename_hint: &str,
    rom: &[u8],
    tv_standard: Option<String>,
    frames: usize,
) -> Result<BootShot, String> {
    let options = missingno_session::factory::LoadOptions {
        tv_standard,
        boot_rom: None,
    };
    let path = Path::new(filename_hint);
    let mut console = missingno_session::factory::create_console_with(path, rom, &options)
        .map_err(|e| format!("core rejected ROM: {e}"))?
        .ok_or("no core recognizes this ROM")?;
    let mut frames_seen = 0;
    for _ in 0..frames {
        if console.step_frame().display.is_some() {
            frames_seen += 1;
        }
    }
    let frame = console.screen_display().resolve_rgba();
    Ok(BootShot {
        frames_run: frames,
        frames_seen,
        width: frame.width,
        height: frame.height,
        rgba: frame.pixels.to_vec(),
    })
}
