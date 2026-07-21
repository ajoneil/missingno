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
        .header(
            "User-Agent",
            concat!("missingno-curator/", env!("CARGO_PKG_VERSION")),
        )
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
    /// The signature database's per-dump name — the evidence string, carrying
    /// the TOSEC bracket flags that identify hacks/trained/bad dumps.
    pub signature_name: Option<String>,
    pub cover_url: Option<String>,
    pub wikipedia_url: Option<String>,
}

/// What a TOSEC bracket flag says about a dump. Derived works were made by
/// someone (mods, with an author and a name); defective dumps were made by
/// nobody (a dumper did it wrong) and must never be filed as mods.
/// `[a]` (alternate) and `[!]` (verified) are deliberately absent: an
/// alternate dump still belongs to the game. Longest flag first, so a
/// translation is not misread as trained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigFlag {
    Derived(&'static str),
    Defective(&'static str),
}

const SIG_FLAGS: [(&str, SigFlag); 6] = [
    ("[tr", SigFlag::Derived("translation")),
    ("[cr", SigFlag::Derived("cracked")),
    ("[h", SigFlag::Derived("hack")),
    ("[t", SigFlag::Derived("trained")),
    ("[b", SigFlag::Defective("bad dump")),
    ("[o", SigFlag::Defective("overdump")),
];

pub fn classify_signature(signature: &str) -> Option<SigFlag> {
    let lower = signature.to_lowercase();
    SIG_FLAGS
        .iter()
        .find(|(flag, _)| lower.contains(flag))
        .map(|(_, class)| *class)
}

/// One dump's signature-database answer.
#[derive(Clone, Debug)]
pub enum SigResult {
    Found {
        signature: Option<String>,
        game: String,
    },
    Unknown,
    Failed(String),
}

/// Sequential, politely-spaced lookups — one entry's dumps, never a sweep.
pub fn lookup_signatures(sha1s: Vec<String>) -> Vec<(String, SigResult)> {
    let mut results = Vec::new();
    for (i, sha1) in sha1s.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_millis(750));
        }
        let outcome = match hasheous_lookup(sha1) {
            Ok(Some(hit)) => SigResult::Found {
                signature: hit.signature_name,
                game: hit.name,
            },
            Ok(None) => SigResult::Unknown,
            Err(e) => SigResult::Failed(e),
        };
        results.push((sha1.clone(), outcome));
    }
    results
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
    hit.signature_name = body["signature"]["rom"]["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
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

/// Stretch a square-pixel RGBA buffer to the display's pixel aspect (nearest).
pub fn aspect_corrected(
    width: u32,
    height: u32,
    aspect: f32,
    pixels: &[u8],
) -> (u32, u32, Vec<u8>) {
    if (aspect - 1.0).abs() < 0.01 {
        return (width, height, pixels.to_vec());
    }
    let out_width = ((width as f32) * aspect).round().max(1.0) as u32;
    let mut out = Vec::with_capacity((out_width * height * 4) as usize);
    for y in 0..height {
        let row = &pixels[(y * width * 4) as usize..((y + 1) * width * 4) as usize];
        for x_out in 0..out_width {
            let x_src = ((x_out as f32 / aspect) as u32).min(width - 1) as usize;
            out.extend_from_slice(&row[x_src * 4..x_src * 4 + 4]);
        }
    }
    (out_width, height, out)
}

pub fn boot_check(
    filename_hint: &str,
    rom: &[u8],
    tv_standard: Option<String>,
    cart_type: Option<String>,
    frames: usize,
) -> Result<BootShot, String> {
    let options = missingno_session::factory::LoadOptions {
        tv_standard,
        boot_rom: None,
        cart_type,
    };
    let path = Path::new(filename_hint);
    let mut console = missingno_session::factory::create_console_with(path, rom, &options)
        .map_err(|e| format!("core rejected ROM: {e}"))?
        .ok_or("no core recognizes this ROM")?;
    let pixel_aspect = console.video_out().pixel_aspect();
    let mut frames_seen = 0;
    for _ in 0..frames {
        if console.step_frame().display.is_some() {
            frames_seen += 1;
        }
    }
    let frame = console.screen_display().resolve_rgba();
    let (width, height, rgba) =
        aspect_corrected(frame.width, frame.height, pixel_aspect, &frame.pixels);
    Ok(BootShot {
        frames_run: frames,
        frames_seen,
        width,
        height,
        rgba,
    })
}

/// Facts a Game Boy family header states about its cartridge.
#[derive(Clone, Debug)]
pub struct GbHeader {
    /// 0x80 = dual-mode (CGB enhanced), 0xC0 = CGB only.
    pub cgb_flag: u8,
    pub sgb: bool,
    pub mapper: String,
}

pub fn gb_header(rom: &[u8]) -> Option<GbHeader> {
    if rom.len() < 0x150 {
        return None;
    }
    let mapper = match rom[0x147] {
        0x00 => "ROM ONLY",
        0x01 => "MBC1",
        0x02 => "MBC1+RAM",
        0x03 => "MBC1+RAM+BATTERY",
        0x05 => "MBC2",
        0x06 => "MBC2+BATTERY",
        0x08 => "ROM+RAM",
        0x09 => "ROM+RAM+BATTERY",
        0x0B => "MMM01",
        0x0C => "MMM01+RAM",
        0x0D => "MMM01+RAM+BATTERY",
        0x0F => "MBC3+TIMER+BATTERY",
        0x10 => "MBC3+TIMER+RAM+BATTERY",
        0x11 => "MBC3",
        0x12 => "MBC3+RAM",
        0x13 => "MBC3+RAM+BATTERY",
        0x19 => "MBC5",
        0x1A => "MBC5+RAM",
        0x1B => "MBC5+RAM+BATTERY",
        0x1C => "MBC5+RUMBLE",
        0x1D => "MBC5+RUMBLE+RAM",
        0x1E => "MBC5+RUMBLE+RAM+BATTERY",
        0x20 => "MBC6",
        0x22 => "MBC7+SENSOR+RUMBLE+RAM+BATTERY",
        0xFC => "POCKET CAMERA",
        0xFD => "BANDAI TAMA5",
        0xFE => "HuC3",
        0xFF => "HuC1+RAM+BATTERY",
        other => {
            return Some(GbHeader {
                cgb_flag: rom[0x143],
                sgb: rom[0x146] == 0x03 && rom[0x14B] == 0x33,
                mapper: format!("Unknown (0x{other:02X})"),
            });
        }
    }
    .to_owned();
    Some(GbHeader {
        cgb_flag: rom[0x143],
        sgb: rom[0x146] == 0x03 && rom[0x14B] == 0x33,
        mapper,
    })
}

#[cfg(test)]
mod tests {
    use super::{SigFlag, classify_signature};

    #[test]
    fn flags_classify_and_translation_is_not_trained() {
        assert_eq!(
            classify_signature("Adventure SI (2003)(Channel2)(NTSC)[h Color Scrolling]"),
            Some(SigFlag::Derived("hack"))
        );
        assert_eq!(
            classify_signature("Game (1983)[tr de]"),
            Some(SigFlag::Derived("translation"))
        );
        assert_eq!(
            classify_signature("Game (1983)[t +5]"),
            Some(SigFlag::Derived("trained"))
        );
        assert_eq!(
            classify_signature("Pitfall! (1982) (Activision) [o1]"),
            Some(SigFlag::Defective("overdump"))
        );
        assert_eq!(
            classify_signature("Game (1983)[b]"),
            Some(SigFlag::Defective("bad dump"))
        );
        assert_eq!(classify_signature("Adventure (1978)(Atari)(NTSC)"), None);
        assert_eq!(classify_signature("Game (1983)[a2]"), None);
        assert_eq!(classify_signature("Game (1983)[!]"), None);
    }
}
