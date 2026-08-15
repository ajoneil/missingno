//! The Texas Instruments SN76489 family of programmable tone/noise
//! generators: three tone generators, a noise generator, a four-stage
//! attenuator on each, and an operational-amplifier summing stage, all behind
//! one write-only port with a latched register address.
//!
//! The discrete TI part and the one Sega integrated into its VDPs differ in
//! shift-register geometry, in what a zero period register means, in power-on
//! state and in whether a READY pin exists. Every one of those sits behind
//! [`Variant`] at the single point it manifests.

/// Which member of the family the model is being asked to be.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variant {
    /// The discrete SN76489AN: SG-1000, SC-3000, OMV, BBC Micro, ColecoVision.
    DiscreteTi,
    /// The PSG Sega integrated into its VDPs, from the Master System onwards.
    SegaIntegrated,
}

/// The prescaler between the CLOCK pin and the channel counters.
const INPUT_CLOCKS_PER_INTERNAL_CLOCK: u8 = 16;
/// Input clocks a byte takes to load, holding READY low meanwhile.
const READY_LOW_CLOCKS: u8 = 32;
/// The attenuation that switches a channel off.
const MUTE: u8 = 0x0F;
/// Attenuator weights: 16, 8, 4 and 2 dB, so one step of the register is 2 dB.
const DECIBELS_PER_STEP: f32 = 2.0;
/// One past the largest period register value.
const FULL_TONE_SPAN: u16 = 0x400;

const TONE_CHANNELS: usize = 3;
const CHANNELS: usize = 4;
/// The register file's fourth channel.
const NOISE_CHANNEL: usize = 3;
/// The tone generator the noise generator can borrow its rate from.
const THIRD_TONE: usize = TONE_CHANNELS - 1;

impl Variant {
    /// The count a channel's period register stands for; `None` never
    /// borrows, holding the flip-flop. Zero is the one value the two parts
    /// read differently.
    fn effective_period(self, register: u16) -> Option<u16> {
        match (register, self) {
            (0, Variant::DiscreteTi) => Some(FULL_TONE_SPAN),
            (0, Variant::SegaIntegrated) => None,
            (period, _) => Some(period),
        }
    }

    fn lfsr_width(self) -> u32 {
        match self {
            Variant::DiscreteTi => 15,
            Variant::SegaIntegrated => 16,
        }
    }

    /// The shift register's top bit: where feedback enters, and the only bit
    /// a noise-control write leaves set.
    fn lfsr_shift_in(self) -> u16 {
        1 << (self.lfsr_width() - 1)
    }

    /// The bits the white-noise feedback network exclusive-ORs together.
    fn white_noise_taps(self) -> u16 {
        match self {
            Variant::DiscreteTi => 0b0011,
            Variant::SegaIntegrated => 0b1001,
        }
    }

    /// The discrete part carries an open-collector READY; the integrated die
    /// has no such pin.
    fn has_ready(self) -> bool {
        self == Variant::DiscreteTi
    }
}

#[derive(Clone, Copy)]
enum RegisterKind {
    Frequency,
    Attenuation,
}

/// The register address held on the chip between transfers.
#[derive(Clone, Copy)]
struct LatchedRegister {
    channel: usize,
    kind: RegisterKind,
}

/// A 10-bit period register, the counter it reloads, and the frequency
/// flip-flop the counter's borrow toggles.
struct ToneChannel {
    period: u16,
    counter: u16,
    output: bool,
}

impl ToneChannel {
    fn new() -> Self {
        ToneChannel {
            period: 0,
            counter: 0,
            output: true,
        }
    }

    /// One internal clock. The borrow reloads the counter and toggles the
    /// flip-flop, so the half-period is the reloaded count.
    fn advance(&mut self, period: Option<u16>) {
        let Some(period) = period else { return };
        if self.counter > 1 {
            self.counter -= 1;
        } else {
            self.counter = period;
            self.output = !self.output;
        }
    }
}

/// The shift rate selector, in divisions of the input clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NoiseRate {
    Div512,
    Div1024,
    Div2048,
    FollowTone3,
}

impl NoiseRate {
    fn from_control(control: u8) -> NoiseRate {
        match control & 0x03 {
            0 => NoiseRate::Div512,
            1 => NoiseRate::Div1024,
            2 => NoiseRate::Div2048,
            _ => NoiseRate::FollowTone3,
        }
    }

    fn bits(self) -> u8 {
        match self {
            NoiseRate::Div512 => 0,
            NoiseRate::Div1024 => 1,
            NoiseRate::Div2048 => 2,
            NoiseRate::FollowTone3 => 3,
        }
    }

    /// Internal clocks between borrows. The named division is the shift
    /// register's rate, and the flip-flop takes two borrows per shift.
    fn fixed_reload(self) -> Option<u16> {
        let input_clocks: u16 = match self {
            NoiseRate::Div512 => 512,
            NoiseRate::Div1024 => 1024,
            NoiseRate::Div2048 => 2048,
            NoiseRate::FollowTone3 => return None,
        };
        Some(input_clocks / (2 * u16::from(INPUT_CLOCKS_PER_INTERNAL_CLOCK)))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NoiseMode {
    Periodic,
    White,
}

/// The noise generator: a counter and flip-flop like a tone channel's, whose
/// rising edge clocks a shift register with an exclusive-OR feedback network.
struct NoiseChannel {
    rate: NoiseRate,
    mode: NoiseMode,
    counter: u16,
    output: bool,
    lfsr: u16,
}

impl NoiseChannel {
    fn new(variant: Variant) -> Self {
        NoiseChannel {
            rate: NoiseRate::Div512,
            mode: NoiseMode::Periodic,
            counter: 0,
            output: true,
            lfsr: variant.lfsr_shift_in(),
        }
    }

    /// One internal clock, reporting the flip-flop's rising edge.
    fn advance(&mut self, period: Option<u16>) -> bool {
        let Some(period) = period else { return false };
        if self.counter > 1 {
            self.counter -= 1;
            return false;
        }
        self.counter = period;
        self.output = !self.output;
        self.output
    }

    /// White noise inserts the exclusive-OR of the tapped bits; "periodic"
    /// noise taps bit 0 alone, so the contents recirculate.
    fn shift(&mut self, variant: Variant) {
        let feedback = match self.mode {
            NoiseMode::White => (self.lfsr & variant.white_noise_taps()).count_ones() & 1 != 0,
            NoiseMode::Periodic => self.lfsr & 1 != 0,
        };
        self.lfsr >>= 1;
        if feedback {
            self.lfsr |= variant.lfsr_shift_in();
        }
    }

    /// The bit leaving the register, which is what reaches the mixer.
    fn output_bit(&self) -> bool {
        self.lfsr & 1 != 0
    }
}

pub struct Psg {
    variant: Variant,
    latched: LatchedRegister,
    tones: [ToneChannel; TONE_CHANNELS],
    noise: NoiseChannel,
    /// 4-bit attenuations, one per channel.
    volumes: [u8; CHANNELS],
    divider: u8,
    busy_clocks: u8,
}

impl Psg {
    pub fn new(variant: Variant) -> Self {
        let (volume, latched) = match variant {
            Variant::DiscreteTi => (
                0,
                LatchedRegister {
                    channel: 0,
                    kind: RegisterKind::Frequency,
                },
            ),
            Variant::SegaIntegrated => (
                MUTE,
                LatchedRegister {
                    channel: 1,
                    kind: RegisterKind::Attenuation,
                },
            ),
        };
        Psg {
            variant,
            latched,
            tones: [ToneChannel::new(), ToneChannel::new(), ToneChannel::new()],
            noise: NoiseChannel::new(variant),
            volumes: [volume; CHANNELS],
            divider: 0,
            busy_clocks: 0,
        }
    }

    /// A byte on the data pins. Bit 7 addresses a register and carries its low
    /// bits; otherwise the byte goes to the register still latched.
    pub fn write(&mut self, byte: u8) {
        if byte & 0x80 != 0 {
            self.latched = LatchedRegister {
                channel: ((byte >> 5) & 0x03) as usize,
                kind: match byte & 0x10 {
                    0 => RegisterKind::Frequency,
                    _ => RegisterKind::Attenuation,
                },
            };
            self.write_latch_data(byte & 0x0F);
        } else {
            self.write_data(byte & 0x3F);
        }
        if self.variant.has_ready() {
            self.busy_clocks = READY_LOW_CLOCKS;
        }
    }

    /// The addressing byte's low nibble: the latched register's low bits.
    fn write_latch_data(&mut self, nibble: u8) {
        match (self.latched.kind, self.latched.channel) {
            (RegisterKind::Attenuation, channel) => self.volumes[channel] = nibble,
            (RegisterKind::Frequency, NOISE_CHANNEL) => self.write_noise_control(nibble),
            (RegisterKind::Frequency, channel) => {
                let period = &mut self.tones[channel].period;
                *period = (*period & 0x3F0) | u16::from(nibble);
            }
        }
    }

    /// A data byte's six bits: a tone register's high bits, or the whole of a
    /// register too narrow to need two transfers.
    fn write_data(&mut self, bits: u8) {
        match (self.latched.kind, self.latched.channel) {
            (RegisterKind::Attenuation, channel) => self.volumes[channel] = bits & 0x0F,
            (RegisterKind::Frequency, NOISE_CHANNEL) => self.write_noise_control(bits),
            (RegisterKind::Frequency, channel) => {
                let period = &mut self.tones[channel].period;
                *period = (*period & 0x00F) | (u16::from(bits) << 4);
            }
        }
    }

    /// The 3-bit noise register. Changing it clears the shift register.
    fn write_noise_control(&mut self, control: u8) {
        self.noise.rate = NoiseRate::from_control(control);
        self.noise.mode = match control & 0x04 {
            0 => NoiseMode::Periodic,
            _ => NoiseMode::White,
        };
        self.noise.lfsr = self.variant.lfsr_shift_in();
    }

    /// One cycle of the CLOCK pin.
    pub fn tick(&mut self) {
        self.busy_clocks = self.busy_clocks.saturating_sub(1);
        self.divider += 1;
        if self.divider < INPUT_CLOCKS_PER_INTERNAL_CLOCK {
            return;
        }
        self.divider = 0;
        self.internal_clock();
    }

    fn internal_clock(&mut self) {
        for channel in 0..TONE_CHANNELS {
            let period = self.variant.effective_period(self.tones[channel].period);
            self.tones[channel].advance(period);
        }
        let period = self.noise_period();
        if self.noise.advance(period) {
            self.noise.shift(self.variant);
        }
    }

    /// The noise counter's reload, read live so a rewritten tone 3 tracks.
    fn noise_period(&self) -> Option<u16> {
        match self.noise.rate.fixed_reload() {
            Some(reload) => Some(reload),
            None => self.variant.effective_period(self.tones[THIRD_TONE].period),
        }
    }

    /// The READY pin: low while a byte loads. The integrated part, having no
    /// such pin, is never busy.
    pub fn ready(&self) -> bool {
        self.busy_clocks == 0
    }

    /// The output buffer sums the three tone generators and the noise
    /// generator, here normalised to all four conducting unattenuated.
    pub fn level(&self) -> f32 {
        let mut sum = 0.0;
        for channel in 0..TONE_CHANNELS {
            if self.tones[channel].output {
                sum += amplitude(self.volumes[channel]);
            }
        }
        if self.noise.output_bit() {
            sum += amplitude(self.volumes[NOISE_CHANNEL]);
        }
        sum / CHANNELS as f32
    }

    /// The three 10-bit period registers.
    pub fn tone_periods(&self) -> [u16; TONE_CHANNELS] {
        [
            self.tones[0].period,
            self.tones[1].period,
            self.tones[2].period,
        ]
    }

    /// The four 4-bit attenuation registers.
    pub fn attenuations(&self) -> [u8; CHANNELS] {
        self.volumes
    }

    /// The 3-bit noise register: mode in bit 2, shift rate in bits 1-0.
    pub fn noise_control(&self) -> u8 {
        let mode = match self.noise.mode {
            NoiseMode::Periodic => 0,
            NoiseMode::White => 0x04,
        };
        mode | self.noise.rate.bits()
    }
}

fn amplitude(attenuation: u8) -> f32 {
    if attenuation >= MUTE {
        0.0
    } else {
        10.0f32.powf(-DECIBELS_PER_STEP * f32::from(attenuation) / 20.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discrete() -> Psg {
        Psg::new(Variant::DiscreteTi)
    }

    fn sega() -> Psg {
        Psg::new(Variant::SegaIntegrated)
    }

    fn tone0(psg: &Psg) -> bool {
        psg.tones[0].output
    }

    fn noise_flip_flop(psg: &Psg) -> bool {
        psg.noise.output
    }

    fn shift_register(psg: &Psg) -> u16 {
        psg.noise.lfsr
    }

    fn tick(psg: &mut Psg, clocks: u32) {
        for _ in 0..clocks {
            psg.tick();
        }
    }

    /// Input clocks until `probe` first differs from its value now.
    fn clocks_until_change<T: Copy + PartialEq + std::fmt::Debug>(
        psg: &mut Psg,
        probe: fn(&Psg) -> T,
        limit: u32,
    ) -> u32 {
        let before = probe(psg);
        for clock in 1..=limit {
            psg.tick();
            if probe(psg) != before {
                return clock;
            }
        }
        panic!("{before:?} unchanged after {limit} input clocks");
    }

    /// The bits leaving the shift register, one per shift.
    fn shift_sequence(variant: Variant, mode: NoiseMode, shifts: usize) -> Vec<u8> {
        let mut noise = NoiseChannel::new(variant);
        noise.mode = mode;
        (0..shifts)
            .map(|_| {
                let out = (noise.lfsr & 1) as u8;
                noise.shift(variant);
                out
            })
            .collect()
    }

    /// Shifts before the register returns to its cleared state — which, the
    /// state deciding the whole future, is the output sequence's period.
    fn shift_cycle_length(variant: Variant, mode: NoiseMode) -> usize {
        let mut noise = NoiseChannel::new(variant);
        noise.mode = mode;
        let cleared = noise.lfsr;
        for shift in 1..=1 << 17 {
            noise.shift(variant);
            if noise.lfsr == cleared {
                return shift;
            }
        }
        panic!("the shift register never returned to its cleared state");
    }

    #[test]
    fn an_addressing_byte_lands_its_nibble_immediately() {
        let mut psg = discrete();
        psg.write(0x8F);
        assert_eq!(psg.tone_periods()[0], 0x00F);
    }

    #[test]
    fn a_data_byte_fills_a_tone_registers_high_six_bits() {
        let mut psg = discrete();
        psg.write(0x8E);
        psg.write(0x0F);
        assert_eq!(psg.tone_periods()[0], 0x0FE);
    }

    #[test]
    fn consecutive_bytes_walk_the_tone_register() {
        // Each byte lands as it arrives; neither waits for the other.
        let mut psg = discrete();
        psg.write(0x80);
        assert_eq!(psg.tone_periods()[0] & 0x00F, 0x000);
        psg.write(0x00);
        assert_eq!(psg.tone_periods()[0], 0x000);
        psg.write(0x8F);
        assert_eq!(psg.tone_periods()[0], 0x00F);
        psg.write(0x3F);
        assert_eq!(psg.tone_periods()[0], 0x3FF);
    }

    #[test]
    fn a_data_byte_after_an_attenuation_address_updates_the_volume() {
        let mut psg = discrete();
        psg.write(0xDF);
        psg.write(0x00);
        assert_eq!(psg.attenuations()[2], 0x00);
    }

    #[test]
    fn a_data_byte_after_a_noise_address_updates_the_control() {
        let mut psg = discrete();
        psg.write(0xE5);
        assert_eq!(psg.noise_control(), 0x05);
        psg.write(0x04);
        assert_eq!(psg.noise_control(), 0x04);
    }

    #[test]
    fn the_noise_register_discards_the_nibbles_high_bit() {
        let mut psg = discrete();
        psg.write(0xEF);
        assert_eq!(psg.noise_control(), 0x07);
    }

    #[test]
    fn the_latched_register_survives_data_bytes() {
        let mut psg = discrete();
        psg.write(0x8E);
        psg.write(0x0F);
        psg.write(0x00);
        assert_eq!(psg.tone_periods()[0], 0x00E);
    }

    #[test]
    fn a_tone_toggles_every_sixteen_input_clocks_per_period_step() {
        let mut psg = discrete();
        psg.write(0x83);
        psg.write(0x00);
        clocks_until_change(&mut psg, tone0, 64);
        for _ in 0..4 {
            assert_eq!(clocks_until_change(&mut psg, tone0, 256), 16 * 3);
        }
    }

    #[test]
    fn a_period_rewrite_applies_at_the_next_borrow() {
        let mut psg = discrete();
        psg.write(0x88);
        psg.write(0x00);
        clocks_until_change(&mut psg, tone0, 64);
        tick(&mut psg, 16 * 3);
        psg.write(0x82);
        psg.write(0x00);
        // The counter keeps the five internal clocks it still owes.
        assert_eq!(clocks_until_change(&mut psg, tone0, 256), 16 * 5);
        assert_eq!(clocks_until_change(&mut psg, tone0, 256), 16 * 2);
    }

    #[test]
    fn a_zero_period_counts_the_full_span_on_the_discrete_part() {
        let mut psg = discrete();
        clocks_until_change(&mut psg, tone0, 64);
        assert_eq!(
            clocks_until_change(&mut psg, tone0, 32_768),
            16 * FULL_TONE_SPAN as u32
        );
    }

    #[test]
    fn a_zero_period_holds_the_channel_on_the_integrated_part() {
        let mut psg = sega();
        let held = tone0(&psg);
        tick(&mut psg, 2 * 16 * FULL_TONE_SPAN as u32);
        assert_eq!(tone0(&psg), held);
    }

    #[test]
    fn the_fixed_shift_rates_divide_the_input_clock() {
        for (control, input_clocks) in [(0xE0u8, 512u32), (0xE1, 1024), (0xE2, 2048)] {
            let mut psg = discrete();
            psg.write(control);
            clocks_until_change(&mut psg, shift_register, 4096);
            assert_eq!(
                clocks_until_change(&mut psg, shift_register, 8192),
                input_clocks
            );
        }
    }

    #[test]
    fn the_follow_rate_tracks_the_third_tone_register() {
        let mut psg = discrete();
        psg.write(0xC4);
        psg.write(0x00);
        psg.write(0xE3);
        clocks_until_change(&mut psg, shift_register, 1024);
        assert_eq!(
            clocks_until_change(&mut psg, shift_register, 1024),
            2 * 16 * 4
        );
        psg.write(0xC8);
        psg.write(0x00);
        clocks_until_change(&mut psg, shift_register, 2048);
        assert_eq!(
            clocks_until_change(&mut psg, shift_register, 2048),
            2 * 16 * 8
        );
    }

    #[test]
    fn the_shift_register_advances_on_rising_edges_only() {
        let mut psg = discrete();
        psg.write(0xE0);
        let (mut toggles, mut shifts) = (0, 0);
        let mut output = noise_flip_flop(&psg);
        let mut register = shift_register(&psg);
        for _ in 0..16 * 16 * 21 {
            psg.tick();
            let toggled = noise_flip_flop(&psg) != output;
            if shift_register(&psg) != register {
                assert!(toggled && noise_flip_flop(&psg));
                register = shift_register(&psg);
                shifts += 1;
            }
            if toggled {
                output = !output;
                toggles += 1;
            }
        }
        assert_eq!(toggles, 21);
        assert_eq!(shifts, 10);
    }

    #[test]
    fn the_discrete_white_sequence_runs_32767_shifts() {
        assert_eq!(
            shift_cycle_length(Variant::DiscreteTi, NoiseMode::White),
            32767
        );
        // Taps at bits 0 and 1 of a 15-bit register.
        let sequence = shift_sequence(Variant::DiscreteTi, NoiseMode::White, 200);
        for i in 15..sequence.len() {
            assert_eq!(sequence[i], sequence[i - 15] ^ sequence[i - 14]);
        }
    }

    #[test]
    fn the_integrated_white_sequence_runs_57337_shifts() {
        assert_eq!(
            shift_cycle_length(Variant::SegaIntegrated, NoiseMode::White),
            57337
        );
        // Taps at bits 0 and 3 of a 16-bit register.
        let sequence = shift_sequence(Variant::SegaIntegrated, NoiseMode::White, 200);
        for i in 16..sequence.len() {
            assert_eq!(sequence[i], sequence[i - 16] ^ sequence[i - 13]);
        }
    }

    #[test]
    fn periodic_noise_is_a_one_in_fifteen_duty_on_the_discrete_part() {
        assert_eq!(
            shift_cycle_length(Variant::DiscreteTi, NoiseMode::Periodic),
            15
        );
        let sequence = shift_sequence(Variant::DiscreteTi, NoiseMode::Periodic, 150);
        assert_eq!(sequence.iter().filter(|&&bit| bit == 1).count(), 10);
    }

    #[test]
    fn periodic_noise_is_a_one_in_sixteen_duty_on_the_integrated_part() {
        assert_eq!(
            shift_cycle_length(Variant::SegaIntegrated, NoiseMode::Periodic),
            16
        );
        let sequence = shift_sequence(Variant::SegaIntegrated, NoiseMode::Periodic, 160);
        assert_eq!(sequence.iter().filter(|&&bit| bit == 1).count(), 10);
    }

    #[test]
    fn a_noise_control_write_clears_the_shift_register() {
        let cleared = Variant::DiscreteTi.lfsr_shift_in();
        let mut psg = discrete();
        psg.write(0xE4);
        tick(&mut psg, 16 * 16 * 40);
        assert_ne!(shift_register(&psg), cleared);
        psg.write(0xE4);
        assert_eq!(shift_register(&psg), cleared);
    }

    #[test]
    fn each_attenuation_step_is_two_decibels() {
        for step in 1..MUTE {
            let ratio = amplitude(step) / amplitude(step - 1);
            assert!((20.0 * ratio.log10() + DECIBELS_PER_STEP).abs() < 1e-4);
        }
        assert_eq!(amplitude(MUTE), 0.0);
    }

    #[test]
    fn the_summing_stage_is_linear() {
        let mut psg = discrete();
        for channel in 0..CHANNELS as u8 {
            psg.write(0x9F | (channel << 5));
        }
        assert_eq!(psg.level(), 0.0);
        psg.write(0x90);
        let one = psg.level();
        psg.write(0xB0);
        assert!((psg.level() - 2.0 * one).abs() < 1e-6);
    }

    #[test]
    fn ready_returns_thirty_two_input_clocks_after_a_write() {
        let mut psg = discrete();
        assert!(psg.ready());
        psg.write(0x9F);
        for _ in 0..READY_LOW_CLOCKS - 1 {
            psg.tick();
            assert!(!psg.ready());
        }
        psg.tick();
        assert!(psg.ready());
    }

    #[test]
    fn the_integrated_part_is_always_ready() {
        let mut psg = sega();
        psg.write(0x9F);
        assert!(psg.ready());
    }

    #[test]
    fn the_discrete_part_powers_on_sounding() {
        let psg = discrete();
        assert_eq!(psg.tone_periods(), [0; TONE_CHANNELS]);
        assert_eq!(psg.attenuations(), [0; CHANNELS]);
        assert!(psg.level() > 0.0);
    }

    #[test]
    fn the_discrete_part_powers_on_addressing_the_first_tone() {
        let mut psg = discrete();
        psg.write(0x3F);
        assert_eq!(psg.tone_periods()[0], 0x3F0);
    }

    #[test]
    fn the_integrated_part_powers_on_silent() {
        let psg = sega();
        assert_eq!(psg.attenuations(), [MUTE; CHANNELS]);
        assert_eq!(psg.level(), 0.0);
    }

    #[test]
    fn the_integrated_part_powers_on_addressing_the_second_attenuation() {
        let mut psg = sega();
        psg.write(0x00);
        assert_eq!(psg.attenuations()[1], 0x00);
    }
}
