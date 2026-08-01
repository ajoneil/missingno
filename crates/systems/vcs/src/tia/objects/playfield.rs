//! The playfield: beam-derived rather than counter-driven, so it has no ÷4
//! ring of its own — the serialiser reads its registers once per 4-clock cell.

/// The playfield is beam-derived, not counter-driven: 20 bits across the
/// left half (PF0 high nibble low-bit-first, PF1 high-bit-first, PF2
/// low-bit-first), repeated or mirrored on the right per CTRLPF.
pub struct Playfield {
    pub pf0: u8,
    pub pf1: u8,
    pub pf2: u8,
    pub mirrored: bool,
    /// The serialiser reads the registers once per 4-clock cell; a write
    /// landing mid-cell takes effect from the next cell.
    latched: [u8; 3],
}

impl Default for Playfield {
    fn default() -> Self {
        Self::new()
    }
}

impl Playfield {
    pub fn new() -> Self {
        Playfield {
            pf0: 0,
            pf1: 0,
            pf2: 0,
            mirrored: false,
            latched: [0; 3],
        }
    }

    pub fn latch_cell(&mut self) {
        self.latched = [self.pf0, self.pf1, self.pf2];
    }

    pub fn pixel(&self, x: u8) -> bool {
        let cell = if x < 80 {
            x / 4
        } else if self.mirrored {
            // The reflected right half scans the same 20 cells backwards.
            19 - (x - 80) / 4
        } else {
            (x - 80) / 4
        };
        let [pf0, pf1, pf2] = self.latched;
        let lit = match cell {
            0..=3 => pf0 & (0x10 << cell),
            4..=11 => pf1 & (0x80 >> (cell - 4)),
            _ => pf2 & (0x01 << (cell - 12)),
        };
        lit != 0
    }
}

/// The playfield's registers and its per-cell serialiser latch.
#[derive(Clone, Copy)]
pub(crate) struct PlayfieldState {
    pub pf0: u8,
    pub pf1: u8,
    pub pf2: u8,
    pub mirrored: bool,
    pub latched: [u8; 3],
}

impl Playfield {
    pub(crate) fn capture(&self) -> PlayfieldState {
        PlayfieldState {
            pf0: self.pf0,
            pf1: self.pf1,
            pf2: self.pf2,
            mirrored: self.mirrored,
            latched: self.latched,
        }
    }

    pub(crate) fn restore(&mut self, s: &PlayfieldState) {
        self.pf0 = s.pf0;
        self.pf1 = s.pf1;
        self.pf2 = s.pf2;
        self.mirrored = s.mirrored;
        self.latched = s.latched;
    }
}
