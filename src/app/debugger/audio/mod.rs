use iced::widget::{column, pane_grid, row, slider, text, vertical_rule};

use crate::{
    app::{
        Message,
        core::sizes::{l, s},
        debugger::panes::{DebuggerPane, checkbox_title_bar, pane},
    },
    emulator::audio::Audio,
};

mod channels;

pub struct AudioPane;

impl AudioPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(&self, audio: &Audio) -> pane_grid::Content<'_, Message> {
        pane(
            checkbox_title_bar("Audio", audio.enabled(), DebuggerPane::Audio),
            column![
                row![
                    column![
                        text("Left"),
                        slider(0..=7, audio.volume_left().0, |_| -> Message {
                            Message::None
                        })
                    ],
                    column![
                        text("Right"),
                        slider(0..=7, audio.volume_right().0, |_| -> Message {
                            Message::None
                        })
                    ]
                ]
                .spacing(l()),
                row![
                    channels::ch1(&audio.channels().ch1),
                    vertical_rule(1),
                    channels::ch2(&audio.channels().ch2),
                    vertical_rule(1),
                    channels::ch3(&audio.channels().ch3),
                    vertical_rule(1),
                    channels::ch4(&audio.channels().ch4),
                ]
                .spacing(s())
            ]
            .spacing(s())
            .into(),
        )
    }
}
