pub enum EnvelopeDirection {
    Decrease,
    Increase,
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
            Self::MIN..0 => Self(Self::SIGN & (value.abs() as u16)),
            _ => unreachable!(),
        }
    }
}
