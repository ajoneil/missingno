#[derive(PartialEq, Eq)]
pub enum EnvelopeDirection {
    Decrease,
    Increase,
}

#[derive(Copy, Clone)]
pub struct WaveformAndInitialLength(pub u8);
impl WaveformAndInitialLength {
    pub fn waveform(&self) -> u8 {
        self.0 >> 6
    }

    pub fn initial_length(&self) -> u8 {
        self.0 & 0b0011_1111
    }
}

#[derive(Copy, Clone)]
pub struct VolumeAndEnvelope(pub u8);

impl VolumeAndEnvelope {
    pub fn initial_volume(&self) -> u8 {
        self.0 >> 4
    }

    pub fn initial_volume_percent(&self) -> f32 {
        self.initial_volume() as f32 / 15.0
    }

    pub fn direction(&self) -> EnvelopeDirection {
        if self.0 & 0b1000 != 0 {
            EnvelopeDirection::Increase
        } else {
            EnvelopeDirection::Decrease
        }
    }

    pub fn sweep_pace(&self) -> u8 {
        self.0 & 0b111
    }
}

#[derive(Copy, Clone)]
pub struct PeriodHighAndControl(pub u8);

impl PeriodHighAndControl {
    const LENGTH: u8 = 0b0100_0000;

    pub fn read(length_enabled: bool) -> u8 {
        if length_enabled {
            0xff
        } else {
            0xff ^ Self::LENGTH
        }
    }

    pub fn trigger(&self) -> bool {
        self.0 & 0b1000_0000 != 0
    }

    pub fn enable_length(&self) -> bool {
        self.0 & Self::LENGTH != 0
    }

    pub fn period_high(&self) -> u8 {
        self.0 & 0b0000_0111
    }
}

#[derive(Copy, Clone)]
pub struct Signed11(pub u16);

impl Signed11 {
    const HIGH: u16 = 0b111_0000_0000;
    const LOW: u16 = 0b000_1111_1111;
    const SIGN: u16 = 0b100_0000_0000;
    const VALUE: u16 = 0b011_1111_1111;

    const MAX: i16 = 1023;
    const MIN: i16 = -1024;

    pub fn set_high3(&mut self, high: u8) {
        self.0 = (self.0 & Self::LOW) | ((high as u16) << 8);
    }

    pub fn set_low8(&mut self, low: u8) {
        self.0 = (self.0 & Self::HIGH) | low as u16;
    }
}

impl Into<i16> for Signed11 {
    fn into(self) -> i16 {
        if self.0 & Self::SIGN != 0 {
            -((!self.0 & Self::VALUE) as i16)
        } else {
            (self.0 & Self::VALUE) as i16
        }
    }
}
impl From<i16> for Signed11 {
    fn from(value: i16) -> Self {
        match value {
            0..=Self::MAX => Self(value as u16),
            Self::MIN..0 => Self(Self::SIGN | (value.abs() as u16)),
            _ => unreachable!(),
        }
    }
}

/// CH1/CH2 prescaler: AJER + CALO (CH1) / ATEP + CEMO (CH2). Two
/// toggle DFFs that divide `apu_4mhz` down to 1 MHz, free-running and
/// reset only by APU power-off. NR14/NR24 trigger writes have no
/// input to these stages — that's the silicon-level realisation of
/// "low two bits of the frequency timer are NOT modified."
#[derive(Clone, Default)]
pub struct Prescaler {
    pub counter: u8,
}

impl Prescaler {
    /// Advance by one T-cycle. Returns true on the wrap edge — the
    /// `chN_1mhz` rising edge that clocks the period divider.
    pub fn tcycle(&mut self) -> bool {
        self.counter = (self.counter + 1) & 0b11;
        self.counter == 0
    }

    pub fn power_off(&mut self) {
        self.counter = 0;
    }
}

/// CH1/CH2 period divider: 11-bit upcounter (GAXE..COPU on CH1,
/// DONE..HERO on CH2). Counts from the loaded `period` value up to
/// 0x7FF; the next 1 MHz tick after 0x7FF overflows and reloads from
/// `period`, advancing the duty step. Natural-overflow and trigger
/// reload share the same load enable (`epyk` / `duju`) — there is no
/// subset-of-stages distinction.
#[derive(Clone, Default)]
pub struct PeriodDivider {
    pub counter: u16,
}

impl PeriodDivider {
    /// Advance by one 1 MHz tick (called when the prescaler wraps).
    /// Returns true on overflow — duty step advances, counter reloads
    /// to `period`.
    pub fn tick(&mut self, period: u16) -> bool {
        if self.counter >= 0x7FF {
            self.counter = period & 0x7FF;
            true
        } else {
            self.counter += 1;
            false
        }
    }

    /// NR14/NR24 trigger reload. Only the divider is touched; the
    /// prescaler upstream keeps running.
    pub fn trigger_reload(&mut self, period: u16) {
        self.counter = period & 0x7FF;
    }

    pub fn power_off(&mut self) {
        self.counter = 0;
    }
}
