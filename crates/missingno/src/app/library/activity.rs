use std::{
    fs,
    path::{Path, PathBuf},
};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

// ── Data structures ────────────────────────────────────────────────────

/// A play session: start/end times, what save we started from, and a
/// chronological log of events (saves, screenshots, etc.).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionFile {
    pub start: Timestamp,
    pub end: Option<Timestamp>,
    /// Filename stem of the activity entry we started from (e.g. "20260403-083646.import").
    pub parent: Option<String>,
    #[serde(default)]
    pub events: Vec<SessionEvent>,

    // Legacy field: old session files stored saves here. On deserialization,
    // these are migrated into `events` by `migrate_legacy_saves()`.
    #[serde(default, skip_serializing)]
    saves: Vec<LegacySessionSave>,
}

impl SessionFile {
    pub fn new(start: Timestamp, parent: Option<String>) -> Self {
        Self {
            start,
            end: None,
            parent,
            events: Vec::new(),
            saves: Vec::new(),
        }
    }

    /// Migrate legacy `saves` field into unified `events`. Called after deserialization.
    fn migrate_legacy_saves(&mut self) {
        if !self.saves.is_empty() && self.events.is_empty() {
            self.events = self
                .saves
                .drain(..)
                .map(|s| SessionEvent {
                    at: s.at,
                    kind: EventKind::Save { sram: s.sram },
                })
                .collect();
        }
    }

    /// Helper: iterate only save events.
    pub fn saves(&self) -> impl Iterator<Item = &SessionEvent> {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Save { .. }))
    }

    /// Helper: get the SRAM from the last save event.
    pub fn last_sram(&self) -> Option<&[u8]> {
        self.events.iter().rev().find_map(|e| match &e.kind {
            EventKind::Save { sram } => Some(sram.as_slice()),
            _ => None,
        })
    }

    /// Helper: count of save events.
    pub fn save_count(&self) -> usize {
        self.saves().count()
    }

    /// Helper: timestamp of the last save event.
    pub fn last_save_time(&self) -> Option<Timestamp> {
        self.events.iter().rev().find_map(|e| match &e.kind {
            EventKind::Save { .. } => Some(e.at),
            _ => None,
        })
    }

    /// Helper: count of screenshot events.
    pub fn screenshot_count(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Screenshot { .. }))
            .count()
    }
}

/// A timestamped event that occurred during a session.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionEvent {
    pub at: Timestamp,
    pub kind: EventKind,
}

/// What kind of event occurred.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum EventKind {
    /// SRAM changed.
    Save { sram: Vec<u8> },
    /// Player captured a screenshot.
    Screenshot { frame: FrameCapture },
}

/// A captured frame: the PPU's shade output plus display context for re-rendering.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FrameCapture {
    /// 160×144 shade values (0-3), flattened row-major from Framebuffer.
    pub pixels: Vec<u8>,
    /// Which display palette the user had active at capture time.
    pub user_palette: String,
}

impl FrameCapture {
    pub fn from_framebuffer(
        fb: &missingno_gb::ppu::screen::Framebuffer,
        palette_name: &str,
    ) -> Self {
        use missingno_gb::ppu::screen::{NUM_SCANLINES, PIXELS_PER_LINE};

        let mut pixels =
            Vec::with_capacity(PIXELS_PER_LINE as usize * NUM_SCANLINES as usize);
        for y in 0..NUM_SCANLINES as usize {
            for x in 0..PIXELS_PER_LINE as usize {
                pixels.push(fb.pixels[y][x].0);
            }
        }
        Self {
            pixels,
            user_palette: palette_name.to_string(),
        }
    }
}

// Legacy save struct for deserializing old session files.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct LegacySessionSave {
    at: Timestamp,
    sram: Vec<u8>,
}

/// An imported save file.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ImportFile {
    pub size_bytes: u32,
    pub sram: Vec<u8>,
}

/// Lightweight reference to an activity file, parsed from the filename.
#[derive(Clone, Debug)]
pub struct ActivityRef {
    pub filename: String,
    pub kind: ActivityKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActivityKind {
    Session,
    Import,
}

/// Summary data for an activity entry, suitable for UI display
/// without loading full SRAM data.
#[derive(Clone, Debug)]
pub struct ActivityDisplay {
    pub filename: String,
    pub kind: ActivityKind,
    pub timestamp: Timestamp,
    // Session fields
    pub end: Option<Timestamp>,
    pub save_count: usize,
    pub last_save_time: Option<Timestamp>,
    pub screenshot_count: usize,
    // Import fields
    pub size_bytes: Option<u32>,
}

/// Aggregate stats derived from all activity files.
pub struct ActivityStats {
    pub total_play_time_secs: f64,
    pub last_played: Option<Timestamp>,
    pub save_count: usize,
}

// ── Filesystem paths ───────────────────────────────────────────────────

fn activity_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("activity")
}

fn activity_path(game_dir: &Path, filename: &str) -> PathBuf {
    activity_dir(game_dir).join(filename)
}

fn timestamp_prefix(ts: &Timestamp) -> String {
    ts.strftime("%Y%m%d-%H%M%S").to_string()
}

// ── Listing ────────────────────────────────────────────────────────────

/// List all activity entries, sorted newest first.
pub fn list_activity(game_dir: &Path) -> Vec<ActivityRef> {
    let dir = activity_dir(game_dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut refs: Vec<ActivityRef> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let kind = if name.ends_with(".session") {
                ActivityKind::Session
            } else if name.ends_with(".import") {
                ActivityKind::Import
            } else {
                return None;
            };
            Some(ActivityRef {
                filename: name,
                kind,
            })
        })
        .collect();

    // Sort by filename (chronological since timestamps are fixed-width), newest first
    refs.sort_by(|a, b| b.filename.cmp(&a.filename));
    refs
}

/// Load display data for all activity entries.
pub fn load_activity_display(game_dir: &Path) -> Vec<ActivityDisplay> {
    list_activity(game_dir)
        .into_iter()
        .filter_map(|r| {
            let data = read_compressed(&activity_path(game_dir, &r.filename))?;
            match r.kind {
                ActivityKind::Session => {
                    let session = read_session_from_str(&data)?;
                    let timestamp = session.start;
                    Some(ActivityDisplay {
                        filename: r.filename,
                        kind: ActivityKind::Session,
                        timestamp,
                        end: session.end,
                        save_count: session.save_count(),
                        last_save_time: session.last_save_time(),
                        screenshot_count: session.screenshot_count(),
                        size_bytes: None,
                    })
                }
                ActivityKind::Import => {
                    let import: ImportFile = ron::from_str(&data).ok()?;
                    // Parse timestamp from filename: "20260403-083646.import"
                    let ts_str = r.filename.strip_suffix(".import")?;
                    let timestamp = parse_filename_timestamp(ts_str)?;
                    Some(ActivityDisplay {
                        filename: r.filename,
                        kind: ActivityKind::Import,
                        timestamp,
                        end: None,
                        save_count: 0,
                        last_save_time: None,
                        screenshot_count: 0,
                        size_bytes: Some(import.size_bytes),
                    })
                }
            }
        })
        .collect()
}

// ── Reading ────────────────────────────────────────────────────────────

/// Deserialize a SessionFile from RON, handling legacy format migration.
fn read_session_from_str(data: &str) -> Option<SessionFile> {
    let mut session: SessionFile = ron::from_str(data).ok()?;
    session.migrate_legacy_saves();
    Some(session)
}

/// Load the SRAM from a specific activity file.
/// For sessions, returns the last save's SRAM. For imports, returns the imported SRAM.
pub fn load_sram_from(game_dir: &Path, filename: &str) -> Option<Vec<u8>> {
    let data = read_compressed(&activity_path(game_dir, filename))?;
    if filename.ends_with(".session") {
        let session = read_session_from_str(&data)?;
        session.last_sram().map(|s| s.to_vec())
    } else if filename.ends_with(".import") {
        let import: ImportFile = ron::from_str(&data).ok()?;
        Some(import.sram)
    } else {
        None
    }
}

/// Load the most recent SRAM across all activity files.
pub fn load_current_sram(game_dir: &Path) -> Option<Vec<u8>> {
    // Activity is sorted newest first; find the first entry with SRAM
    for r in list_activity(game_dir) {
        if let Some(sram) = load_sram_from(game_dir, &r.filename) {
            return Some(sram);
        }
    }
    None
}

// ── Writing ────────────────────────────────────────────────────────────

/// Write (or overwrite) a session file. Called on session start, each save, and session end.
pub fn write_session(game_dir: &Path, session: &SessionFile) {
    let dir = activity_dir(game_dir);
    let _ = fs::create_dir_all(&dir);

    let filename = format!("{}.session", timestamp_prefix(&session.start));
    let path = dir.join(&filename);

    if let Ok(ron_data) = ron::ser::to_string_pretty(session, ron::ser::PrettyConfig::default()) {
        write_compressed(&path, &ron_data);
    }
}

/// Write an import file.
pub fn write_import(game_dir: &Path, sram: &[u8]) -> String {
    let dir = activity_dir(game_dir);
    let _ = fs::create_dir_all(&dir);

    let now = Timestamp::now();
    let filename = format!("{}.import", timestamp_prefix(&now));
    let path = dir.join(&filename);

    let import = ImportFile {
        size_bytes: sram.len() as u32,
        sram: sram.to_vec(),
    };

    if let Ok(ron_data) = ron::ser::to_string_pretty(&import, ron::ser::PrettyConfig::default()) {
        write_compressed(&path, &ron_data);
    }

    filename
}

/// Import a legacy .sav file found next to a ROM.
pub fn import_legacy_sav(game_dir: &Path, sav_path: &Path) -> bool {
    let Ok(data) = fs::read(sav_path) else {
        return false;
    };

    // Don't re-import if we already have activity
    if !list_activity(game_dir).is_empty() {
        return false;
    }

    write_import(game_dir, &data);
    true
}

// ── Stats ──────────────────────────────────────────────────────────────

/// Compute aggregate stats by scanning all activity files.
pub fn compute_stats(game_dir: &Path) -> ActivityStats {
    let mut total_secs = 0.0;
    let mut last_played: Option<Timestamp> = None;
    let mut save_count = 0usize;

    for r in list_activity(game_dir) {
        let Some(data) = read_compressed(&activity_path(game_dir, &r.filename)) else {
            continue;
        };

        match r.kind {
            ActivityKind::Session => {
                let Some(session) = read_session_from_str(&data) else {
                    continue;
                };

                let session_end = session.end.unwrap_or(session.start);
                if last_played.is_none() || Some(session_end) > last_played {
                    last_played = Some(session_end);
                }

                // Accumulate play time
                if let Some(end) = session.end {
                    total_secs += end.duration_since(session.start).as_secs_f64();
                }

                save_count += session.save_count();
            }
            ActivityKind::Import => {
                save_count += 1;
            }
        }
    }

    ActivityStats {
        total_play_time_secs: total_secs,
        last_played,
        save_count,
    }
}

/// Format total play time as human-readable string.
pub fn format_play_time(total_secs: f64) -> String {
    let secs = total_secs as u64;
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        "< 1m".to_string()
    }
}

// ── Compression helpers ────────────────────────────────────────────────

fn write_compressed(path: &Path, ron_data: &str) {
    if let Ok(compressed) = zstd::encode_all(ron_data.as_bytes(), 3) {
        let _ = fs::write(path, compressed);
    }
}

fn read_compressed(path: &Path) -> Option<String> {
    let compressed = fs::read(path).ok()?;
    let decompressed = zstd::decode_all(compressed.as_slice()).ok()?;
    String::from_utf8(decompressed).ok()
}

// ── Timestamp parsing ──────────────────────────────────────────────────

fn parse_filename_timestamp(s: &str) -> Option<Timestamp> {
    // Format: "20260403-083646"
    if s.len() < 15 {
        return None;
    }
    let iso = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &s[0..4],
        &s[4..6],
        &s[6..8],
        &s[9..11],
        &s[11..13],
        &s[13..15],
    );
    iso.parse().ok()
}

// ── Time formatting (preserved from saves.rs) ──────────────────────────

/// Format a timestamp as locale-aware date + time (e.g. "3 Apr 2026, 2:32 PM").
pub fn format_local(ts: &Timestamp) -> String {
    libc_strftime("%e %b %Y, %X", ts.as_second())
}

/// Format a timestamp as locale-aware time only (e.g. "2:32 PM").
pub fn format_local_time(ts: &Timestamp) -> String {
    libc_strftime("%X", ts.as_second())
}

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
