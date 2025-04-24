pub struct Waveform {
    waveform: u8,
    index: u8,
    counter: u16,
}

impl Waveform {
    const WAVEFORMS: [[u8; 16]; 4] = [
        [1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 0],
        [0, 1, 1, 1, 1, 1, 1, 0, 0, 1, 1, 1, 1, 1, 1, 0],
        [0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0],
        [1, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0, 1],
    ];

    pub fn new(waveform: u8) -> Self {
        Waveform {
            waveform,
            index: 0,
            counter: 0,
        }
    }

    pub fn waveform(&self) -> u8 {
        self.waveform
    }

    pub fn set_waveform(&mut self, waveform: u8) {
        self.waveform = waveform;
    }

    pub fn trigger(&mut self, period: u16) {
        self.index = 0;
        self.counter = period;
    }

    pub fn tick(&mut self, current_period: u16, current_volume: u8) -> u8 {
        self.counter += 1;

        if self.counter == 0x7ff {
            self.counter = current_period;
            self.index = (self.index + 1) % 16;
        }

        Self::WAVEFORMS[self.waveform as usize][self.index as usize] * current_volume
    }
}
