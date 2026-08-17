pub(crate) mod activity;
pub(crate) mod catalogue;
pub(crate) mod detail_view;
pub(crate) mod game_db;
pub(crate) mod hasheous;
pub(crate) mod homebrew_browser;
pub(crate) mod homebrew_hub;
pub(crate) mod scanner;
pub(crate) mod screenshot_gallery;
pub(crate) mod store;
pub(in crate::app) mod update;
pub(crate) mod view;

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::app::system::LaunchValues;

/// Current version of the GameEntry format. Increment when adding migrations.
const CURRENT_VERSION: u32 = 1;

/// Accepts the platform enum, or the free-text platform string older
/// entries stored (our old names or raw Hasheous ones), mapped best-effort.
fn compat_platform<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<crate::app::system::Platform>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Compat {
        Platform(crate::app::system::Platform),
        Legacy(String),
    }
    Ok(match Option::<Compat>::deserialize(deserializer)? {
        Some(Compat::Platform(platform)) => Some(platform),
        Some(Compat::Legacy(text)) => crate::app::system::Platform::from_description(&text),
        None => None,
    })
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameEntry {
    /// Schema version. Migrations run on load when this is less than CURRENT_VERSION.
    #[serde(default)]
    pub version: u32,
    pub sha1: String,
    pub title: String,
    /// The raw title from the ROM header (bytes 0x134-0x143). Preserved across enrichment
    /// so we can match against physical cartridge headers.
    #[serde(default)]
    pub header_title: Option<String>,
    #[serde(default, deserialize_with = "compat_platform")]
    pub platform: Option<crate::app::system::Platform>,
    /// Launch options the user set for this game themselves, sparse and keyed
    /// by option id. What the catalogue states about the dump is resolved live
    /// at launch, so only the user's own word is kept here — and a rescan never
    /// touches it.
    #[serde(default, skip_serializing_if = "LaunchValues::is_empty")]
    pub overrides: LaunchValues,
    pub publisher: Option<String>,
    pub year: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub wikipedia_url: Option<String>,
    #[serde(default)]
    pub igdb_url: Option<String>,
    #[serde(default)]
    pub enrichment_attempted: bool,
    pub rom_paths: Vec<PathBuf>,
}

/// Where a game's system name sits in its metadata line, when it appears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SystemName {
    Leading,
    Trailing,
    Omitted,
}

impl GameEntry {
    pub fn new(sha1: String, title: String, rom_path: PathBuf) -> Self {
        Self {
            version: CURRENT_VERSION,
            sha1,
            title,
            header_title: None,
            platform: None,
            overrides: LaunchValues::default(),
            publisher: None,
            year: None,
            description: None,
            wikipedia_url: None,
            igdb_url: None,
            enrichment_attempted: false,
            rom_paths: vec![rom_path],
        }
    }

    pub fn display_title(&self) -> String {
        // No-Intro names put articles after the main title:
        //   "Legend of Zelda, The - Link's Awakening"
        //   "Final Fantasy Legend, The"
        // Move the article back to the front of that segment.
        for article in [", The", ", A", ", An"] {
            // Check for "Name, The - Subtitle" or "Name, The" at end
            if let Some(pos) = self.title.find(article) {
                let after_article = pos + article.len();
                let art = &article[2..]; // "The", "A", "An"
                let base = &self.title[..pos];
                let rest = &self.title[after_article..];
                return format!("{art} {base}{rest}");
            }
        }
        self.title.clone()
    }

    /// "Publisher · 1994 · Game Boy", minus whatever the entry doesn't know.
    /// Nothing known at all reads as no line rather than an empty one.
    pub fn metadata_line(&self, system: SystemName) -> Option<String> {
        let system_name = match system {
            SystemName::Omitted => None,
            _ => self.platform.map(|platform| platform.name().to_string()),
        };
        let publisher = self.publisher.clone();
        let year = self.year.as_ref().map(|year| activity::release_year(year));

        let parts: Vec<String> = match system {
            SystemName::Leading => [system_name, publisher, year],
            _ => [publisher, year, system_name],
        }
        .into_iter()
        .flatten()
        .collect();

        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Fold a metadata lookup's answer into the entry, marking it looked up.
    pub fn apply_metadata(&mut self, info: hasheous::GameInfo) {
        self.title = info.name;
        // Hasheous's platform string never overrides the header-derived
        // classification; it only fills a gap, mapped to our own type.
        if self.platform.is_none() {
            self.platform = info
                .platform
                .as_deref()
                .and_then(crate::app::system::Platform::from_description);
        }
        self.publisher = info.publisher;
        self.year = info.year;
        self.wikipedia_url = info.wikipedia_url;
        self.igdb_url = info.igdb_url;
        self.enrichment_attempted = true;
    }

    pub fn add_rom_path(&mut self, path: PathBuf) {
        let path_str = path.to_string_lossy();
        if !self
            .rom_paths
            .iter()
            .any(|p| p.to_string_lossy() == path_str)
        {
            self.rom_paths.push(path);
        }
    }
}

pub fn library_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("missingno").join("games"))
}

pub fn game_dir_for(title: &str, sha1: &str) -> Option<PathBuf> {
    let folder_name = format!(
        "{}_{}",
        sanitize_folder_name(title),
        &sha1[..8.min(sha1.len())]
    );
    library_dir().map(|dir| dir.join(folder_name))
}

pub fn list_all() -> Vec<(PathBuf, GameEntry)> {
    let Some(lib_dir) = library_dir() else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&lib_dir) else {
        return Vec::new();
    };

    let mut games = Vec::new();
    for dir_entry in entries.flatten() {
        let path = dir_entry.path();
        if path.is_dir()
            && let Some(entry) = load_entry(&path)
        {
            games.push((path, entry));
        }
    }
    games.sort_by_key(|a| a.1.title.to_lowercase());
    games
}

pub fn find_by_sha1(sha1: &str) -> Option<(PathBuf, GameEntry)> {
    let lib_dir = library_dir()?;
    let entries = fs::read_dir(&lib_dir).ok()?;

    for dir_entry in entries.flatten() {
        let path = dir_entry.path();
        if !path.is_dir() {
            continue;
        }
        if let Some(entry) = load_entry(&path)
            && entry.sha1 == sha1
        {
            return Some((path, entry));
        }
    }
    None
}

pub fn save_entry(game_dir: &Path, entry: &GameEntry) {
    let _ = fs::create_dir_all(game_dir);
    let path = game_dir.join("game.ron");
    if let Ok(data) = ron::ser::to_string_pretty(entry, ron::ser::PrettyConfig::default()) {
        let _ = fs::write(path, data);
    }
}

pub fn load_entry(game_dir: &Path) -> Option<GameEntry> {
    let path = game_dir.join("game.ron");
    let data = fs::read_to_string(path).ok()?;
    let mut entry: GameEntry = ron::from_str(&data).ok()?;

    if entry.version < CURRENT_VERSION {
        migrate(&mut entry);
        save_entry(game_dir, &entry);
    }

    Some(entry)
}

/// Run all pending migrations on a GameEntry.
fn migrate(entry: &mut GameEntry) {
    use missingno_gb::cartridge::Cartridge;

    // v0 → v1: backfill header_title from ROM file
    if entry.version < 1 && entry.header_title.is_none() {
        entry.header_title = entry.rom_paths.iter().find_map(|path| {
            let mut file = fs::File::open(path).ok()?;
            let mut buf = vec![0u8; 0x144];
            std::io::Read::read_exact(&mut file, &mut buf).ok()?;
            let title = Cartridge::peek_title(&buf);
            if title.is_empty() { None } else { Some(title) }
        });
    }

    entry.version = CURRENT_VERSION;
}

// Thumbnails are 2× the display size (160×120) for crisp rendering on HiDPI.
const THUMBNAIL_WIDTH: u32 = 240;
const THUMBNAIL_HEIGHT: u32 = 320;

pub fn save_cover(game_dir: &Path, bytes: &[u8]) {
    let _ = fs::create_dir_all(game_dir);
    let _ = fs::write(game_dir.join("cover.png"), bytes);
    generate_thumbnail(game_dir, bytes);
}

pub fn load_cover(game_dir: &Path) -> Option<Vec<u8>> {
    fs::read(game_dir.join("cover.png")).ok()
}

pub fn load_thumbnail(game_dir: &Path) -> Option<Vec<u8>> {
    let thumb_path = game_dir.join("thumbnail.png");
    if thumb_path.exists() {
        return fs::read(thumb_path).ok();
    }
    // Generate from cover if thumbnail is missing
    if let Some(cover_bytes) = load_cover(game_dir) {
        generate_thumbnail(game_dir, &cover_bytes);
        return fs::read(game_dir.join("thumbnail.png")).ok();
    }
    None
}

fn generate_thumbnail(game_dir: &Path, cover_bytes: &[u8]) {
    let Ok(img) = image::load_from_memory(cover_bytes) else {
        return;
    };
    let thumbnail = img.resize(
        THUMBNAIL_WIDTH,
        THUMBNAIL_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let _ = thumbnail.save(game_dir.join("thumbnail.png"));
}

/// Remove a game from the library entirely.
pub fn remove_game(game_dir: &Path) {
    let _ = fs::remove_dir_all(game_dir);
}

fn sanitize_folder_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            c => c,
        })
        .collect();

    let trimmed = sanitized.trim().trim_matches('.').to_string();

    if trimmed.is_empty() {
        "unknown".to_string()
    } else if trimmed.len() > 64 {
        trimmed[..64].trim_end().to_string()
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::system::Platform;

    fn entry_ron(platform: &str) -> String {
        format!(
            "(version: 1, sha1: \"abc\", title: \"T\", platform: {platform}, \
             publisher: None, year: None, description: None, rom_paths: [])"
        )
    }

    #[test]
    fn platform_round_trips_as_enum() {
        let mut entry = GameEntry::new("abc".into(), "T".into(), PathBuf::from("t.gb"));
        entry.platform = Some(Platform::GameBoyColor);
        let ron = ron::ser::to_string(&entry).unwrap();
        let back: GameEntry = ron::from_str(&ron).unwrap();
        assert_eq!(back.platform, Some(Platform::GameBoyColor));
    }

    #[test]
    fn legacy_platform_strings_map_to_the_enum() {
        let entry: GameEntry = ron::from_str(&entry_ron("Some(\"Atari 2600\")")).unwrap();
        assert_eq!(entry.platform, Some(Platform::AtariVcs));
        let entry: GameEntry = ron::from_str(&entry_ron("Some(\"Nintendo Game Boy\")")).unwrap();
        assert_eq!(entry.platform, Some(Platform::GameBoy));
        let entry: GameEntry = ron::from_str(&entry_ron("Some(\"Game Boy Color\")")).unwrap();
        assert_eq!(entry.platform, Some(Platform::GameBoyColor));
    }

    #[test]
    fn unknown_platform_strings_drop_to_none() {
        let entry: GameEntry = ron::from_str(&entry_ron("Some(\"Neo Geo\")")).unwrap();
        assert_eq!(entry.platform, None);
    }

    // Entries written before catalogue facts were resolved live carry them as
    // fields of their own; they must still load, minus the stale copies.
    #[test]
    fn an_entry_carrying_the_old_catalogue_copies_still_loads() {
        let entry: GameEntry = ron::from_str(
            "(version: 1, sha1: \"abc\", title: \"T\", platform: Some(AtariVcs), \
             tv_standard: Some(Pal), cart_type: Some(\"F8\"), overdump: true, \
             controllers: [Paddle], publisher: None, year: None, description: None, \
             rom_paths: [])",
        )
        .unwrap();
        assert_eq!(entry.platform, Some(Platform::AtariVcs));
        assert!(entry.overrides.is_empty());
    }

    #[test]
    fn overrides_round_trip() {
        let mut entry = GameEntry::new("abc".into(), "T".into(), PathBuf::from("t.a26"));
        entry.overrides.set_choice("board", "F8");
        let ron = ron::ser::to_string(&entry).unwrap();
        let back: GameEntry = ron::from_str(&ron).unwrap();
        assert_eq!(back.overrides.choice("board"), Some("F8"));
    }

    /// An empty override bag leaves nothing behind in the file.
    #[test]
    fn an_entry_with_no_overrides_writes_none() {
        let entry = GameEntry::new("abc".into(), "T".into(), PathBuf::from("t.gb"));
        let ron = ron::ser::to_string(&entry).unwrap();
        assert!(!ron.contains("overrides"));
    }
}
