mod channels;

use crate::{emulator::audio::Audio, ui::Message};
use iced::{
    Element,
    widget::{checkbox, column, row, slider, text, vertical_rule},
};

pub fn audio(audio: &Audio) -> Element<'_, Message> {
    column![
        checkbox("Audio", audio.enabled()),
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
        .spacing(20),
        row![
            channels::ch1(&audio.channels().ch1),
            vertical_rule(1),
            channels::ch2(&audio.channels().ch2),
            vertical_rule(1),
            channels::ch3(&audio.channels().ch3),
            vertical_rule(1),
            channels::ch4(&audio.channels().ch4),
        ]
        .spacing(5)
    ]
    .spacing(5)
    .into()
}
