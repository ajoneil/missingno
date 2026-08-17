use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use iced::widget::image;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::{GameEntry, activity};

// ── Public data types ─────────────────────────────────────────────────

/// Everything the library grid needs for one game tile.
#[derive(Clone, Debug)]
pub struct GameSummary {
    pub entry: GameEntry,
    pub thumbnail: Option<image::Handle>,
    pub play_time_secs: f64,
    pub last_played: Option<Timestamp>,
    pub save_count: usize,
}

impl GameSummary {
    fn matches(&self, lowercase_filter: &str) -> bool {
        if lowercase_filter.is_empty() {
            return true;
        }
        let title_hit = self
            .entry
            .display_title()
            .to_lowercase()
            .contains(lowercase_filter);
        let publisher_hit = self
            .entry
            .publisher
            .as_ref()
            .is_some_and(|publisher| publisher.to_lowercase().contains(lowercase_filter));
        title_hit || publisher_hit
    }
}

/// Library orderings. Every key falls back to title order so ties are stable
/// and scannable.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum SortKey {
    #[default]
    LastPlayed,
    Title,
    Year,
    MostPlayed,
}

impl SortKey {
    /// Selectable keys, in toolbar order.
    pub const ALL: [SortKey; 4] = [
        SortKey::LastPlayed,
        SortKey::Title,
        SortKey::Year,
        SortKey::MostPlayed,
    ];

    fn label(self) -> &'static str {
        match self {
            SortKey::LastPlayed => "Last played",
            SortKey::Title => "Title",
            SortKey::Year => "Year",
            SortKey::MostPlayed => "Most played",
        }
    }

    fn compare(self, a: &GameSummary, b: &GameSummary) -> std::cmp::Ordering {
        let by_title = |a: &GameSummary, b: &GameSummary| {
            a.entry
                .display_title()
                .to_lowercase()
                .cmp(&b.entry.display_title().to_lowercase())
        };
        match self {
            SortKey::Title => by_title(a, b),
            SortKey::LastPlayed => match (&a.last_played, &b.last_played) {
                (Some(a_ts), Some(b_ts)) => b_ts.cmp(a_ts),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => by_title(a, b),
            },
            SortKey::Year => match (&a.entry.year, &b.entry.year) {
                (Some(a_year), Some(b_year)) => a_year.cmp(b_year).then_with(|| by_title(a, b)),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => by_title(a, b),
            },
            SortKey::MostPlayed => b
                .play_time_secs
                .total_cmp(&a.play_time_secs)
                .then_with(|| by_title(a, b)),
        }
    }
}

impl std::fmt::Display for SortKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Library system filter. "All systems" shows every entry (including ones
/// with no known platform); a specific system shows only entries tagged with
/// it. The picker doubles as an at-a-glance list of supported systems.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SystemFilter {
    #[default]
    All,
    System(crate::app::system::Platform),
}

impl SystemFilter {
    /// Picker options: "All systems" followed by every registered platform,
    /// alphabetically.
    pub fn all_options() -> Vec<SystemFilter> {
        std::iter::once(SystemFilter::All)
            .chain(
                crate::app::system::platforms_by_name()
                    .into_iter()
                    .map(SystemFilter::System),
            )
            .collect()
    }

    fn accepts(self, summary: &GameSummary) -> bool {
        match self {
            SystemFilter::All => true,
            SystemFilter::System(platform) => summary.entry.platform == Some(platform),
        }
    }
}

impl std::fmt::Display for SystemFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemFilter::All => f.write_str("All systems"),
            SystemFilter::System(platform) => f.write_str(platform.name()),
        }
    }
}

/// Activity detail for the currently viewed game.
/// The state of activity data for a game.
pub enum ActivityState {
    /// Background load in progress.
    Loading,
    /// Loaded and ready.
    Loaded(ActivityDetail),
}

#[derive(Clone, Debug)]
pub struct ActivityDetail {
    pub sessions: Vec<SessionSummary>,
    /// Last cartridge sync hash and timestamp, if any.
    pub last_cart_sync: Option<(String, Timestamp)>,
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
    pub prints: Vec<image::Handle>,
    /// For imports: the size in bytes.
    pub size_bytes: Option<u32>,
    /// For imports: where the save came from.
    pub import_source: Option<activity::ImportSource>,
}

/// Raw session data loaded from disk (no image handles — those are created
/// on the main thread after the background load completes).
#[derive(Clone, Debug)]
pub struct RawActivityDetail {
    pub sha1: String,
    pub sessions: Vec<RawSessionSummary>,
    pub last_cart_sync: Option<(String, Timestamp)>,
}

#[derive(Clone, Debug)]
pub struct RawSessionSummary {
    pub filename: String,
    pub kind: activity::ActivityKind,
    pub start: Timestamp,
    pub end: Option<Timestamp>,
    pub save_count: usize,
    pub last_save_time: Option<Timestamp>,
    pub screenshots: Vec<activity::FrameCapture>,
    pub prints: Vec<activity::PrintCapture>,
    pub size_bytes: Option<u32>,
    pub import_source: Option<activity::ImportSource>,
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

    /// Activity state for the currently viewed game.
    activity_state: Option<(String, ActivityState)>,

    /// Cached screenshot handles for the live session (avoids re-rendering
    /// every frame). Only invalidated when a new screenshot is taken.
    live_screenshots: Vec<image::Handle>,
    live_prints: Vec<image::Handle>,
}

impl GameStore {
    fn empty() -> Self {
        Self {
            index: HashMap::new(),
            summaries: HashMap::new(),
            activity_state: None,
            live_screenshots: Vec::new(),
            live_prints: Vec::new(),
        }
    }

    /// Create a new store and scan the library directory.
    pub fn new() -> Self {
        let mut store = Self::empty();
        store.rebuild_index();
        store
    }

    // ── Index ──────────────────────────────────────────────────────────

    /// Scan the library directory and build the sha1 → game_dir index.
    /// Also eagerly loads all summaries (game count is small enough).
    pub fn rebuild_index(&mut self) {
        self.index.clear();
        self.summaries.clear();

        let Some(lib_dir) = super::library_dir() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&lib_dir) else {
            return;
        };

        for dir_entry in entries.flatten() {
            let path = dir_entry.path();
            if path.is_dir()
                && let Some(entry) = super::load_entry(&path)
            {
                let sha1 = entry.sha1.clone();
                self.index.insert(sha1.clone(), path.clone());
                self.summaries.insert(sha1, Self::load_summary(path, entry));
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

    /// Get all game summaries in the default library order.
    pub fn all_summaries(&self) -> Vec<&GameSummary> {
        self.summaries_sorted(SortKey::default(), "", SystemFilter::All)
    }

    /// Game summaries matching `filter` (case-insensitive substring of title
    /// or publisher; empty matches all) and `system`, ordered by `sort`.
    pub fn summaries_sorted(
        &self,
        sort: SortKey,
        filter: &str,
        system: SystemFilter,
    ) -> Vec<&GameSummary> {
        let filter = filter.trim().to_lowercase();
        let mut entries: Vec<&GameSummary> = self
            .summaries
            .values()
            .filter(|summary| summary.matches(&filter) && system.accepts(summary))
            .collect();
        entries.sort_by(|a, b| sort.compare(a, b));
        entries
    }

    /// Get a specific game summary.
    pub fn summary(&self, sha1: &str) -> Option<&GameSummary> {
        self.summaries.get(sha1)
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    fn load_summary(game_dir: PathBuf, entry: GameEntry) -> GameSummary {
        let thumbnail = super::load_thumbnail(&game_dir).map(image::Handle::from_bytes);
        let stats = activity::compute_stats(&game_dir);

        GameSummary {
            entry,
            thumbnail,
            play_time_secs: stats.total_play_time_secs,
            last_played: stats.last_played,
            save_count: stats.save_count,
        }
    }

    // ── Activity detail (detail page) ──────────────────────────────────

    /// Get the activity state for a game.
    pub fn activity_for(&self, sha1: &str) -> &ActivityState {
        match &self.activity_state {
            Some((s, state)) if s == sha1 => state,
            _ => &ActivityState::Loading,
        }
    }

    /// Mark activity as loading for a game (call before kicking off background load).
    pub fn mark_activity_loading(&mut self, sha1: &str) {
        self.activity_state = Some((sha1.to_string(), ActivityState::Loading));
    }

    /// Load raw activity data from disk. Safe to call from a background thread
    /// (no iced image handles created — those happen on the main thread via
    /// `set_raw_activity_detail`).
    pub fn load_raw_activity(sha1: &str, game_dir: &Path) -> RawActivityDetail {
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
                                activity::EventKind::Screenshot { frame } => Some(frame.clone()),
                                _ => None,
                            })
                            .collect();
                        let prints = session
                            .events
                            .iter()
                            .filter_map(|e| match &e.kind {
                                activity::EventKind::Print { print } => Some(print.clone()),
                                _ => None,
                            })
                            .collect();

                        // If the session was never closed (crash/force quit),
                        // estimate end from the last event or fall back to start.
                        let end = session.end.or_else(|| {
                            session.events.last().map(|e| e.at).or(Some(session.start))
                        });

                        Some(RawSessionSummary {
                            filename: r.filename,
                            kind: activity::ActivityKind::Session,
                            start: session.start,
                            end,
                            save_count: session.save_count(),
                            last_save_time: session.last_save_time(),
                            screenshots,
                            prints,
                            size_bytes: None,
                            import_source: None,
                        })
                    }
                    kind @ (activity::ActivityKind::Import
                    | activity::ActivityKind::CartridgeWrite) => {
                        let (suffix, size_bytes, import_source) = match kind {
                            activity::ActivityKind::Import => {
                                let import: activity::ImportSave = ron::from_str(&data).ok()?;
                                (".import", import.size_bytes, Some(import.source))
                            }
                            _ => {
                                let write: activity::CartridgeWrite = ron::from_str(&data).ok()?;
                                (".cart_write", write.size_bytes, None)
                            }
                        };
                        let ts_str = r.filename.strip_suffix(suffix)?;
                        let timestamp = activity::parse_filename_timestamp(ts_str)?;
                        Some(RawSessionSummary {
                            filename: r.filename,
                            kind,
                            start: timestamp,
                            end: None,
                            save_count: 0,
                            last_save_time: None,
                            screenshots: Vec::new(),
                            prints: Vec::new(),
                            size_bytes: Some(size_bytes),
                            import_source,
                        })
                    }
                }
            })
            .collect();

        let last_cart_sync = activity::last_cartridge_sram_hash(game_dir);

        RawActivityDetail {
            sha1: sha1.to_string(),
            sessions,
            last_cart_sync,
        }
    }

    /// Convert raw activity (from background load) into cached ActivityDetail
    /// with rendered image handles. Call on the main thread.
    pub fn set_raw_activity_detail(&mut self, raw: RawActivityDetail) {
        let sessions = raw
            .sessions
            .into_iter()
            .map(|s| SessionSummary {
                filename: s.filename,
                kind: s.kind,
                start: s.start,
                end: s.end,
                save_count: s.save_count,
                last_save_time: s.last_save_time,
                screenshots: s.screenshots.iter().map(|f| f.to_image_handle()).collect(),
                prints: s.prints.iter().map(|p| p.to_image_handle()).collect(),
                size_bytes: s.size_bytes,
                import_source: s.import_source,
            })
            .collect();

        self.activity_state = Some((
            raw.sha1,
            ActivityState::Loaded(ActivityDetail {
                sessions,
                last_cart_sync: raw.last_cart_sync,
            }),
        ));
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

        if current_count > self.live_screenshots.len() {
            // Only render the new ones
            let new_handles: Vec<_> = session
                .events
                .iter()
                .filter_map(|e| match &e.kind {
                    activity::EventKind::Screenshot { frame } => Some(frame.to_image_handle()),
                    _ => None,
                })
                .skip(self.live_screenshots.len())
                .collect();

            self.live_screenshots.extend(new_handles);
        }
    }

    /// Get cached print handles for the live session.
    pub fn live_prints(&self) -> &[image::Handle] {
        &self.live_prints
    }

    /// Update the live print cache from the current session, rendering only
    /// newly added prints.
    pub fn update_live_prints(&mut self, session: &activity::SessionFile) {
        let current_count = session
            .events
            .iter()
            .filter(|e| matches!(e.kind, activity::EventKind::Print { .. }))
            .count();

        if current_count > self.live_prints.len() {
            let new_handles: Vec<_> = session
                .events
                .iter()
                .filter_map(|e| match &e.kind {
                    activity::EventKind::Print { print } => Some(print.to_image_handle()),
                    _ => None,
                })
                .skip(self.live_prints.len())
                .collect();

            self.live_prints.extend(new_handles);
        }
    }

    /// Reset live screenshot and print caches (e.g., when starting a new session).
    pub fn reset_live_screenshots(&mut self) {
        self.live_screenshots.clear();
        self.live_prints.clear();
    }

    // ── Invalidation ───────────────────────────────────────────────────

    /// Called after a session event is written (save, screenshot, session end).
    /// Invalidates activity detail and updates the game summary stats.
    pub fn notify_activity_changed(&mut self, sha1: &str) {
        // Invalidate activity detail if it's for this game
        if matches!(&self.activity_state, Some((s, _)) if s == sha1) {
            self.activity_state = None;
        }

        // Refresh the summary stats for this game
        if let Some(game_dir) = self.index.get(sha1).cloned()
            && let Some(summary) = self.summaries.get_mut(sha1)
        {
            let stats = activity::compute_stats(&game_dir);
            summary.play_time_secs = stats.total_play_time_secs;
            summary.last_played = stats.last_played;
            summary.save_count = stats.save_count;
        }
    }

    /// Called after metadata changes (enrichment, title update).
    pub fn notify_metadata_changed(&mut self, sha1: &str) {
        if let Some(game_dir) = self.index.get(sha1).cloned()
            && let Some(entry) = super::load_entry(&game_dir)
        {
            let thumbnail = super::load_thumbnail(&game_dir).map(image::Handle::from_bytes);
            if let Some(summary) = self.summaries.get_mut(sha1) {
                summary.entry = entry;
                summary.thumbnail = thumbnail;
            }
        }
    }

    /// Called after a game is added to the library.
    pub fn notify_game_added(&mut self, sha1: &str, game_dir: PathBuf) {
        if let Some(entry) = super::load_entry(&game_dir) {
            self.index.insert(sha1.to_string(), game_dir.clone());
            self.summaries
                .insert(sha1.to_string(), Self::load_summary(game_dir, entry));
        }
    }

    /// Called after a game is removed from the library.
    pub fn notify_game_removed(&mut self, sha1: &str) {
        self.index.remove(sha1);
        self.summaries.remove(sha1);

        if matches!(&self.activity_state, Some((s, _)) if s == sha1) {
            self.activity_state = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(title: &str, publisher: Option<&str>, year: Option<&str>) -> GameSummary {
        let mut entry = GameEntry::new(
            format!("sha-{title}"),
            title.to_string(),
            PathBuf::from("/tmp/rom.gb"),
        );
        entry.publisher = publisher.map(str::to_string);
        entry.year = year.map(str::to_string);
        GameSummary {
            entry,
            thumbnail: None,
            play_time_secs: 0.0,
            last_played: None,
            save_count: 0,
        }
    }

    fn titles(summaries: &[&GameSummary]) -> Vec<String> {
        summaries.iter().map(|s| s.entry.title.clone()).collect()
    }

    fn store_with(summaries: Vec<GameSummary>) -> GameStore {
        let mut store = GameStore::empty();
        for summary in summaries {
            store.summaries.insert(summary.entry.sha1.clone(), summary);
        }
        store
    }

    #[test]
    fn last_played_sorts_recent_first_then_titles() {
        let mut played = summary("Zelda", None, None);
        played.last_played = Some(Timestamp::UNIX_EPOCH);
        let store = store_with(vec![
            summary("Metroid II", None, None),
            played,
            summary("Alleyway", None, None),
        ]);
        assert_eq!(
            titles(&store.summaries_sorted(SortKey::LastPlayed, "", SystemFilter::All)),
            ["Zelda", "Alleyway", "Metroid II"]
        );
    }

    #[test]
    fn year_sort_puts_unknown_years_last() {
        let store = store_with(vec![
            summary("B", None, Some("1993")),
            summary("C", None, None),
            summary("A", None, Some("1989")),
        ]);
        assert_eq!(
            titles(&store.summaries_sorted(SortKey::Year, "", SystemFilter::All)),
            ["A", "B", "C"]
        );
    }

    #[test]
    fn most_played_sorts_by_play_time() {
        let mut long = summary("Long", None, None);
        long.play_time_secs = 4000.0;
        let mut short = summary("Short", None, None);
        short.play_time_secs = 10.0;
        let store = store_with(vec![short, long]);
        assert_eq!(
            titles(&store.summaries_sorted(SortKey::MostPlayed, "", SystemFilter::All)),
            ["Long", "Short"]
        );
    }

    #[test]
    fn filter_matches_title_and_publisher_case_insensitively() {
        let store = store_with(vec![
            summary("Wario Land", Some("Nintendo"), None),
            summary("Shantae", Some("Capcom"), None),
        ]);
        assert_eq!(
            titles(&store.summaries_sorted(SortKey::Title, "WARIO", SystemFilter::All)),
            ["Wario Land"]
        );
        assert_eq!(
            titles(&store.summaries_sorted(SortKey::Title, "capcom", SystemFilter::All)),
            ["Shantae"]
        );
        assert_eq!(
            store
                .summaries_sorted(SortKey::Title, "tetris", SystemFilter::All)
                .len(),
            0
        );
        assert_eq!(
            store
                .summaries_sorted(SortKey::Title, "  ", SystemFilter::All)
                .len(),
            2
        );
    }

    #[test]
    fn system_filter_selects_by_platform_and_hides_untagged() {
        use crate::app::system::Platform;
        let mut gb = summary("Wario Land", None, None);
        gb.entry.platform = Some(Platform::GameBoy);
        let mut nes = summary("Metroid", None, None);
        nes.entry.platform = Some(Platform::Nes);
        let untagged = summary("Mystery", None, None);
        let store = store_with(vec![gb, nes, untagged]);

        assert_eq!(
            titles(&store.summaries_sorted(SortKey::Title, "", SystemFilter::All)).len(),
            3
        );
        assert_eq!(
            titles(&store.summaries_sorted(
                SortKey::Title,
                "",
                SystemFilter::System(Platform::GameBoy)
            )),
            ["Wario Land"]
        );
        assert_eq!(
            titles(&store.summaries_sorted(
                SortKey::Title,
                "",
                SystemFilter::System(Platform::Nes)
            )),
            ["Metroid"]
        );
    }
}
