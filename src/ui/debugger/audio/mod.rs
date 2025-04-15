mod channels;

use super::panes::{checkbox_title_bar, pane};
use crate::{
    emulator::audio::Audio,
    ui::{Message, styles::spacing},
};
use iced::widget::{column, pane_grid, row, slider, text, vertical_rule};

pub fn audio_pane(audio: &Audio) -> pane_grid::Content<'_, Message> {
    pane(
        checkbox_title_bar("Audio", audio.enabled()),
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
            .spacing(spacing::l()),
            row![
                channels::ch1(&audio.channels().ch1),
                vertical_rule(1),
                channels::ch2(&audio.channels().ch2),
                vertical_rule(1),
                channels::ch3(&audio.channels().ch3),
                vertical_rule(1),
                channels::ch4(&audio.channels().ch4),
            ]
            .spacing(spacing::s())
        ]
        .spacing(spacing::s())
        .into(),
    )
}
