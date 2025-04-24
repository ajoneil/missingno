use iced::{
    Element, Length,
    widget::{checkbox, column, row, text},
};
use plotters::series::{AreaSeries, LineSeries};
use plotters_iced::{Chart, ChartWidget};
use spectrum_analyzer::{
    FrequencyLimit, FrequencySpectrum, samples_fft_to_spectrum, scaling::divide_by_N_sqrt,
    windows::hann_window,
};

use crate::{
    app::Message,
    emulator::audio::channels::{self, Enabled},
};

pub struct PulseSweepChannel {
    chart: ChannelChart,
}

impl PulseSweepChannel {
    pub fn new() -> Self {
        Self {
            chart: ChannelChart::new(),
        }
    }

    pub fn view(&self, channel: &channels::pulse_sweep::PulseSweepChannel) -> Element<'_, Message> {
        column![enabled("Channel 1", &channel.enabled), self.chart.view()].into()
    }

    pub fn update_data(&mut self, data: &[f32]) {
        self.chart.update_data(data);
    }
}

struct ChannelChart {
    data: Option<FrequencySpectrum>,
}

impl ChannelChart {
    fn new() -> Self {
        Self { data: None }
    }

    fn update_data(&mut self, data: &[f32]) {
        if data.len() > 2 {
            let num_samples = data.len().next_power_of_two() >> 1;

            self.data = Some(
                samples_fft_to_spectrum(&data[0..num_samples], 4194304, FrequencyLimit::All, None)
                    .unwrap(),
            );
        }
    }

    fn view(&self) -> Element<'_, Message> {
        ChartWidget::new(self)
            .width(Length::Fill)
            .height(Length::Fixed(200.0))
            .into()
    }
}

impl Chart<Message> for ChannelChart {
    type State = ();

    fn build_chart<DB: plotters_iced::DrawingBackend>(
        &self,
        _state: &Self::State,
        mut builder: plotters_iced::ChartBuilder<DB>,
    ) {
        if let Some(data) = &self.data {
            let mut chart = builder
                .build_cartesian_2d(
                    data.min_fr().val()..data.max_fr().val(),
                    data.min().1.val()..data.max().1.val(),
                )
                .unwrap();
            chart.configure_mesh().draw().unwrap();

            chart
                .draw_series(LineSeries::new(
                    data.data()
                        .iter()
                        .map(|(frequency, value)| (frequency.val(), value.val())),
                    &plotters::style::colors::BLUE,
                ))
                .unwrap();
        }
    }
}

// pub fn ch2(channel: &PulseChannel) -> Element<'static, Message> {
//     column![
//         enabled("Channel 2", &channel.enabled),
//         volume_and_envelope(channel.volume_and_envelope)
//     ]
//     .into()
// }

// pub fn ch3(channel: &WaveChannel) -> Element<'static, Message> {
//     column![
//         enabled("Channel 3", &channel.enabled),
//         text!("Vol {}%", (channel.volume.volume() * 100.0) as u8)
//     ]
//     .into()
// }

// pub fn ch4(channel: &NoiseChannel) -> Element<'static, Message> {
//     column![
//         enabled("Channel 4", &channel.enabled),
//         volume_and_envelope(channel.volume_and_envelope)
//     ]
//     .into()
// }

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

// fn volume_and_envelope(register: VolumeAndEnvelope) -> Element<'static, Message> {
//     if register.sweep_pace() == 0 {
//         text!("Vol static")
//     } else {
//         text(format!(
//             "Vol {} from {}%@{}Hz",
//             match register.direction() {
//                 EnvelopeDirection::Increase => "up",
//                 EnvelopeDirection::Decrease => "down",
//             },
//             (register.initial_volume_percent() * 100.0) as u8,
//             64 / register.sweep_pace()
//         ))
//     }
//     .into()
// }
