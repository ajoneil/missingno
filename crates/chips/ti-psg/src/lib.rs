//! The Texas Instruments SN76489 family of programmable tone/noise
//! generators: three tone generators, a noise generator, a four-stage
//! attenuator on each, and an operational-amplifier summing stage, all behind
//! one write-only port with a latched register address.
//!
//! The discrete TI part and the one Sega integrated into its VDPs differ in
//! shift-register geometry, in what a zero period register means, in power-on
//! state and in whether a READY pin exists. Every one of those sits behind
//! [`Variant`] at the single point it manifests.

#[cfg(feature = "inspect")]
pub mod inspect;

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
pub const MUTE_ATTENUATION: u8 = 0x0F;
/// Attenuator weights: 16, 8, 4 and 2 dB, so one step of the register is 2 dB.
pub const DECIBELS_PER_STEP: f32 = 2.0;
/// One past the largest period register value.
pub const FULL_TONE_SPAN: u16 = 0x400;

pub const TONE_CHANNELS: usize = 3;
pub const CHANNELS: usize = 4;

/// A generator, which is also a register-file address: the output buffer sums
/// the three tone generators and the noise generator.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Tone1,
    Tone2,
    Tone3,
    Noise,
}

impl Channel {
    /// In register-address order, which is the order the mixer sums them.
    pub const ALL: [Channel; CHANNELS] = [
        Channel::Tone1,
        Channel::Tone2,
        Channel::Tone3,
        Channel::Noise,
    ];

    /// The two channel-select bits of an addressing byte.
    fn from_address(bits: u8) -> Channel {
        match bits & 0x03 {
            0 => Channel::Tone1,
            1 => Channel::Tone2,
            2 => Channel::Tone3,
            _ => Channel::Noise,
        }
    }

    /// Its place in the register file, and in [`Psg::dac_codes`].
    pub fn index(self) -> usize {
        self as usize
    }
}

impl Variant {
    /// The count a channel's period register stands for; `None` never
    /// borrows, holding the flip-flop. Zero is the one value the two parts
    /// read differently.
    pub fn effective_period(self, register: u16) -> Option<u16> {
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

/// Which of a channel's two registers an addressing byte selected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegisterKind {
    Frequency,
    Attenuation,
}

/// Which of the write port's two transfers carried a value: the addressing
/// byte's low nibble, or a following data byte's six bits.
#[derive(Clone, Copy)]
enum Transfer {
    Address,
    Data,
}

/// The register address held on the chip between transfers.
#[derive(Clone, Copy)]
struct LatchedRegister {
    channel: Channel,
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
}

/// One internal clock of a channel's divider. The borrow reloads the counter
/// and toggles the flip-flop, so the half-period is the reloaded count; the
/// return reports the flip-flop's rising edge.
fn advance_divider(counter: &mut u16, output: &mut bool, period: Option<u16>) -> bool {
    let Some(period) = period else { return false };
    if *counter > 1 {
        *counter -= 1;
        return false;
    }
    *counter = period;
    *output = !*output;
    *output
}

/// The shift rate selector, in divisions of the input clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoiseRate {
    Div512,
    Div1024,
    Div2048,
    FollowTone3,
}

impl NoiseRate {
    /// The noise register's low two bits.
    pub fn from_control(control: u8) -> NoiseRate {
        match control & 0x03 {
            0 => NoiseRate::Div512,
            1 => NoiseRate::Div1024,
            2 => NoiseRate::Div2048,
            _ => NoiseRate::FollowTone3,
        }
    }

    pub fn bits(self) -> u8 {
        match self {
            NoiseRate::Div512 => 0,
            NoiseRate::Div1024 => 1,
            NoiseRate::Div2048 => 2,
            NoiseRate::FollowTone3 => 3,
        }
    }

    /// The input clocks a shift takes, or `None` where the rate is the third
    /// tone generator's period instead.
    pub fn input_clock_division(self) -> Option<u16> {
        match self {
            NoiseRate::Div512 => Some(512),
            NoiseRate::Div1024 => Some(1024),
            NoiseRate::Div2048 => Some(2048),
            NoiseRate::FollowTone3 => None,
        }
    }

    /// Internal clocks between borrows. The named division is the shift
    /// register's rate, and the flip-flop takes two borrows per shift.
    fn fixed_reload(self) -> Option<u16> {
        let input_clocks = self.input_clock_division()?;
        Some(input_clocks / (2 * u16::from(INPUT_CLOCKS_PER_INTERNAL_CLOCK)))
    }
}

/// The feedback the shift register is clocked with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoiseMode {
    Periodic,
    White,
}

impl NoiseMode {
    /// The noise register's bit 2.
    pub fn from_control(control: u8) -> NoiseMode {
        match control & 0x04 {
            0 => NoiseMode::Periodic,
            _ => NoiseMode::White,
        }
    }

    pub fn bits(self) -> u8 {
        match self {
            NoiseMode::Periodic => 0,
            NoiseMode::White => 0x04,
        }
    }
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
                    channel: Channel::Tone1,
                    kind: RegisterKind::Frequency,
                },
            ),
            Variant::SegaIntegrated => (
                MUTE_ATTENUATION,
                LatchedRegister {
                    channel: Channel::Tone2,
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
                channel: Channel::from_address(byte >> 5),
                kind: match byte & 0x10 {
                    0 => RegisterKind::Frequency,
                    _ => RegisterKind::Attenuation,
                },
            };
            self.write_latched(byte & 0x0F, Transfer::Address);
        } else {
            self.write_latched(byte & 0x3F, Transfer::Data);
        }
        if self.variant.has_ready() {
            self.busy_clocks = READY_LOW_CLOCKS;
        }
    }

    /// A transfer into the latched register. Only a tone period is wide
    /// enough to take both halves; every other register takes the whole
    /// value from either transfer.
    fn write_latched(&mut self, value: u8, transfer: Transfer) {
        match (self.latched.kind, self.latched.channel) {
            (RegisterKind::Attenuation, channel) => self.volumes[channel.index()] = value & 0x0F,
            (RegisterKind::Frequency, Channel::Noise) => self.write_noise_control(value),
            (RegisterKind::Frequency, tone) => {
                let period = &mut self.tones[tone.index()].period;
                *period = match transfer {
                    Transfer::Address => (*period & 0x3F0) | u16::from(value),
                    Transfer::Data => (*period & 0x00F) | (u16::from(value) << 4),
                };
            }
        }
    }

    /// The 3-bit noise register. Changing it clears the shift register.
    fn write_noise_control(&mut self, control: u8) {
        self.noise.rate = NoiseRate::from_control(control);
        self.noise.mode = NoiseMode::from_control(control);
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
            let tone = &mut self.tones[channel];
            advance_divider(&mut tone.counter, &mut tone.output, period);
        }
        let period = self.noise_period();
        if advance_divider(&mut self.noise.counter, &mut self.noise.output, period) {
            self.noise.shift(self.variant);
        }
    }

    /// The noise counter's reload, read live so a rewritten tone 3 tracks.
    fn noise_period(&self) -> Option<u16> {
        match self.noise.rate.fixed_reload() {
            Some(reload) => Some(reload),
            None => self
                .variant
                .effective_period(self.tones[Channel::Tone3.index()].period),
        }
    }

    /// The READY pin: low while a byte loads. The integrated part, having no
    /// such pin, is never busy.
    pub fn ready(&self) -> bool {
        self.busy_clocks == 0
    }

    /// Whether a channel's generator is driving its attenuator: a tone's
    /// frequency flip-flop high, or the bit leaving the shift register set.
    fn conducting(&self, channel: Channel) -> bool {
        match channel {
            Channel::Noise => self.noise.output_bit(),
            tone => self.tones[tone.index()].output,
        }
    }

    /// The output buffer sums the three tone generators and the noise
    /// generator, here normalised to all four conducting unattenuated.
    pub fn level(&self) -> f32 {
        let mut sum = 0.0;
        for channel in Channel::ALL {
            if self.conducting(channel) {
                sum += amplitude(self.volumes[channel.index()]);
            }
        }
        sum / CHANNELS as f32
    }

    /// Per channel, the amplitude code it hands its DAC: the attenuation
    /// complemented while the channel conducts, zero otherwise. The 2 dB per
    /// step the ladder turns each code into is the DAC's, not the code's.
    pub fn dac_codes(&self) -> [u8; CHANNELS] {
        Channel::ALL.map(|channel| {
            if self.conducting(channel) {
                MUTE_ATTENUATION - self.volumes[channel.index()]
            } else {
                0
            }
        })
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
        self.noise.mode.bits() | self.noise.rate.bits()
    }

    /// The shift rate the noise register selects.
    pub fn noise_rate(&self) -> NoiseRate {
        self.noise.rate
    }

    /// The feedback network the noise register selects.
    pub fn noise_mode(&self) -> NoiseMode {
        self.noise.mode
    }

    /// Which member of the family this is, and so how its registers read.
    pub fn variant(&self) -> Variant {
        self.variant
    }
}

/// One tone generator: its period register, the counter reloading from it, and
/// the frequency flip-flop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ToneState {
    pub period: u16,
    pub counter: u16,
    pub output: bool,
}

/// The noise generator: what the noise register selects, its counter and
/// flip-flop, and the shift register's contents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NoiseState {
    pub rate: NoiseRate,
    pub mode: NoiseMode,
    pub counter: u16,
    pub output: bool,
    pub shift_register: u16,
}

/// The whole chip at one instant: the register file, the generators behind it,
/// the prescaler, and the READY countdown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PsgState {
    /// The register address held between transfers.
    pub latched_channel: Channel,
    pub latched_kind: RegisterKind,
    pub tones: [ToneState; TONE_CHANNELS],
    pub noise: NoiseState,
    /// The four 4-bit attenuations, in register-address order.
    pub attenuations: [u8; CHANNELS],
    /// The prescaler's count toward an internal clock.
    pub clock_divider: u8,
    /// Input clocks left of the byte load holding READY low.
    pub ready_countdown: u8,
}

impl Psg {
    /// The chip's state at this instant. The variant is the part's identity,
    /// not its state, so it stays with the part a restore lands in.
    pub fn boundary_state(&self) -> PsgState {
        PsgState {
            latched_channel: self.latched.channel,
            latched_kind: self.latched.kind,
            tones: std::array::from_fn(|channel| ToneState {
                period: self.tones[channel].period,
                counter: self.tones[channel].counter,
                output: self.tones[channel].output,
            }),
            noise: NoiseState {
                rate: self.noise.rate,
                mode: self.noise.mode,
                counter: self.noise.counter,
                output: self.noise.output,
                shift_register: self.noise.lfsr,
            },
            attenuations: self.volumes,
            clock_divider: self.divider,
            ready_countdown: self.busy_clocks,
        }
    }

    pub fn restore_boundary(&mut self, state: &PsgState) {
        self.latched = LatchedRegister {
            channel: state.latched_channel,
            kind: state.latched_kind,
        };
        for (tone, captured) in self.tones.iter_mut().zip(state.tones) {
            tone.period = captured.period;
            tone.counter = captured.counter;
            tone.output = captured.output;
        }
        self.noise.rate = state.noise.rate;
        self.noise.mode = state.noise.mode;
        self.noise.counter = state.noise.counter;
        self.noise.output = state.noise.output;
        self.noise.lfsr = state.noise.shift_register;
        self.volumes = state.attenuations;
        self.divider = state.clock_divider;
        self.busy_clocks = state.ready_countdown;
    }
}

fn amplitude(attenuation: u8) -> f32 {
    if attenuation >= MUTE_ATTENUATION {
        0.0
    } else {
        10.0f32.powf(-DECIBELS_PER_STEP * f32::from(attenuation) / 20.0)
    }
}

#[cfg(test)]
mod tests;
