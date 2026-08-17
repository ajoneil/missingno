//! What a colour clock does with the six object outputs: latch the collision
//! pairs, then resolve the priority ladder into one colour byte.

/// One colour clock's opaque-output bits, feeding compose and collisions.
#[derive(Clone, Copy)]
pub(super) struct Pixels {
    pub(super) p0: bool,
    pub(super) p1: bool,
    pub(super) m0: bool,
    pub(super) m1: bool,
    pub(super) bl: bool,
    pub(super) pf: bool,
}

/// The two collision bits packed into each CXxx latch: D7 and D6.
const COLLISION_HIGH: u8 = 0x80;
const COLLISION_LOW: u8 = 0x40;

/// The eight collision latches, ordered as the CXxx read registers ($00–$07).
#[derive(Clone, Copy)]
enum CollisionRegister {
    M0P,  // CXM0P
    M1P,  // CXM1P
    P0FB, // CXP0FB
    P1FB, // CXP1FB
    M0FB, // CXM0FB
    M1FB, // CXM1FB
    BlPf, // CXBLPF
    PpMm, // CXPPMM
}

pub(super) struct Collisions(pub(super) [u8; 8]);

impl Collisions {
    pub(super) fn new() -> Self {
        Collisions([0; 8])
    }

    pub(super) fn latch(&mut self, px: Pixels) {
        let Pixels {
            p0,
            p1,
            m0,
            m1,
            bl,
            pf,
        } = px;
        use CollisionRegister::*;
        let pairs = [
            (M0P, m0 && p1, m0 && p0),
            (M1P, m1 && p0, m1 && p1),
            (P0FB, p0 && pf, p0 && bl),
            (P1FB, p1 && pf, p1 && bl),
            (M0FB, m0 && pf, m0 && bl),
            (M1FB, m1 && pf, m1 && bl),
            (BlPf, bl && pf, false),
            (PpMm, p0 && p1, m0 && m1),
        ];
        for (register, high, low) in pairs {
            if high {
                self.0[register as usize] |= COLLISION_HIGH;
            }
            if low {
                self.0[register as usize] |= COLLISION_LOW;
            }
        }
    }

    pub(super) fn clear(&mut self) {
        self.0 = [0; 8];
    }
}

/// The colour registers and the CTRLPF bits that pick between them: the
/// priority ladder resolving each clock's opaque objects to one colour byte.
pub(super) struct ColorMux {
    pub(super) color_p0: u8,
    pub(super) color_p1: u8,
    pub(super) color_pf: u8,
    pub(super) color_bk: u8,
    pub(super) playfield_priority: bool,
    pub(super) score_mode: bool,
}

impl ColorMux {
    pub(super) fn new() -> Self {
        ColorMux {
            color_p0: 0,
            color_p1: 0,
            color_pf: 0,
            color_bk: 0,
            playfield_priority: false,
            score_mode: false,
        }
    }

    pub(super) fn compose(&self, x: u8, px: Pixels) -> u8 {
        let Pixels {
            p0,
            p1,
            m0,
            m1,
            bl,
            pf,
        } = px;
        let playfield_color = if self.score_mode {
            if x < 80 { self.color_p0 } else { self.color_p1 }
        } else {
            self.color_pf
        };
        if self.playfield_priority && (pf || bl) {
            return if pf { playfield_color } else { self.color_pf };
        }
        if p0 || m0 {
            self.color_p0
        } else if p1 || m1 {
            self.color_p1
        } else if bl {
            self.color_pf
        } else if pf {
            playfield_color
        } else {
            self.color_bk
        }
    }
}
