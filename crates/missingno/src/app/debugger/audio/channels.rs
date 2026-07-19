use iced::{
    Element, Length,
    widget::{checkbox, column, row, text},
};

use crate::app::Message;
use crate::app::debugger::inspect::WaveChannelView;
use missingno_gb::audio::channels::{
    Enabled,
    registers::{EnvelopeDirection, VolumeAndEnvelope},
};

pub fn envelope_channel(
    label: &'static str,
    enabled_state: Enabled,
    register: VolumeAndEnvelope,
) -> Element<'static, Message> {
    column![enabled(label, enabled_state), volume_and_envelope(register)].into()
}

pub fn wave_channel(label: &'static str, channel: &WaveChannelView) -> Element<'static, Message> {
    column![
        enabled(label, channel.enabled),
        text!("Vol {}%", (channel.volume * 100.0) as u8)
    ]
    .into()
}

pub fn enabled(label: &'static str, enabled: Enabled) -> Element<'static, Message> {
    column![
        checkbox(enabled.enabled).label(label),
        row![
            checkbox(enabled.output_left)
                .label("Left")
                .width(Length::Fill),
            checkbox(enabled.output_right)
                .label("Right")
                .width(Length::Fill)
        ]
    ]
    .width(Length::Fill)
    .into()
}

fn volume_and_envelope(register: VolumeAndEnvelope) -> Element<'static, Message> {
    if register.sweep_pace() == 0 {
        text!("Vol static")
    } else {
        text(format!(
            "Vol {} from {}%@{}Hz",
            match register.direction() {
                EnvelopeDirection::Increase => "up",
                EnvelopeDirection::Decrease => "down",
            },
            (register.initial_volume_percent() * 100.0) as u8,
            64 / register.sweep_pace()
        ))
    }
    .into()
}
