use iced::{
    Element, Length,
    widget::{checkbox, column, row, text},
};

use crate::{
    app::Message,
    emulator::audio::channels::{
        Enabled,
        noise::NoiseChannel,
        pulse::PulseChannel,
        pulse_sweep::PulseSweepChannel,
        registers::{EnvelopeDirection, VolumeAndEnvelope},
        wave::WaveChannel,
    },
};

pub fn ch1(channel: &PulseSweepChannel) -> Element<'_, Message> {
    column![
        enabled("Channel 1", &channel.enabled),
        volume_and_envelope(channel.volume_and_envelope)
    ]
    .into()
}

pub fn ch2(channel: &PulseChannel) -> Element<'_, Message> {
    column![
        enabled("Channel 2", &channel.enabled),
        volume_and_envelope(channel.volume_and_envelope)
    ]
    .into()
}

pub fn ch3(channel: &WaveChannel) -> Element<'_, Message> {
    column![
        enabled("Channel 3", &channel.enabled),
        text!("Vol {}%", (channel.volume.volume() * 100.0) as u8)
    ]
    .into()
}

pub fn ch4(channel: &NoiseChannel) -> Element<'_, Message> {
    column![
        enabled("Channel 4", &channel.enabled),
        volume_and_envelope(channel.volume_and_envelope)
    ]
    .into()
}

pub fn enabled(label: &str, enabled: &Enabled) -> Element<'static, Message> {
    column![
        checkbox(label, enabled.enabled),
        row![
            checkbox("Left", enabled.output_left).width(Length::Fill),
            checkbox("Right", enabled.output_right).width(Length::Fill)
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
