//! TIA audio: two channels of AUDC-selected waveforms clocked from line
//! timing, so pitch is coupled to video rate. The waveform relationships
//! are the community-documented behavioural model; their gate-level
//! structure is unanalysed territory.

/// The AUDCx waveform modes, named by mechanism in the community/Fries model
/// this channel implements (the TIA's audio gate structure is unanalysed
/// silicon; a few names follow implementation consensus over the Programmer's
/// Guide's labels — noted per variant).
#[derive(Clone, Copy)]
enum Waveform {
    Silence,    // 0x0, 0xB
    Poly4,      // 0x1
    Poly4Div31, // 0x2 — Fries model; the Programmer's Guide labels this div-15
    Poly5Poly4, // 0x3
    PureTone,   // 0x4, 0x5 — ÷2 square
    Div31Tone,  // 0x6, 0xA
    Poly5,      // 0x7, 0x9 — our model omits 0x7's Guide-documented extra ÷2
    Poly9Noise, // 0x8 — 511-period white noise
    Div6Tone,   // 0xC, 0xD — ÷3 prescale then ÷2
    Div93Tone,  // 0xE — ÷3 prescale then ÷31
    Poly5Div6,  // 0xF
}

impl Waveform {
    fn from_control(control: u8) -> Self {
        match control & 0x0F {
            0x0 | 0xB => Self::Silence,
            0x1 => Self::Poly4,
            0x2 => Self::Poly4Div31,
            0x3 => Self::Poly5Poly4,
            0x4 | 0x5 => Self::PureTone,
            0x6 | 0xA => Self::Div31Tone,
            0x7 | 0x9 => Self::Poly5,
            0x8 => Self::Poly9Noise,
            0xC | 0xD => Self::Div6Tone,
            0xE => Self::Div93Tone,
            _ => Self::Poly5Div6,
        }
    }
}

pub struct Channel {
    /// AUDC waveform select, AUDF frequency divisor, AUDV volume.
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
        self.output = match Waveform::from_control(self.control) {
            Waveform::Silence => true,
            Waveform::Poly4 => self.poly4_clock(),
            Waveform::Poly4Div31 => {
                let gate = self.div31_clock();
                if gate {
                    self.poly4_clock();
                }
                self.poly4 & 1 != 0
            }
            Waveform::Poly5Poly4 => {
                if self.poly5_clock() {
                    self.poly4_clock();
                }
                self.poly4 & 1 != 0
            }
            Waveform::PureTone => {
                self.tone = !self.tone;
                self.tone
            }
            Waveform::Div31Tone => self.div31_clock(),
            Waveform::Poly5 => self.poly5_clock(),
            Waveform::Poly9Noise => self.poly9_clock(),
            Waveform::Div6Tone => {
                if self.third_rate() {
                    self.tone = !self.tone;
                }
                self.tone
            }
            Waveform::Div93Tone => {
                if self.third_rate() {
                    self.tone = self.div31_clock();
                }
                self.tone
            }
            Waveform::Poly5Div6 => {
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
