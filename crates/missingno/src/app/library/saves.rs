use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// The save manifest for a game. Stored as `saves.ron` in the game directory.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SaveManifest {
    /// ID of the save to load when starting the game. None = fresh start.
    #[serde(default)]
    pub current: Option<String>,
    /// All known saves for this game.
    #[serde(default)]
    pub saves: Vec<SaveEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SaveEntry {
    /// Unique ID, used as filename stem: `{id}.sav`
    pub id: String,
    pub created: Timestamp,
    pub size_bytes: u32,
    pub origin: SaveOrigin,
    /// The save this was forked from (e.g. restored a historical save,
    /// then played and saved — the new save's parent is the restored one).
    #[serde(default)]
    pub parent: Option<String>,
    /// Index of the play session that created this save, if any.
    #[serde(default)]
    pub session_index: Option<usize>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SaveOrigin {
    /// Created during normal gameplay.
    Emulation,
    /// Imported by the user via file picker.
    Imported,
    /// Auto-imported from a .sav file next to the ROM on first library add.
    LegacyImport,
}

impl SaveManifest {
    #[allow(dead_code)]
    pub fn current_save(&self) -> Option<&SaveEntry> {
        let id = self.current.as_ref()?;
        self.saves.iter().find(|s| s.id == *id)
    }

    /// Record a new save created during emulation.
    pub fn record_emulation_save(
        &mut self,
        size_bytes: u32,
        session_index: Option<usize>,
    ) -> &SaveEntry {
        let now = Timestamp::now();
        let id = now.strftime("%Y%m%d-%H%M%S").to_string();

        let parent = self.current.clone();

        self.saves.push(SaveEntry {
            id: id.clone(),
            created: now,
            size_bytes,
            origin: SaveOrigin::Emulation,
            parent,
            session_index,
        });
        self.current = Some(id);

        self.saves.last().unwrap()
    }

    /// Record an imported save.
    pub fn record_import(&mut self, size_bytes: u32) -> &SaveEntry {
        let now = Timestamp::now();
        let id = format!("{}-import", now.strftime("%Y%m%d-%H%M%S"));

        self.saves.push(SaveEntry {
            id: id.clone(),
            created: now,
            size_bytes,
            origin: SaveOrigin::Imported,
            parent: None,
            session_index: None,
        });
        self.current = Some(id);

        self.saves.last().unwrap()
    }

    /// Record a legacy import (auto-imported from ROM directory).
    pub fn record_legacy_import(&mut self, size_bytes: u32) -> &SaveEntry {
        let now = Timestamp::now();
        let id = format!("{}-legacy", now.strftime("%Y%m%d-%H%M%S"));

        self.saves.push(SaveEntry {
            id: id.clone(),
            created: now,
            size_bytes,
            origin: SaveOrigin::LegacyImport,
            parent: None,
            session_index: None,
        });
        self.current = Some(id);

        self.saves.last().unwrap()
    }

    /// Set the current save to a historical one (for restore).
    pub fn restore(&mut self, save_id: &str) -> bool {
        if self.saves.iter().any(|s| s.id == save_id) {
            self.current = Some(save_id.to_string());
            true
        } else {
            false
        }
    }
}

// ── Filesystem operations ──────────────────────────────────────────────

fn saves_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("saves")
}

fn save_file_path(game_dir: &Path, id: &str) -> PathBuf {
    saves_dir(game_dir).join(format!("{id}.sav"))
}

fn manifest_path(game_dir: &Path) -> PathBuf {
    game_dir.join("saves.ron")
}

pub fn load_manifest(game_dir: &Path) -> SaveManifest {
    let path = manifest_path(game_dir);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| ron::from_str(&data).ok())
        .unwrap_or_default()
}

pub fn save_manifest(game_dir: &Path, manifest: &SaveManifest) {
    if let Ok(data) = ron::ser::to_string_pretty(manifest, ron::ser::PrettyConfig::default()) {
        let _ = std::fs::write(manifest_path(game_dir), data);
    }
}

/// Write save data to the saves directory and return the file path.
pub fn write_save_data(game_dir: &Path, id: &str, data: &[u8]) -> PathBuf {
    let dir = saves_dir(game_dir);
    let _ = std::fs::create_dir_all(&dir);
    let path = save_file_path(game_dir, id);
    let _ = std::fs::write(&path, data);
    path
}

/// Load save data for a given save ID.
pub fn load_save_data(game_dir: &Path, id: &str) -> Option<Vec<u8>> {
    std::fs::read(save_file_path(game_dir, id)).ok()
}

/// Load the current save's data.
pub fn load_current_save(game_dir: &Path) -> Option<Vec<u8>> {
    let manifest = load_manifest(game_dir);
    let id = manifest.current.as_ref()?;
    load_save_data(game_dir, id)
}

/// Migrate a legacy `battery.sav` into the new save system.
/// Returns true if a migration occurred.
pub fn migrate_legacy_battery(game_dir: &Path) -> bool {
    let legacy_path = game_dir.join("battery.sav");
    if !legacy_path.exists() {
        return false;
    }

    let mut manifest = load_manifest(game_dir);
    // Don't migrate if we already have saves
    if !manifest.saves.is_empty() {
        // Clean up the legacy file
        let _ = std::fs::remove_file(&legacy_path);
        return false;
    }

    let Ok(data) = std::fs::read(&legacy_path) else {
        return false;
    };

    let entry = manifest.record_legacy_import(data.len() as u32);
    let id = entry.id.clone();
    write_save_data(game_dir, &id, &data);
    save_manifest(game_dir, &manifest);

    // Remove legacy file
    let _ = std::fs::remove_file(&legacy_path);
    true
}
