//! The register file as a debugger shows it: three tone channels and one
//! noise channel, each with an audibility pip — attenuation $F fully mutes, so
//! anything below it is audible. The period and attenuation rows carry the
//! arithmetic a reader would otherwise do by hand, which needs the frequency
//! the board drives the CLOCK pin at.

use missingno_core::inspect::{Row, Section, SectionBlock};

use crate::{
    CHANNELS, Channel, DECIBELS_PER_STEP, MUTE_ATTENUATION, NoiseMode, NoiseRate, TONE_CHANNELS,
    Variant,
};

/// The ÷16 prescaler and the two borrows a flip-flop toggle takes.
const TONE_DIVISOR: f32 = 32.0;

/// What the last write to each latch left, plus the part it left it on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Registers {
    pub tone_periods: [u16; TONE_CHANNELS],
    pub attenuations: [u8; CHANNELS],
    pub noise_mode: NoiseMode,
    pub noise_rate: NoiseRate,
    pub variant: Variant,
}

/// `clock_hz` is what the board drives the CLOCK pin at — the tone arithmetic
/// is meaningless without it.
pub fn section(registers: &Registers, clock_hz: u32) -> Section {
    let audible = registers
        .attenuations
        .iter()
        .filter(|&&attenuation| attenuation != MUTE_ATTENUATION)
        .count();
    Section {
        name: "PSG",
        summary: format!("{audible}/4 audible"),
        active: None,
        detail: None,
        blocks: vec![
            tone_rows(registers, Channel::Tone1, clock_hz),
            SectionBlock::Rule,
            tone_rows(registers, Channel::Tone2, clock_hz),
            SectionBlock::Rule,
            tone_rows(registers, Channel::Tone3, clock_hz),
            SectionBlock::Rule,
            noise_rows(registers),
        ],
    }
}

/// One tone channel: the period register with the tone it produces, and the
/// attenuation with its place on the ladder.
fn tone_rows(registers: &Registers, channel: Channel, clock_hz: u32) -> SectionBlock {
    let number = channel.index() + 1;
    let period = registers.tone_periods[channel.index()];
    let attenuation = registers.attenuations[channel.index()];
    SectionBlock::Rows(vec![
        Row::flag(format!("tone {number}"), attenuation != MUTE_ATTENUATION)
            .help("channel audible — attenuation below $F"),
        Row::value(
            format!("per{number}"),
            tone_label(period, registers.variant, clock_hz),
        )
        .help("10-bit period register; the tone is the CLOCK pin ÷ 32n"),
        Row::value(format!("att{number}"), attenuation_label(attenuation))
            .help("attenuator — 2 dB a step, $F switching the channel off"),
    ])
}

/// The noise channel: what its feedback network and its counter's rate are
/// set to, and the same attenuator as a tone channel's.
fn noise_rows(registers: &Registers) -> SectionBlock {
    let attenuation = registers.attenuations[Channel::Noise.index()];
    SectionBlock::Rows(vec![
        Row::flag("noise", attenuation != MUTE_ATTENUATION)
            .help("channel audible — attenuation below $F"),
        Row::value("mode", noise_mode_label(registers.noise_mode))
            .help("feedback network (noise control bit 2)"),
        Row::value("rate", noise_rate_label(registers.noise_rate))
            .help("shift rate (noise control bits 0-1) — a fixed division, or tone 3's period"),
        Row::value("att4", attenuation_label(attenuation))
            .help("attenuator — 2 dB a step, $F switching the channel off"),
    ])
}

/// A period register beside the tone it produces.
fn tone_label(period: u16, variant: Variant, clock_hz: u32) -> String {
    match tone_frequency(period, variant, clock_hz) {
        Some(hertz) => format!("{period:03X} ({} Hz)", hertz.round() as u32),
        // A register the counter never borrows on holds the flip-flop still.
        None => format!("{period:03X} (dc)"),
    }
}

/// The counter reloads its effective count and toggles its flip-flop each
/// borrow, so the tone is the input clock over 32 counts.
fn tone_frequency(period: u16, variant: Variant, clock_hz: u32) -> Option<f32> {
    let count = variant.effective_period(period)?;
    Some(clock_hz as f32 / (TONE_DIVISOR * f32::from(count)))
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

    /// The clock both Sega boards drive the part at.
    const CLOCK_HZ: u32 = 3_579_545;

    #[test]
    fn tone_frequency_is_the_clock_over_thirty_two_counts() {
        let hertz = |period| {
            tone_frequency(period, Variant::DiscreteTi, CLOCK_HZ)
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

    /// The integrated part holds its flip-flop on a zero register where the
    /// discrete part counts a full span.
    #[test]
    fn a_zero_period_reads_per_variant() {
        assert_eq!(tone_label(0, Variant::DiscreteTi, CLOCK_HZ), "000 (109 Hz)");
        assert_eq!(tone_label(0, Variant::SegaIntegrated, CLOCK_HZ), "000 (dc)");
    }
}
