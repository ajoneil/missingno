use std::{fmt, path::PathBuf};

use iced::widget::{column, container, pane_grid, pick_list, text};

use crate::app::{
    self,
    core::{buttons, sizes::m},
    debugger::{
        self,
        panes::{self, DebuggerPane, pane, title_bar},
    },
};

use super::Debugger;

#[derive(Debug, Clone, PartialEq)]
pub struct RecordingFile {
    pub path: PathBuf,
    pub label: String,
}

impl RecordingFile {
    pub fn new(path: PathBuf, last_frame: u64) -> Self {
        let filename = path.file_stem().unwrap().to_string_lossy();
        let timestamp = filename
            .rsplit_once('-')
            .and_then(|(rest, time)| {
                rest.rsplit_once('-').and_then(|(rest, day)| {
                    rest.rsplit_once('-').and_then(|(rest, month)| {
                        rest.rsplit_once('-')
                            .map(|(_, year)| format!("{year}-{month}-{day} {}", format_time(time)))
                    })
                })
            })
            .unwrap_or_else(|| filename.to_string());

        let duration = format_duration(last_frame);
        let label = format!("{timestamp} ({duration})");

        Self { path, label }
    }
}

fn format_time(time: &str) -> String {
    if time.len() == 6 {
        format!("{}:{}:{}", &time[0..2], &time[2..4], &time[4..6])
    } else {
        time.to_string()
    }
}

fn format_duration(frames: u64) -> String {
    let total_seconds = frames / 60;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

impl fmt::Display for RecordingFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label)
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    RecordingSaved(RecordingFile),
    RecordingsScanned(Vec<RecordingFile>),
    SelectRecording(RecordingFile),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(debugger::Message::Pane(panes::Message::Pane(
            panes::PaneMessage::Playback(self),
        )))
    }
}

pub struct PlaybackPane {
    recordings: Vec<RecordingFile>,
    selected_recording: Option<RecordingFile>,
}

impl PlaybackPane {
    pub fn new() -> Self {
        Self {
            recordings: Vec::new(),
            selected_recording: None,
        }
    }

    pub fn update(&mut self, message: &Message) {
        match message {
            Message::RecordingSaved(recording) => {
                self.recordings.push(recording.clone());
                self.selected_recording = Some(recording.clone());
            }
            Message::RecordingsScanned(recordings) => {
                self.recordings = recordings.clone();
                self.selected_recording = None;
            }
            Message::SelectRecording(recording) => {
                self.selected_recording = Some(recording.clone());
            }
        }
    }

    pub fn content(&self, debugger: &Debugger) -> pane_grid::Content<'_, app::Message> {
        let status: String = if debugger.is_recording() {
            format!("Recording — frame {}", debugger.frame())
        } else if debugger.is_playing() {
            let total = debugger
                .playback_total_frames()
                .map(|t| t.to_string())
                .unwrap_or_else(|| "?".into());
            format!("Playing — frame {} / {}", debugger.frame(), total)
        } else {
            "Idle".into()
        };

        let recording_picker = pick_list(
            self.recordings.as_slice(),
            self.selected_recording.as_ref(),
            |recording| Message::SelectRecording(recording).into(),
        )
        .placeholder("Select recording...");

        let can_play =
            self.selected_recording.is_some() && !debugger.is_recording() && !debugger.is_playing();
        let play_button = if can_play {
            buttons::success("Play").on_press(app::Message::StartPlayback(
                self.selected_recording.as_ref().unwrap().path.clone(),
            ))
        } else {
            buttons::success("Play")
        };

        let record_button = if debugger.is_recording() {
            buttons::danger("Stop").on_press(app::Message::StopRecording)
        } else if debugger.is_playing() {
            buttons::standard("Record")
        } else {
            buttons::standard("Record").on_press(app::Message::StartRecording)
        };

        pane(
            title_bar("Playback", DebuggerPane::Playback),
            container(
                column![text(status), recording_picker, play_button, record_button].spacing(m()),
            )
            .padding(m())
            .into(),
        )
    }
}
