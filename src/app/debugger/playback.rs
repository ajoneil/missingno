use iced::widget::{column, container, pane_grid, text};

use crate::app::{
    self,
    core::{buttons, sizes::m},
    debugger::{
        self,
        panes::{self, DebuggerPane, pane, title_bar},
    },
};

use super::Debugger;

#[derive(Debug, Clone)]
pub enum Message {
    RecordingSaved(String),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        app::Message::Debugger(debugger::Message::Pane(panes::Message::Pane(
            panes::PaneMessage::Playback(self),
        )))
    }
}

pub struct PlaybackPane {
    last_saved_recording: Option<String>,
}

impl PlaybackPane {
    pub fn new() -> Self {
        Self {
            last_saved_recording: None,
        }
    }

    pub fn update(&mut self, message: &Message) {
        match message {
            Message::RecordingSaved(filename) => {
                self.last_saved_recording = Some(filename.clone());
            }
        }
    }

    pub fn content(&self, debugger: &Debugger) -> pane_grid::Content<'_, app::Message> {
        let status: String = if debugger.is_recording() {
            format!("Recording — frame {}", debugger.frame())
        } else if let Some(filename) = &self.last_saved_recording {
            format!("Saved {filename}")
        } else {
            "Idle".into()
        };

        let button = if debugger.is_recording() {
            buttons::danger("Stop").on_press(app::Message::StopRecording)
        } else {
            buttons::standard("Record").on_press(app::Message::StartRecording)
        };

        pane(
            title_bar("Playback", DebuggerPane::Playback),
            container(column![text(status), button].spacing(m()))
                .padding(m())
                .into(),
        )
    }
}
