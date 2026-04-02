pub(crate) mod hasheous;
pub(crate) mod info_panel;
pub(crate) mod play_log;
pub(crate) mod scanner;
pub(crate) mod view;

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameEntry {
    pub sha1: String,
    pub title: String,
    pub platform: Option<String>,
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

impl GameEntry {
    pub fn new(sha1: String, title: String, rom_path: PathBuf) -> Self {
        Self {
            sha1,
            title,
            platform: None,
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

    pub fn add_rom_path(&mut self, path: PathBuf) {
        let path_str = path.to_string_lossy();
        if !self.rom_paths.iter().any(|p| p.to_string_lossy() == path_str) {
            self.rom_paths.push(path);
        }
    }
}

pub fn library_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("missingno").join("games"))
}

pub fn game_dir_for(title: &str, sha1: &str) -> Option<PathBuf> {
    let folder_name = format!("{}_{}", sanitize_folder_name(title), &sha1[..8.min(sha1.len())]);
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
        if path.is_dir() {
            if let Some(entry) = load_entry(&path) {
                games.push((path, entry));
            }
        }
    }
    games.sort_by(|a, b| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase()));
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
        if let Some(entry) = load_entry(&path) {
            if entry.sha1 == sha1 {
                return Some((path, entry));
            }
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
    ron::from_str(&data).ok()
}

const THUMBNAIL_HEIGHT: u32 = 240;

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
        u32::MAX,
        THUMBNAIL_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    let _ = thumbnail.save(game_dir.join("thumbnail.png"));
}

pub fn battery_path(game_dir: &Path) -> PathBuf {
    game_dir.join("battery.sav")
}

pub fn save_battery(game_dir: &Path, data: &[u8]) {
    let _ = fs::create_dir_all(game_dir);
    let _ = fs::write(battery_path(game_dir), data);
}

pub fn load_battery(game_dir: &Path) -> Option<Vec<u8>> {
    fs::read(battery_path(game_dir)).ok()
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
