use iced::widget::{column, pane_grid, row, slider, text};

use crate::{
    app::{
        self,
        core::sizes::{l, s},
        debugger::{
            self,
            panes::{self, DebuggerPane, checkbox_title_bar, pane},
        },
    },
    emulator::audio::Audio,
};

mod channels;

#[derive(Debug, Clone)]
pub enum Message {
    UpdateCharts(Vec<f32>),
}

impl Into<app::Message> for Message {
    fn into(self) -> app::Message {
        panes::PaneMessage::Audio(self).into()
    }
}

pub struct AudioPane {
    ch1: channels::PulseSweepChannel,
}

impl AudioPane {
    pub fn new() -> Self {
        Self {
            ch1: channels::PulseSweepChannel::new(),
        }
    }

    pub fn content(&self, audio: &Audio) -> pane_grid::Content<'_, app::Message> {
        pane(
            checkbox_title_bar("Audio", audio.enabled(), DebuggerPane::Audio),
            column![
                // row![
                //     column![
                //         text("Left"),
                //         slider(0..=7, audio.volume_left().0, |_| -> app::Message {
                //             app::Message::None
                //         })
                //     ],
                //     column![
                //         text("Right"),
                //         slider(0..=7, audio.volume_right().0, |_| -> app::Message {
                //             app::Message::None
                //         })
                //     ]
                // ]
                // .spacing(l()),
                row![
                    self.ch1.view(&audio.channels().ch1),
                    // vertical_rule(1),
                    // channels::ch2(&audio.channels().ch2),
                    // vertical_rule(1),
                    // channels::ch3(&audio.channels().ch3),
                    // vertical_rule(1),
                    // channels::ch4(&audio.channels().ch4),
                ]
                .spacing(s())
            ]
            .spacing(s())
            .into(),
        )
    }

    pub fn update(&mut self, message: &Message) {
        match message {
            Message::UpdateCharts(data) => {
                self.ch1.update_data(&data);
            }
        }
    }
}
