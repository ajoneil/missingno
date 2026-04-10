use iced::{
    Alignment::Center,
    widget::{column, container, pane_grid, row, text},
};

use crate::app::{
    self,
    ui::{
        buttons,
        icons::{self, Icon},
        sizes::{m, s},
    },
    debugger::panes::{DebuggerPane, pane, title_bar},
};

use super::Debugger;

pub struct PlaybackPane;

impl PlaybackPane {
    pub fn new() -> Self {
        Self
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

        let can_play =
            debugger.has_recording() && !debugger.is_recording() && !debugger.is_playing();
        let play_label = row![icons::m(Icon::Play), "Play"]
            .spacing(s())
            .align_y(Center);
        let play_button = if can_play {
            buttons::primary(play_label).on_press(app::Message::StartPlayback)
        } else {
            buttons::primary(play_label)
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
            container(column![text(status), play_button, record_button].spacing(m()))
                .padding(m())
                .into(),
        )
    }
}
