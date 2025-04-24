pub struct Volume {
    volume: u8,
    initial_volume: u8,
    direction: EnvelopeDirection,
    pace: u8,
    timer: u8,
}

impl Volume {
    const RATE_VS_AUDIO_TIMER: u8 = 2;

    pub fn new(initial_volume: u8, direction: EnvelopeDirection, pace: u8) -> Self {
        Volume {
            volume: initial_volume,
            initial_volume,
            direction,
            pace,
            timer: 0,
        }
    }

    pub fn read_register(&self) -> u8 {
        let direction_bits = if self.direction == EnvelopeDirection::Increase {
            1 << 3
        } else {
            0
        };

        self.initial_volume << 4 | direction_bits | self.pace
    }

    pub fn write_register(&mut self, value: u8) {
        self.initial_volume = value >> 4;
        self.direction = if value & 0b1000 != 0 {
            EnvelopeDirection::Increase
        } else {
            EnvelopeDirection::Decrease
        };
        self.pace = value & 0b111;
        self.timer = 0;
    }

    pub fn initial_volume(&self) -> u8 {
        self.initial_volume
    }

    pub fn direction(&self) -> EnvelopeDirection {
        self.direction
    }

    pub fn current_volume(&self) -> u8 {
        self.volume
    }

    pub fn tick(&mut self) {
        if self.pace > 1 {
            match self.direction {
                EnvelopeDirection::Increase => {
                    if self.volume < 0xf && self.timer_tick() {
                        self.volume += 1;
                    }
                }
                EnvelopeDirection::Decrease => {
                    if self.volume > 0 && self.timer_tick() {
                        self.volume -= 1;
                    }
                }
            }
        }
    }

    fn timer_tick(&mut self) -> bool {
        self.timer += 1;
        if self.timer == Self::RATE_VS_AUDIO_TIMER {
            self.timer = 0;
            true
        } else {
            false
        }
    }

    pub fn trigger(&mut self) {
        self.timer = 0;
        self.volume = self.initial_volume;
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum EnvelopeDirection {
    Decrease,
    Increase,
}
