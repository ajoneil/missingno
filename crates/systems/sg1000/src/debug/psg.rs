//! The SN76489AN: three tone channels and one noise channel, each with an
//! audibility pip — attenuation $F fully mutes, so anything below it is
//! audible. The period and attenuation rows carry the arithmetic a reader
//! would otherwise do by hand.

use missingno_core::inspect::{Row, Section, SectionBlock};
use missingno_ti_psg::{
    Channel, DECIBELS_PER_STEP, MUTE_ATTENUATION, NoiseMode, NoiseRate, Variant,
};

use super::Sg1000InspectState;
use crate::console::CLOCK_HZ;

/// The ÷16 prescaler and the two borrows a flip-flop toggle takes.
const TONE_DIVISOR: f32 = 32.0;

pub(crate) fn section(state: &Sg1000InspectState) -> Section {
    let audible = state
        .psg_volumes
        .iter()
        .filter(|&&attenuation| attenuation != MUTE_ATTENUATION)
        .count();
    Section {
        name: "PSG",
        summary: format!("{audible}/4 audible"),
        active: None,
        detail: None,
        blocks: vec![
            tone_rows(state, Channel::Tone1),
            SectionBlock::Rule,
            tone_rows(state, Channel::Tone2),
            SectionBlock::Rule,
            tone_rows(state, Channel::Tone3),
            SectionBlock::Rule,
            noise_rows(state),
        ],
    }
}

/// One tone channel: the period register with the tone it produces, and the
/// attenuation with its place on the ladder.
fn tone_rows(state: &Sg1000InspectState, channel: Channel) -> SectionBlock {
    let number = channel.index() + 1;
    let period = state.psg_periods[channel.index()];
    let attenuation = state.psg_volumes[channel.index()];
    SectionBlock::Rows(vec![
        Row::flag(format!("tone {number}"), attenuation != MUTE_ATTENUATION)
            .help("channel audible — attenuation below $F"),
        Row::value(
            format!("per{number}"),
            tone_label(period, state.psg_variant),
        )
        .help("10-bit period register; the tone is the 3.579545 MHz clock ÷ 32n"),
        Row::value(format!("att{number}"), attenuation_label(attenuation))
            .help("attenuator — 2 dB a step, $F switching the channel off"),
    ])
}

/// The noise channel: what its feedback network and its counter's rate are
/// set to, and the same attenuator as a tone channel's.
fn noise_rows(state: &Sg1000InspectState) -> SectionBlock {
    let attenuation = state.psg_volumes[Channel::Noise.index()];
    SectionBlock::Rows(vec![
        Row::flag("noise", attenuation != MUTE_ATTENUATION)
            .help("channel audible — attenuation below $F"),
        Row::value("mode", noise_mode_label(state.psg_noise_mode))
            .help("feedback network (noise control bit 2)"),
        Row::value("rate", noise_rate_label(state.psg_noise_rate))
            .help("shift rate (noise control bits 0-1) — a fixed division, or tone 3's period"),
        Row::value("att4", attenuation_label(attenuation))
            .help("attenuator — 2 dB a step, $F switching the channel off"),
    ])
}

/// A period register beside the tone it produces.
fn tone_label(period: u16, variant: Variant) -> String {
    match tone_frequency(period, variant) {
        Some(hertz) => format!("{period:03X} ({} Hz)", hertz.round() as u32),
        // A register the counter never borrows on holds the flip-flop still.
        None => format!("{period:03X} (dc)"),
    }
}

/// The counter reloads its effective count and toggles its flip-flop each
/// borrow, so the tone is the input clock over 32 counts.
fn tone_frequency(period: u16, variant: Variant) -> Option<f32> {
    let count = variant.effective_period(period)?;
    Some(CLOCK_HZ / (TONE_DIVISOR * f32::from(count)))
}

/// An attenuation register beside the attenuation it sets.
fn attenuation_label(attenuation: u8) -> String {
    if attenuation >= MUTE_ATTENUATION {
        format!("{attenuation:X} (off)")
    } else if attenuation == 0 {
        format!("{attenuation:X} (0 dB)")
    } else {
        format!(
            "{attenuation:X} (-{} dB)",
            f32::from(attenuation) * DECIBELS_PER_STEP
        )
    }
}

fn noise_mode_label(mode: NoiseMode) -> &'static str {
    match mode {
        NoiseMode::White => "white",
        NoiseMode::Periodic => "periodic",
    }
}

fn noise_rate_label(rate: NoiseRate) -> String {
    match rate.input_clock_division() {
        Some(division) => format!("clock ÷ {division}"),
        None => "tone 3".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::fixtures::{power_on_state, rows, value_of};

    #[test]
    fn tone_frequency_is_the_clock_over_thirty_two_counts() {
        let hertz = |period| {
            tone_frequency(period, Variant::DiscreteTi)
                .map(|hertz| hertz.round() as u32)
                .unwrap()
        };
        // 3.579545 MHz / (32 · 254) = 440.4 Hz.
        assert_eq!(hertz(0x0FE), 440);
        assert_eq!(hertz(0x1FE), 219);
        // A zero register counts as $400 on the discrete part.
        assert_eq!(hertz(0), 109);
    }

    #[test]
    fn attenuation_reads_as_decibels() {
        assert_eq!(attenuation_label(0x0), "0 (0 dB)");
        assert_eq!(attenuation_label(0x1), "1 (-2 dB)");
        assert_eq!(attenuation_label(0x5), "5 (-10 dB)");
        assert_eq!(attenuation_label(0xF), "F (off)");
    }

    #[test]
    fn psg_section_pairs_registers_with_their_arithmetic() {
        let state = Sg1000InspectState {
            psg_periods: [0x0FE, 0x1FE, 0],
            // Tone 1 audible (attenuation 0), the rest muted at $F.
            psg_volumes: [0x00, 0x0F, 0x0F, 0x0F],
            psg_noise_mode: NoiseMode::White,
            psg_noise_rate: NoiseRate::Div1024,
            ..power_on_state()
        };
        let section = section(&state);
        assert_eq!(section.name, "PSG");
        assert_eq!(section.summary, "1/4 audible");

        let rows = rows(&section);
        assert_eq!(value_of(&rows, "per1"), Some("0FE (440 Hz)"));
        assert_eq!(value_of(&rows, "att1"), Some("0 (0 dB)"));
        assert_eq!(value_of(&rows, "per2"), Some("1FE (219 Hz)"));
        assert_eq!(value_of(&rows, "att2"), Some("F (off)"));
        // The discrete part's zero period is a full-span count, not silence.
        assert_eq!(value_of(&rows, "per3"), Some("000 (109 Hz)"));
        assert_eq!(value_of(&rows, "mode"), Some("white"));
        assert_eq!(value_of(&rows, "rate"), Some("clock ÷ 1024"));
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "tone 1")
                .and_then(|row| row.active),
            Some(true)
        );
        assert_eq!(
            rows.iter()
                .find(|row| row.label == "tone 2")
                .and_then(|row| row.active),
            Some(false)
        );
    }

    /// The integrated part holds its flip-flop on a zero register where the
    /// discrete part counts a full span.
    #[test]
    fn a_zero_period_reads_per_variant() {
        assert_eq!(tone_label(0, Variant::DiscreteTi), "000 (109 Hz)");
        assert_eq!(tone_label(0, Variant::SegaIntegrated), "000 (dc)");
    }
}
