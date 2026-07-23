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

/// Which folder a scanned ROM file came from: the inbox is the to-curate
/// queue, the collection holds ROMs already curated on a previous pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RomHome {
    Inbox,
    Collection,
}

#[derive(Clone, Debug)]
pub struct ScannedRom {
    pub path: PathBuf,
    pub home: RomHome,
}

/// sha1 → file index over the inbox and collection directories.
#[derive(Default, Clone, Debug)]
pub struct RomIndex {
    pub by_sha1: HashMap<String, ScannedRom>,
    pub scanned: usize,
    /// Inbox files whose hash the collection already holds, set aside into
    /// `<inbox>/duplicates/` at scan time rather than deleted.
    pub duplicates_moved: usize,
}

impl RomIndex {
    /// Scan the collection first (its hashes are authoritative), then the
    /// inbox; an inbox file already in the collection moves to
    /// `<inbox>/duplicates/` so the inbox only ever holds new work.
    pub fn scan(inbox: Option<&Path>, collection: Option<&Path>) -> io::Result<Self> {
        let mut index = Self::default();
        if let Some(dir) = collection {
            index.scan_into(dir, RomHome::Collection, None)?;
        }
        if let Some(dir) = inbox {
            let duplicates = dir.join("duplicates");
            index.scan_into(dir, RomHome::Inbox, Some(&duplicates))?;
        }
        Ok(index)
    }

    fn scan_into(
        &mut self,
        dir: &Path,
        home: RomHome,
        duplicates: Option<&Path>,
    ) -> io::Result<()> {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    // The set-aside folder is not part of the inbox.
                    if duplicates.is_some_and(|d| path == *d) {
                        continue;
                    }
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
                let sha1 = sha1_hex(&bytes);
                if let Some(existing) = self.by_sha1.get(&sha1) {
                    if existing.home == RomHome::Collection
                        && home == RomHome::Inbox
                        && let Some(duplicates) = duplicates
                    {
                        std::fs::create_dir_all(duplicates)?;
                        let target = duplicates.join(path.file_name().unwrap_or(path.as_os_str()));
                        if std::fs::rename(&path, &target).is_ok() {
                            self.duplicates_moved += 1;
                        }
                    }
                    continue;
                }
                self.by_sha1.insert(sha1, ScannedRom { path, home });
                self.scanned += 1;
            }
        }
        Ok(())
    }
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
