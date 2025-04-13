use crate::{
    emulator::audio::channels::{
        noise::NoiseChannel, pulse::PulseChannel, pulse_sweep::PulseSweepChannel, wave::WaveChannel,
    },
    ui::Message,
};
use iced::{
    Element, Length,
    widget::{checkbox, column},
};

pub fn ch1(channel: &PulseSweepChannel) -> Element<'_, Message> {
    column![checkbox("Channel 1", channel.enabled())]
        .width(Length::Fill)
        .into()
}

pub fn ch2(channel: &PulseChannel) -> Element<'_, Message> {
    column![checkbox("Channel 2", channel.enabled())]
        .width(Length::Fill)
        .into()
}

pub fn ch3(channel: &WaveChannel) -> Element<'_, Message> {
    column![checkbox("Channel 3", channel.enabled())]
        .width(Length::Fill)
        .into()
}

pub fn ch4(channel: &NoiseChannel) -> Element<'_, Message> {
    column![checkbox("Channel 4", channel.enabled())]
        .width(Length::Fill)
        .into()
}
