use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use iced::widget::image;
use jiff::Timestamp;

use super::{activity, GameEntry};

// ── Public data types ─────────────────────────────────────────────────

/// Everything the library grid needs for one game tile.
#[derive(Clone, Debug)]
pub struct GameSummary {
    pub entry: GameEntry,
    pub game_dir: PathBuf,
    pub thumbnail: Option<image::Handle>,
    pub play_time_secs: f64,
    pub last_played: Option<Timestamp>,
    pub save_count: usize,
}

/// Activity detail for the currently viewed game.
#[derive(Clone, Debug)]
pub struct ActivityDetail {
    pub sha1: String,
    pub sessions: Vec<SessionSummary>,
}

/// One session in the activity detail.
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub filename: String,
    pub kind: activity::ActivityKind,
    pub start: Timestamp,
    pub end: Option<Timestamp>,
    pub save_count: usize,
    pub last_save_time: Option<Timestamp>,
    pub screenshots: Vec<image::Handle>,
    /// For imports: the size in bytes.
    pub size_bytes: Option<u32>,
}

// ── GameStore ──────────────────────────────────────────────────────────

/// Centralised game data store. Owns the index of known games and
/// provides lazy, cached access to metadata and activity data.
/// The UI never does disk I/O — it asks the store.
pub struct GameStore {
    /// sha1 → game_dir. Built from directory listing.
    index: HashMap<String, PathBuf>,

    /// Cached game summaries, keyed by sha1. Loaded on demand.
    summaries: HashMap<String, GameSummary>,

    /// Sorted sha1 list for the library grid. Rebuilt when summaries change.
    sorted_keys: Vec<String>,
    sorted_dirty: bool,

    /// Activity detail for one game at a time. Cleared when switching games.
    activity_detail: Option<ActivityDetail>,

    /// Cached screenshot handles for the live session (avoids re-rendering
    /// every frame). Only invalidated when a new screenshot is taken.
    live_screenshots: Vec<image::Handle>,
    live_screenshot_count: usize,
}

impl GameStore {
    /// Create a new store and scan the library directory.
    pub fn new() -> Self {
        let mut store = Self {
            index: HashMap::new(),
            summaries: HashMap::new(),
            sorted_keys: Vec::new(),
            sorted_dirty: true,
            activity_detail: None,
            live_screenshots: Vec::new(),
            live_screenshot_count: 0,
        };
        store.rebuild_index();
        store
    }

    // ── Index ──────────────────────────────────────────────────────────

    /// Scan the library directory and build the sha1 → game_dir index.
    /// Also eagerly loads all summaries (game count is small enough).
    pub fn rebuild_index(&mut self) {
        self.index.clear();
        self.summaries.clear();
        self.sorted_dirty = true;

        let Some(lib_dir) = super::library_dir() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&lib_dir) else {
            return;
        };

        for dir_entry in entries.flatten() {
            let path = dir_entry.path();
            if path.is_dir() {
                if let Some(entry) = super::load_entry(&path) {
                    let sha1 = entry.sha1.clone();
                    self.index.insert(sha1.clone(), path.clone());
                    self.summaries
                        .insert(sha1, Self::load_summary(path, entry));
                }
            }
        }
    }

    /// Resolve a sha1 to its game directory.
    pub fn game_dir(&self, sha1: &str) -> Option<&Path> {
        self.index.get(sha1).map(|p| p.as_path())
    }

    /// Get a game entry by sha1.
    pub fn entry(&self, sha1: &str) -> Option<&GameEntry> {
        self.summaries.get(sha1).map(|s| &s.entry)
    }

    // ── Summaries (library grid) ───────────────────────────────────────

    /// Get all game summaries, sorted for the library grid.
    pub fn all_summaries(&self) -> Vec<&GameSummary> {
        if self.sorted_dirty {
            // Sorted keys are stale — sort on the fly
            let mut entries: Vec<&GameSummary> = self.summaries.values().collect();
            entries.sort_by(|a, b| Self::sort_cmp(a, b));
            return entries;
        }
        self.sorted_keys
            .iter()
            .filter_map(|sha1| self.summaries.get(sha1))
            .collect()
    }

    /// Flush the sorted key cache (call when you have &mut self available).
    pub fn ensure_sorted(&mut self) {
        if self.sorted_dirty {
            self.rebuild_sorted_keys();
        }
    }

    fn sort_cmp(a: &GameSummary, b: &GameSummary) -> std::cmp::Ordering {
        match (&a.last_played, &b.last_played) {
            (Some(a_ts), Some(b_ts)) => b_ts.cmp(a_ts),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a
                .entry
                .display_title()
                .to_lowercase()
                .cmp(&b.entry.display_title().to_lowercase()),
        }
    }

    /// Get a specific game summary.
    pub fn summary(&self, sha1: &str) -> Option<&GameSummary> {
        self.summaries.get(sha1)
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    fn rebuild_sorted_keys(&mut self) {
        let mut entries: Vec<_> = self.summaries.values().collect();
        entries.sort_by(|a, b| {
            match (&a.last_played, &b.last_played) {
                (Some(a_ts), Some(b_ts)) => b_ts.cmp(a_ts),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a
                    .entry
                    .display_title()
                    .to_lowercase()
                    .cmp(&b.entry.display_title().to_lowercase()),
            }
        });
        self.sorted_keys = entries.iter().map(|e| e.entry.sha1.clone()).collect();
        self.sorted_dirty = false;
    }

    fn load_summary(game_dir: PathBuf, entry: GameEntry) -> GameSummary {
        let thumbnail =
            super::load_thumbnail(&game_dir).map(|bytes| image::Handle::from_bytes(bytes));
        let stats = activity::compute_stats(&game_dir);

        GameSummary {
            entry,
            game_dir,
            thumbnail,
            play_time_secs: stats.total_play_time_secs,
            last_played: stats.last_played,
            save_count: stats.save_count,
        }
    }

    // ── Activity detail (detail page) ──────────────────────────────────

    /// Get cached activity detail. Returns None if not loaded for this game.
    /// Call `ensure_activity_loaded(sha1)` when you have &mut self.
    pub fn activity_for(&self, sha1: &str) -> Option<&ActivityDetail> {
        self.activity_detail
            .as_ref()
            .filter(|d| d.sha1 == sha1)
    }

    /// Ensure activity detail is loaded for this game. Call from message
    /// handlers (where &mut self is available), not from view().
    pub fn ensure_activity_loaded(&mut self, sha1: &str) {
        let needs_load = self
            .activity_detail
            .as_ref()
            .map(|d| d.sha1 != sha1)
            .unwrap_or(true);

        if needs_load {
            self.activity_detail = Some(self.load_activity_detail(sha1));
        }
    }

    fn load_activity_detail(&self, sha1: &str) -> ActivityDetail {
        let Some(game_dir) = self.index.get(sha1) else {
            return ActivityDetail {
                sha1: sha1.to_string(),
                sessions: Vec::new(),
            };
        };

        let refs = activity::list_activity(game_dir);
        let sessions = refs
            .into_iter()
            .filter_map(|r| {
                let data = activity::read_compressed_file(game_dir, &r.filename)?;
                match r.kind {
                    activity::ActivityKind::Session => {
                        let session = activity::read_session_from_str(&data)?;
                        let screenshots = session
                            .events
                            .iter()
                            .filter_map(|e| match &e.kind {
                                activity::EventKind::Screenshot { frame } => {
                                    Some(frame.to_image_handle())
                                }
                                _ => None,
                            })
                            .collect();

                        Some(SessionSummary {
                            filename: r.filename,
                            kind: activity::ActivityKind::Session,
                            start: session.start,
                            end: session.end,
                            save_count: session.save_count(),
                            last_save_time: session.last_save_time(),
                            screenshots,
                            size_bytes: None,
                        })
                    }
                    activity::ActivityKind::Import => {
                        let import: activity::ImportFile = ron::from_str(&data).ok()?;
                        let ts_str = r.filename.strip_suffix(".import")?;
                        let timestamp = activity::parse_filename_timestamp(ts_str)?;
                        Some(SessionSummary {
                            filename: r.filename,
                            kind: activity::ActivityKind::Import,
                            start: timestamp,
                            end: None,
                            save_count: 0,
                            last_save_time: None,
                            screenshots: Vec::new(),
                            size_bytes: Some(import.size_bytes),
                        })
                    }
                }
            })
            .collect();

        ActivityDetail {
            sha1: sha1.to_string(),
            sessions,
        }
    }

    // ── Live session screenshots ───────────────────────────────────────

    /// Get cached screenshot handles for the live session.
    /// Call `update_live_screenshots` when a new screenshot is taken.
    pub fn live_screenshots(&self) -> &[image::Handle] {
        &self.live_screenshots
    }

    /// Update the live screenshot cache from the current session.
    /// Only re-renders handles for newly added screenshots.
    pub fn update_live_screenshots(&mut self, session: &activity::SessionFile) {
        let current_count = session
            .events
            .iter()
            .filter(|e| matches!(e.kind, activity::EventKind::Screenshot { .. }))
            .count();

        if current_count > self.live_screenshot_count {
            // Only render the new ones
            let new_handles: Vec<_> = session
                .events
                .iter()
                .filter_map(|e| match &e.kind {
                    activity::EventKind::Screenshot { frame } => Some(frame.to_image_handle()),
                    _ => None,
                })
                .skip(self.live_screenshot_count)
                .collect();

            self.live_screenshots.extend(new_handles);
            self.live_screenshot_count = current_count;
        }
    }

    /// Reset live screenshot cache (e.g., when starting a new session).
    pub fn reset_live_screenshots(&mut self) {
        self.live_screenshots.clear();
        self.live_screenshot_count = 0;
    }

    // ── Invalidation ───────────────────────────────────────────────────

    /// Called after a session event is written (save, screenshot, session end).
    /// Invalidates activity detail and updates the game summary stats.
    pub fn notify_activity_changed(&mut self, sha1: &str) {
        // Invalidate activity detail if it's for this game
        if self
            .activity_detail
            .as_ref()
            .map(|d| d.sha1 == sha1)
            .unwrap_or(false)
        {
            self.activity_detail = None;
        }

        // Refresh the summary stats for this game
        if let Some(game_dir) = self.index.get(sha1).cloned() {
            if let Some(summary) = self.summaries.get_mut(sha1) {
                let stats = activity::compute_stats(&game_dir);
                summary.play_time_secs = stats.total_play_time_secs;
                summary.last_played = stats.last_played;
                summary.save_count = stats.save_count;
                self.sorted_dirty = true;
            }
        }
    }

    /// Called after metadata changes (enrichment, title update).
    pub fn notify_metadata_changed(&mut self, sha1: &str) {
        if let Some(game_dir) = self.index.get(sha1).cloned() {
            if let Some(entry) = super::load_entry(&game_dir) {
                let thumbnail =
                    super::load_thumbnail(&game_dir).map(|b| image::Handle::from_bytes(b));
                if let Some(summary) = self.summaries.get_mut(sha1) {
                    summary.entry = entry;
                    summary.thumbnail = thumbnail;
                    self.sorted_dirty = true;
                }
            }
        }
    }

    /// Called after a game is added to the library.
    pub fn notify_game_added(&mut self, sha1: &str, game_dir: PathBuf) {
        if let Some(entry) = super::load_entry(&game_dir) {
            self.index.insert(sha1.to_string(), game_dir.clone());
            self.summaries
                .insert(sha1.to_string(), Self::load_summary(game_dir, entry));
            self.sorted_dirty = true;
        }
    }

    /// Called after a game is removed from the library.
    pub fn notify_game_removed(&mut self, sha1: &str) {
        self.index.remove(sha1);
        self.summaries.remove(sha1);
        self.sorted_dirty = true;
        if self
            .activity_detail
            .as_ref()
            .map(|d| d.sha1 == sha1)
            .unwrap_or(false)
        {
            self.activity_detail = None;
        }
    }
}
