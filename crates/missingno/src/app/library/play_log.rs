use std::path::Path;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PlayLog {
    pub first_played: Option<Timestamp>,
    pub last_played: Option<Timestamp>,
    #[serde(default)]
    pub total_play_time_secs: f64,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub save_events: Vec<SaveEvent>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Session {
    pub start: Timestamp,
    pub end: Option<Timestamp>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SaveEvent {
    pub timestamp: Timestamp,
    /// Size of the SRAM data in bytes at time of save.
    pub size_bytes: u32,
}

impl Default for PlayLog {
    fn default() -> Self {
        Self {
            first_played: None,
            last_played: None,
            total_play_time_secs: 0.0,
            sessions: Vec::new(),
            save_events: Vec::new(),
        }
    }
}

impl PlayLog {
    pub fn start_session(&mut self) {
        let now = Timestamp::now();
        if self.first_played.is_none() {
            self.first_played = Some(now);
        }
        self.last_played = Some(now);
        self.sessions.push(Session {
            start: now,
            end: None,
        });
    }

    pub fn end_session(&mut self) {
        let now = Timestamp::now();
        self.last_played = Some(now);
        if let Some(session) = self.sessions.last_mut() {
            if session.end.is_none() {
                session.end = Some(now);
                let duration = now.duration_since(session.start);
                self.total_play_time_secs += duration.as_secs_f64();
            }
        }
    }

    pub fn record_save(&mut self, size_bytes: u32) {
        let now = Timestamp::now();
        self.last_played = Some(now);
        self.save_events.push(SaveEvent {
            timestamp: now,
            size_bytes,
        });
    }

    #[allow(dead_code)]
    pub fn format_play_time(&self) -> String {
        let total_secs = self.total_play_time_secs as u64;
        let hours = total_secs / 3600;
        let minutes = (total_secs % 3600) / 60;
        if hours > 0 {
            format!("{hours}h {minutes}m")
        } else if minutes > 0 {
            format!("{minutes}m")
        } else {
            "< 1m".to_string()
        }
    }
}

pub fn load(game_dir: &Path) -> PlayLog {
    let path = game_dir.join("play_log.ron");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|data| ron::from_str(&data).ok())
        .unwrap_or_default()
}

pub fn save(game_dir: &Path, log: &PlayLog) {
    let path = game_dir.join("play_log.ron");
    if let Ok(data) = ron::ser::to_string_pretty(log, ron::ser::PrettyConfig::default()) {
        let _ = std::fs::write(path, data);
    }
}
