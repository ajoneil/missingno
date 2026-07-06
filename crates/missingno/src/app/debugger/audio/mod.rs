use iced::widget::{column, pane_grid, row, rule, slider, text};

use crate::app::{
    Message,
    debugger::{
        inspect::AudioView,
        panes::{self, checkbox_title_bar, pane},
    },
    ui::sizes::{l, s},
};

mod channels;

pub struct AudioPane;

impl AudioPane {
    pub fn new() -> Self {
        Self
    }

    pub fn content(&self, audio: &AudioView) -> pane_grid::Content<'_, Message> {
        pane(
            checkbox_title_bar("Audio", audio.enabled),
            column![
                row![
                    column![
                        text("Left"),
                        slider(0..=7, audio.volume_left, |_| -> Message { Message::None })
                    ],
                    column![
                        text("Right"),
                        slider(0..=7, audio.volume_right, |_| -> Message { Message::None })
                    ]
                ]
                .spacing(l()),
                row![
                    channels::envelope_channel("Channel 1", &audio.ch1),
                    rule::vertical(1),
                    channels::envelope_channel("Channel 2", &audio.ch2),
                    rule::vertical(1),
                    channels::wave_channel("Channel 3", &audio.ch3),
                    rule::vertical(1),
                    channels::envelope_channel("Channel 4", &audio.ch4),
                ]
                .spacing(s())
            ]
            .spacing(s())
            .into(),
        )
    }
}

impl panes::Pane for AudioPane {
    fn kind(&self) -> panes::DebuggerPane {
        panes::DebuggerPane::Audio
    }

    fn view<'a>(&'a self, ctx: Option<&panes::PaneContext<'_>>) -> pane_grid::Content<'a, Message> {
        match ctx.and_then(|ctx| ctx.gb) {
            Some(source) => self.content(&source.audio()),
            None => panes::running_placeholder("Audio"),
        }
    }
}
