use crate::{
    emulator::audio::channels::{
        Channel, noise::NoiseChannel, pulse::PulseChannel, pulse_sweep::PulseSweepChannel,
        wave::WaveChannel,
    },
    ui::Message,
};
use iced::{
    Element, Length,
    widget::{checkbox, column, row},
};

pub fn ch1(channel: &PulseSweepChannel) -> Element<'_, Message> {
    channel_shared("Channel 1", &channel.channel)
}

pub fn ch2(channel: &PulseChannel) -> Element<'_, Message> {
    channel_shared("Channel 2", &channel.channel)
}

pub fn ch3(channel: &WaveChannel) -> Element<'_, Message> {
    channel_shared("Channel 3", &channel.channel)
}

pub fn ch4(channel: &NoiseChannel) -> Element<'_, Message> {
    channel_shared("Channel 4", &channel.channel)
}

pub fn channel_shared(label: &str, channel: &Channel) -> Element<'static, Message> {
    column![
        checkbox(label, channel.enabled),
        row![
            checkbox("Left", channel.output_left).width(Length::Fill),
            checkbox("Right", channel.output_right).width(Length::Fill)
        ]
    ]
    .width(Length::Fill)
    .into()
}
