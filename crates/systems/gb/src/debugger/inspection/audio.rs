use missingno_core::inspect;

use crate::audio::{
    ApuSpec, Audio,
    channels::{Enabled, registers::VolumeAndEnvelope},
};

/// The APU state the audio pane and the sidebar's APU section draw, captured as
/// plain data so both serve a live [`Audio`] and its snapshot copy. Each view
/// carries the channel's register bytes plus the honest runtime summaries the
/// core already tracks (envelope volume, length counter, wave position).
#[derive(Clone)]
pub struct AudioView {
    /// NR52 power bit.
    pub enabled: bool,
    pub volume_left: u8,
    pub volume_right: u8,
    /// NR50 master volume / VIN byte.
    pub nr50: u8,
    /// Frame-sequencer step (0-7): the DIV-APU divider's phase.
    pub frame_sequencer_step: u8,
    /// The DIV bit the frame sequencer last sampled — its falling edge clocks
    /// the sequencer.
    pub prev_div_apu_bit: bool,
    pub ch1: PulseChannelView,
    pub ch2: PulseChannelView,
    pub ch3: WaveChannelView,
    pub ch4: NoiseChannelView,
}

/// A pulse channel (CH1 with its sweep, CH2 without).
#[derive(Clone, Copy)]
pub struct PulseChannelView {
    pub enabled: Enabled,
    /// NR10 sweep byte — present on CH1 only.
    pub sweep: Option<u8>,
    /// NR11/NR21 wave-duty and initial-length byte.
    pub duty_and_length: u8,
    /// NR12/NR22 volume-and-envelope byte.
    pub volume_and_envelope: VolumeAndEnvelope,
    /// 11-bit period (NR13 low, NR14 high three bits).
    pub period: u16,
    /// NRx4 length-enable bit.
    pub length_enabled: bool,
    pub length_counter: u16,
    /// Current envelope output volume (0-15).
    pub envelope_volume: u8,
    /// Envelope period counter — steps the volume when it reaches the pace.
    pub envelope_timer: u8,
    /// CH1 sweep shadow frequency, or `None` on the sweepless CH2.
    pub shadow_frequency: Option<u16>,
    /// CH1 sweep period counter, or `None` on CH2.
    pub sweep_timer: Option<u8>,
    /// CH1 sweep enabled, or `None` on CH2.
    pub sweep_enabled: Option<bool>,
    /// CH1 sweep has performed a negate calculation, or `None` on CH2.
    pub sweep_negate_used: Option<bool>,
}

#[derive(Clone, Copy)]
pub struct WaveChannelView {
    pub enabled: Enabled,
    /// NR30 DAC power bit.
    pub dac_enabled: bool,
    /// NR32 output-level byte.
    pub level: u8,
    /// Output volume as a fraction of full scale (the audio pane's readout).
    pub volume: f32,
    pub period: u16,
    pub length_enabled: bool,
    pub length_counter: u16,
    /// Wave-RAM sample position (0-31).
    pub wave_position: u8,
}

#[derive(Clone, Copy)]
pub struct NoiseChannelView {
    pub enabled: Enabled,
    /// NR42 volume-and-envelope byte.
    pub volume_and_envelope: VolumeAndEnvelope,
    /// NR43 clock-shift, LFSR width and divisor byte.
    pub frequency: u8,
    /// NR44 length-enable bit.
    pub length_enabled: bool,
    pub length_counter: u16,
    pub envelope_volume: u8,
    /// Envelope period counter — steps the volume when it reaches the pace.
    pub envelope_timer: u8,
    /// The noise LFSR (15-bit shift register).
    pub lfsr: u16,
}

impl AudioView {
    pub fn capture<A: ApuSpec>(audio: &Audio<A>) -> Self {
        let channels = audio.channels();
        let ch1 = &channels.ch1;
        let ch2 = &channels.ch2;
        let ch3 = &channels.ch3;
        let ch4 = &channels.ch4;
        Self {
            enabled: audio.enabled(),
            volume_left: audio.volume_left().0,
            volume_right: audio.volume_right().0,
            nr50: audio.nr50(),
            frame_sequencer_step: audio.frame_sequencer_step(),
            prev_div_apu_bit: audio.prev_div_apu_bit(),
            ch1: PulseChannelView {
                enabled: ch1.enabled,
                sweep: Some(ch1.sweep.0),
                duty_and_length: ch1.waveform_and_initial_length.0,
                volume_and_envelope: ch1.volume_and_envelope,
                period: ch1.period.0 & 0x7FF,
                length_enabled: ch1.length.enabled,
                length_counter: ch1.length.counter,
                envelope_volume: ch1.envelope.volume,
                envelope_timer: ch1.envelope.timer,
                shadow_frequency: Some(ch1.shadow_frequency),
                sweep_timer: Some(ch1.sweep_timer),
                sweep_enabled: Some(ch1.sweep_enabled),
                sweep_negate_used: Some(ch1.sweep_negate_used),
            },
            ch2: PulseChannelView {
                enabled: ch2.enabled,
                sweep: None,
                duty_and_length: ch2.waveform_and_initial_length.0,
                volume_and_envelope: ch2.volume_and_envelope,
                period: ch2.period.0 & 0x7FF,
                length_enabled: ch2.length.enabled,
                length_counter: ch2.length.counter,
                envelope_volume: ch2.envelope.volume,
                envelope_timer: ch2.envelope.timer,
                shadow_frequency: None,
                sweep_timer: None,
                sweep_enabled: None,
                sweep_negate_used: None,
            },
            ch3: WaveChannelView {
                enabled: ch3.enabled,
                dac_enabled: ch3.dac_enabled,
                level: ch3.volume.0,
                volume: ch3.volume.volume(),
                period: ch3.period.0 & 0x7FF,
                length_enabled: ch3.length.enabled,
                length_counter: ch3.length.counter,
                wave_position: ch3.wave_position,
            },
            ch4: NoiseChannelView {
                enabled: ch4.enabled,
                volume_and_envelope: ch4.volume_and_envelope,
                frequency: ch4.frequency_and_randomness.0,
                length_enabled: ch4.length.enabled,
                length_counter: ch4.length.counter,
                envelope_volume: ch4.envelope.volume,
                envelope_timer: ch4.envelope.timer,
                lfsr: ch4.lfsr,
            },
        }
    }
}

/// The NR14/NR24/NR34-style high byte reconstructed from the tracked period and
/// length-enable bit; the trigger bit is write-only and never held.
fn period_high_byte(period: u16, length_enabled: bool) -> u8 {
    (((period >> 8) & 0x07) as u8) | if length_enabled { 0x40 } else { 0x00 }
}

fn pulse_channel_block(
    label: &'static str,
    on_help: &'static str,
    ch: &PulseChannelView,
    nr1: &'static str,
    nr2: &'static str,
    nr3: &'static str,
    nr4: &'static str,
) -> inspect::SectionBlock {
    let mut rows = vec![inspect::Row::flag(label, ch.enabled.enabled).help(on_help)];
    if let Some(sweep) = ch.sweep {
        rows.push(
            inspect::Row::value("nr10", format!("{sweep:02X}"))
                .help("sweep pace / direction / step (NR10)"),
        );
    }
    rows.extend([
        inspect::Row::value(nr1, format!("{:02X}", ch.duty_and_length))
            .help("wave duty & initial length"),
        inspect::Row::value(nr2, format!("{:02X}", ch.volume_and_envelope.0))
            .help("initial volume & envelope"),
        inspect::Row::value(nr3, format!("{:02X}", ch.period & 0xFF)).help("period low byte"),
        inspect::Row::value(
            nr4,
            format!("{:02X}", period_high_byte(ch.period, ch.length_enabled)),
        )
        .help("period high & length-enable (trigger is write-only)"),
        inspect::Row::value("vol", ch.envelope_volume.to_string())
            .help("current envelope volume (0-15)"),
        inspect::Row::value("env timer", ch.envelope_timer.to_string())
            .help("envelope period counter — steps volume at the pace"),
        inspect::Row::value("len", ch.length_counter.to_string()).help("length counter (0-64)"),
    ]);
    if let (Some(shadow), Some(timer), Some(enabled), Some(negate)) = (
        ch.shadow_frequency,
        ch.sweep_timer,
        ch.sweep_enabled,
        ch.sweep_negate_used,
    ) {
        rows.extend([
            inspect::Row::value("shadow", format!("{shadow:03X}"))
                .help("sweep shadow frequency (11-bit)"),
            inspect::Row::value("swp timer", timer.to_string())
                .help("sweep period counter — recalculates at the pace"),
            inspect::Row::flag("swp on", enabled).help("sweep unit enabled"),
            inspect::Row::flag("negate", negate)
                .help("a negate-direction sweep calculation has run"),
        ]);
    }
    inspect::SectionBlock::Rows(rows)
}

fn wave_channel_block(ch: &WaveChannelView) -> inspect::SectionBlock {
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("ch3", ch.enabled.enabled).help("channel 3 on (NR52 bit 2)"),
        inspect::Row::value("nr30", if ch.dac_enabled { "80" } else { "00" })
            .help("DAC power (NR30 bit 7)"),
        inspect::Row::value("nr32", format!("{:02X}", ch.level)).help("output level (NR32)"),
        inspect::Row::value("nr33", format!("{:02X}", ch.period & 0xFF)).help("period low byte"),
        inspect::Row::value(
            "nr34",
            format!("{:02X}", period_high_byte(ch.period, ch.length_enabled)),
        )
        .help("period high & length-enable (trigger is write-only)"),
        inspect::Row::value("len", ch.length_counter.to_string()).help("length counter (0-256)"),
        inspect::Row::value("pos", ch.wave_position.to_string())
            .help("wave-RAM sample position (0-31)"),
    ])
}

fn noise_channel_block(ch: &NoiseChannelView) -> inspect::SectionBlock {
    inspect::SectionBlock::Rows(vec![
        inspect::Row::flag("ch4", ch.enabled.enabled).help("channel 4 on (NR52 bit 3)"),
        inspect::Row::value("nr42", format!("{:02X}", ch.volume_and_envelope.0))
            .help("initial volume & envelope"),
        inspect::Row::value("nr43", format!("{:02X}", ch.frequency))
            .help("clock shift, LFSR width & divisor (NR43)"),
        inspect::Row::value("vol", ch.envelope_volume.to_string())
            .help("current envelope volume (0-15)"),
        inspect::Row::value("env timer", ch.envelope_timer.to_string())
            .help("envelope period counter — steps volume at the pace"),
        inspect::Row::value("lfsr", format!("{:04X}", ch.lfsr))
            .help("noise shift register (15-bit LFSR)"),
        inspect::Row::value("len", ch.length_counter.to_string()).help("length counter (0-64)"),
    ])
}

/// NR51 sound-panning byte reconstructed from each channel's per-side output
/// enables: high nibble left (ch4..ch1), low nibble right (ch4..ch1).
fn panning_byte(audio: &AudioView) -> u8 {
    let sides = [
        audio.ch1.enabled,
        audio.ch2.enabled,
        audio.ch3.enabled,
        audio.ch4.enabled,
    ];
    let mut nr51 = 0u8;
    for (channel, enabled) in sides.iter().enumerate() {
        if enabled.output_right {
            nr51 |= 1 << channel;
        }
        if enabled.output_left {
            nr51 |= 1 << (channel + 4);
        }
    }
    nr51
}

/// The APU section, shared by DMG and CGB (the sound block is the same silicon):
/// the four channels' NRxx register bytes with the runtime summaries the core
/// tracks, plus the master NR50/NR51 registers. The header pip is the NR52 power
/// bit; the summary lists the powered-on channels.
pub fn apu_section(audio: &AudioView) -> inspect::Section {
    use inspect::SectionBlock::{Rows, Rule};

    let on: Vec<&str> = [
        (audio.ch1.enabled.enabled, "ch1"),
        (audio.ch2.enabled.enabled, "ch2"),
        (audio.ch3.enabled.enabled, "ch3"),
        (audio.ch4.enabled.enabled, "ch4"),
    ]
    .into_iter()
    .filter_map(|(on, name)| on.then_some(name))
    .collect();
    let summary = if !audio.enabled {
        "off".to_string()
    } else if on.is_empty() {
        "on".to_string()
    } else {
        on.join(" ")
    };

    inspect::Section {
        name: "APU",
        summary,
        active: Some(audio.enabled),
        detail: None,
        blocks: vec![
            Rows(vec![
                inspect::Row::value("nr50", format!("{:02X}", audio.nr50))
                    .help("master volume L/R & VIN (NR50)"),
                inspect::Row::value("nr51", format!("{:02X}", panning_byte(audio)))
                    .help("sound panning — per-channel L/R (NR51)"),
                inspect::Row::value("fs step", audio.frame_sequencer_step.to_string())
                    .help("frame-sequencer step (0-7) — DIV-APU divider phase"),
                inspect::Row::flag("div bit", audio.prev_div_apu_bit)
                    .help("DIV bit last sampled — its fall clocks the sequencer"),
            ]),
            Rule,
            pulse_channel_block(
                "ch1",
                "channel 1 on (NR52 bit 0)",
                &audio.ch1,
                "nr11",
                "nr12",
                "nr13",
                "nr14",
            ),
            Rule,
            pulse_channel_block(
                "ch2",
                "channel 2 on (NR52 bit 1)",
                &audio.ch2,
                "nr21",
                "nr22",
                "nr23",
                "nr24",
            ),
            Rule,
            wave_channel_block(&audio.ch3),
            Rule,
            noise_channel_block(&audio.ch4),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Console;
    use crate::cartridge::Cartridge;
    use crate::debugger::inspection::tests::row_labels;

    fn ran_console(capture: bool) -> Console<crate::Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut console =
            Console::<crate::Dmg>::new(Cartridge::new(rom, None, None).unwrap(), None);
        console.set_wave_capture(capture);
        // One frame's worth of steps fills the capture window.
        for _ in 0..20_000 {
            console.step();
        }
        console
    }

    #[test]
    fn capture_windows_fill_when_enabled() {
        let console = ran_console(true);
        let waves = console.channel_waves().expect("capture enabled");
        assert_eq!(waves.len(), 4);
        for wave in &waves {
            assert_eq!(wave.rate, 44100);
            assert!(!wave.levels.is_empty());
        }
    }

    #[test]
    fn no_capture_windows_when_disabled() {
        assert!(ran_console(false).channel_waves().is_none());
    }

    #[test]
    fn apu_section_reports_power_and_channel_registers() {
        let audio = AudioView::capture(&Audio::<crate::audio::DmgApu>::post_boot(0));
        let section = apu_section(&audio);
        assert_eq!(section.name, "APU");
        // Post-boot: APU powered and CH1 running.
        assert_eq!(section.active, Some(true));
        assert!(section.summary.contains("ch1"));

        // CH1's NR12 register byte reads back the post-boot envelope value 0xF3.
        let nr12 = section
            .blocks
            .iter()
            .find_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => rows
                    .iter()
                    .find(|row| row.label == "nr12")
                    .map(|row| row.value.clone()),
                _ => None,
            })
            .expect("a CH1 NR12 row");
        assert_eq!(nr12, "F3");

        // The CH1 pip tracks the channel-on state.
        let ch1_on = section.blocks.iter().any(|block| match block {
            inspect::SectionBlock::Rows(rows) => rows
                .iter()
                .any(|row| row.label == "ch1" && row.active == Some(true)),
            _ => false,
        });
        assert!(ch1_on);
    }

    #[test]
    fn apu_section_carries_runtime_rows() {
        let audio = AudioView::capture(&Audio::<crate::audio::DmgApu>::post_boot(0));
        let section = apu_section(&audio);
        let labels = row_labels(&section);
        // Master block: frame-sequencer step and prev-DIV-bit pip.
        for expected in ["fs step", "div bit"] {
            assert!(labels.iter().any(|l| l == expected), "missing {expected}");
        }
        // CH1 carries the envelope timer plus its sweep runtime.
        for expected in ["env timer", "shadow", "swp timer", "swp on", "negate"] {
            assert!(labels.iter().any(|l| l == expected), "missing {expected}");
        }
        // CH4 carries the LFSR; CH2 (no sweep) carries no sweep rows.
        assert!(labels.iter().any(|l| l == "lfsr"));
    }
}
