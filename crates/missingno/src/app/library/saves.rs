use std::path::{Path, PathBuf};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// The save manifest for a game. Stored as `saves.ron` in the game directory.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SaveManifest {
    /// Legacy field — kept for backward-compatible deserialization.
    /// Boot save is now always the most recent, or explicitly chosen via PlayWithSave.
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
    /// Index of this entry in the save archive file.
    #[serde(default)]
    pub archive_index: Option<usize>,
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
    /// The next archive index for a new entry.
    fn next_archive_index(&self) -> usize {
        self.saves
            .iter()
            .filter_map(|s| s.archive_index)
            .max()
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    /// Record a new save created during emulation.
    pub fn record_emulation_save(
        &mut self,
        size_bytes: u32,
        session_index: Option<usize>,
    ) -> &SaveEntry {
        let now = Timestamp::now();
        let id = now.strftime("%Y%m%d-%H%M%S").to_string();
        let parent = self.saves.last().map(|s| s.id.clone());
        let archive_index = Some(self.next_archive_index());

        self.saves.push(SaveEntry {
            id,
            created: now,
            size_bytes,
            origin: SaveOrigin::Emulation,
            parent,
            session_index,
            archive_index,
        });

        self.saves.last().unwrap()
    }

    /// Record an imported save.
    pub fn record_import(&mut self, size_bytes: u32) -> &SaveEntry {
        let now = Timestamp::now();
        let id = format!("{}-import", now.strftime("%Y%m%d-%H%M%S"));
        let archive_index = Some(self.next_archive_index());

        self.saves.push(SaveEntry {
            id,
            created: now,
            size_bytes,
            origin: SaveOrigin::Imported,
            parent: None,
            session_index: None,
            archive_index,
        });

        self.saves.last().unwrap()
    }

    /// Record a legacy import (auto-imported from ROM directory).
    pub fn record_legacy_import(&mut self, size_bytes: u32) -> &SaveEntry {
        let now = Timestamp::now();
        let id = format!("{}-legacy", now.strftime("%Y%m%d-%H%M%S"));
        let archive_index = Some(self.next_archive_index());

        self.saves.push(SaveEntry {
            id,
            created: now,
            size_bytes,
            origin: SaveOrigin::LegacyImport,
            parent: None,
            session_index: None,
            archive_index,
        });

        self.saves.last().unwrap()
    }
}

/// Format a timestamp as locale-aware date + time (e.g. "3 Apr 2026, 2:32 PM").
pub fn format_local(ts: &Timestamp) -> String {
    libc_strftime("%e %b %Y, %X", ts.as_second())
}

/// Format a timestamp as locale-aware time only (e.g. "2:32 PM").
pub fn format_local_time(ts: &Timestamp) -> String {
    libc_strftime("%X", ts.as_second())
}

/// Use libc's strftime which respects LC_TIME for locale-aware formatting.
fn libc_strftime(fmt: &str, unix_secs: i64) -> String {
    use std::ffi::CString;
    let fmt_c = CString::new(fmt).unwrap();
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time = unix_secs as libc::time_t;
    unsafe { libc::localtime_r(&time, &mut tm) };
    let mut buf = vec![0u8; 256];
    let len = unsafe {
        libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            fmt_c.as_ptr(),
            &tm,
        )
    };
    String::from_utf8_lossy(&buf[..len]).trim().to_string()
}

// ── Filesystem operations ──────────────────────────────────────────────

use super::save_archive;

fn archive_path(game_dir: &Path) -> PathBuf {
    game_dir.join("saves.bin")
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

/// Write save data to the archive. `prev_data` is used for delta compression.
pub fn write_save_data(game_dir: &Path, data: &[u8], archive_index: usize, prev_data: Option<&[u8]>) {
    let _ = std::fs::create_dir_all(game_dir);
    let _ = save_archive::append_save(
        &archive_path(game_dir),
        data,
        archive_index,
        prev_data,
    );
}

/// Load save data by archive index.
pub fn load_save_by_index(game_dir: &Path, index: usize) -> Option<Vec<u8>> {
    save_archive::read_save(&archive_path(game_dir), index).ok()
}

/// Load the most recent save's data.
pub fn load_current_save(game_dir: &Path) -> Option<Vec<u8>> {
    let manifest = load_manifest(game_dir);
    let entry = manifest.saves.last()?;
    let index = entry.archive_index?;
    load_save_by_index(game_dir, index)
}

/// Load a specific save's data by ID.
pub fn load_save_by_id(game_dir: &Path, id: &str) -> Option<Vec<u8>> {
    let manifest = load_manifest(game_dir);
    let entry = manifest.saves.iter().find(|s| s.id == id)?;
    let index = entry.archive_index?;
    load_save_by_index(game_dir, index)
}

/// Export a save as raw .sav data (for the download button).
pub fn export_save(game_dir: &Path, id: &str) -> Option<Vec<u8>> {
    load_save_by_id(game_dir, id)
}

/// Migrate a legacy `battery.sav` into the new save system.
pub fn migrate_legacy_battery(game_dir: &Path) -> bool {
    let legacy_path = game_dir.join("battery.sav");
    if !legacy_path.exists() {
        return false;
    }

    let mut manifest = load_manifest(game_dir);
    if !manifest.saves.is_empty() {
        let _ = std::fs::remove_file(&legacy_path);
        return false;
    }

    let Ok(data) = std::fs::read(&legacy_path) else {
        return false;
    };

    let entry = manifest.record_legacy_import(data.len() as u32);
    let archive_idx = entry.archive_index.unwrap();
    write_save_data(game_dir, &data, archive_idx, None);
    save_manifest(game_dir, &manifest);

    let _ = std::fs::remove_file(&legacy_path);
    true
}

/// Migrate individual .sav files to the archive (for existing libraries).
pub fn migrate_individual_saves(game_dir: &Path) {
    let manifest = load_manifest(game_dir);
    let saves_dir = game_dir.join("saves");

    if !saves_dir.exists() {
        return;
    }

    // Find saves that have no archive_index (old format)
    let unarchived: Vec<&SaveEntry> = manifest
        .saves
        .iter()
        .filter(|s| s.archive_index.is_none())
        .collect();

    if unarchived.is_empty() {
        return;
    }

    let save_ids: Vec<String> = unarchived.iter().map(|s| s.id.clone()).collect();
    let _ = save_archive::migrate_individual_saves(
        &archive_path(game_dir),
        &saves_dir,
        &save_ids,
    );

    // Update manifest with archive indices
    let mut manifest = load_manifest(game_dir);
    let base_index = save_archive::entry_count(&archive_path(game_dir))
        .saturating_sub(save_ids.len());
    for save in manifest.saves.iter_mut() {
        if save.archive_index.is_none() {
            if let Some(pos) = save_ids.iter().position(|id| id == &save.id) {
                save.archive_index = Some(base_index + pos);
            }
        }
    }
    save_manifest(game_dir, &manifest);
}
