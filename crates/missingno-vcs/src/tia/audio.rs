//! TIA audio: two channels of AUDC-selected waveforms clocked from line
//! timing, so pitch is coupled to video rate. The waveform relationships
//! are the community-documented behavioural model; their gate-level
//! structure is unanalysed territory.

pub struct Channel {
    pub control: u8,
    pub frequency: u8,
    pub volume: u8,
    divider: u8,
    prescale: u8,
    poly4: u8,
    poly5: u8,
    poly9: u16,
    div31: u8,
    tone: bool,
    output: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self::new()
    }
}

impl Channel {
    pub fn new() -> Self {
        Channel {
            control: 0,
            frequency: 0,
            volume: 0,
            divider: 0,
            prescale: 0,
            poly4: 0x0F,
            poly5: 0x1F,
            poly9: 0x1FF,
            div31: 0,
            tone: false,
            output: true,
        }
    }

    /// One audio clock (two per scanline).
    pub fn tick(&mut self) {
        if self.divider == 0 {
            self.divider = self.frequency;
            self.clock_waveform();
        } else {
            self.divider -= 1;
        }
    }

    fn poly4_clock(&mut self) -> bool {
        let bit = (self.poly4 ^ (self.poly4 >> 1)) & 1;
        self.poly4 = (self.poly4 >> 1) | (bit << 3);
        self.poly4 & 1 != 0
    }

    fn poly5_clock(&mut self) -> bool {
        let bit = (self.poly5 ^ (self.poly5 >> 2)) & 1;
        self.poly5 = (self.poly5 >> 1) | (bit << 4);
        self.poly5 & 1 != 0
    }

    fn poly9_clock(&mut self) -> bool {
        let bit = (self.poly9 ^ (self.poly9 >> 4)) & 1;
        self.poly9 = (self.poly9 >> 1) | (bit << 8);
        self.poly9 & 1 != 0
    }

    /// The 31-step divider: 18 clocks high, 13 low.
    fn div31_clock(&mut self) -> bool {
        self.div31 = (self.div31 + 1) % 31;
        self.div31 < 18
    }

    /// Modes 0xC-0xF run their source at one third rate.
    fn third_rate(&mut self) -> bool {
        self.prescale = (self.prescale + 1) % 3;
        self.prescale == 0
    }

    fn clock_waveform(&mut self) {
        self.output = match self.control & 0x0F {
            0x0 | 0xB => true,
            0x1 => self.poly4_clock(),
            0x2 => {
                let gate = self.div31_clock();
                if gate {
                    self.poly4_clock();
                }
                self.poly4 & 1 != 0
            }
            0x3 => {
                if self.poly5_clock() {
                    self.poly4_clock();
                }
                self.poly4 & 1 != 0
            }
            0x4 | 0x5 => {
                self.tone = !self.tone;
                self.tone
            }
            0x6 | 0xA => self.div31_clock(),
            0x7 | 0x9 => self.poly5_clock(),
            0x8 => self.poly9_clock(),
            0xC | 0xD => {
                if self.third_rate() {
                    self.tone = !self.tone;
                }
                self.tone
            }
            0xE => {
                if self.third_rate() {
                    self.tone = self.div31_clock();
                }
                self.tone
            }
            _ => {
                if self.third_rate() {
                    self.tone = self.poly5_clock();
                }
                self.tone
            }
        };
    }

    /// Current level, 0-15.
    pub fn level(&self) -> u8 {
        if self.output { self.volume } else { 0 }
    }
}
