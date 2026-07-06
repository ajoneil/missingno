//! TIA: beam timing, the five movable objects, playfield, collisions, and
//! the HMOVE motion mechanism.
//!
//! Motion is modelled as the hardware does it: HMOVE arms a per-object
//! "more movement" latch and a ripple counter delivers extra motion clocks
//! every four colour clocks until each object's comparator (HM value ^ 8)
//! matches; the strobe also latches an 8-clock hblank extension (the HMOVE
//! comb). Late/mid-line strobes reuse the same machinery, so the classic
//! "illegal HMOVE" positions emerge rather than being special-cased.

pub mod objects;

use objects::{Ball, Missile, Player, Playfield};

pub const CLOCKS_PER_LINE: u16 = 228;
pub const HBLANK_CLOCKS: u16 = 68;
pub const VISIBLE_CLOCKS: usize = 160;
const LATE_HBLANK_CLOCKS: u16 = HBLANK_CLOCKS + 8;

mod registers {
    pub const VSYNC: u16 = 0x00;
    pub const VBLANK: u16 = 0x01;
    pub const WSYNC: u16 = 0x02;
    pub const RSYNC: u16 = 0x03;
    pub const NUSIZ0: u16 = 0x04;
    pub const NUSIZ1: u16 = 0x05;
    pub const COLUP0: u16 = 0x06;
    pub const COLUP1: u16 = 0x07;
    pub const COLUPF: u16 = 0x08;
    pub const COLUBK: u16 = 0x09;
    pub const CTRLPF: u16 = 0x0A;
    pub const REFP0: u16 = 0x0B;
    pub const REFP1: u16 = 0x0C;
    pub const PF0: u16 = 0x0D;
    pub const PF1: u16 = 0x0E;
    pub const PF2: u16 = 0x0F;
    pub const RESP0: u16 = 0x10;
    pub const RESP1: u16 = 0x11;
    pub const RESM0: u16 = 0x12;
    pub const RESM1: u16 = 0x13;
    pub const RESBL: u16 = 0x14;
    pub const GRP0: u16 = 0x1B;
    pub const GRP1: u16 = 0x1C;
    pub const ENAM0: u16 = 0x1D;
    pub const ENAM1: u16 = 0x1E;
    pub const ENABL: u16 = 0x1F;
    pub const HMP0: u16 = 0x20;
    pub const HMP1: u16 = 0x21;
    pub const HMM0: u16 = 0x22;
    pub const HMM1: u16 = 0x23;
    pub const HMBL: u16 = 0x24;
    pub const VDELP0: u16 = 0x25;
    pub const VDELP1: u16 = 0x26;
    pub const VDELBL: u16 = 0x27;
    pub const RESMP0: u16 = 0x28;
    pub const RESMP1: u16 = 0x29;
    pub const HMOVE: u16 = 0x2A;
    pub const HMCLR: u16 = 0x2B;
    pub const CXCLR: u16 = 0x2C;
}

/// One colour clock's opaque-output bits, feeding compose and collisions.
#[derive(Clone, Copy)]
struct Pixels {
    p0: bool,
    p1: bool,
    m0: bool,
    m1: bool,
    bl: bool,
    pf: bool,
}

/// One finished scanline: 160 TIA colour indices plus its blanking state.
#[derive(Clone)]
pub struct Scanline {
    pub pixels: [u8; VISIBLE_CLOCKS],
    pub vsync: bool,
    pub vblank: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MovableIndex {
    P0,
    P1,
    M0,
    M1,
    Bl,
}
const MOVABLES: [MovableIndex; 5] = [
    MovableIndex::P0,
    MovableIndex::P1,
    MovableIndex::M0,
    MovableIndex::M1,
    MovableIndex::Bl,
];

/// The HMOVE ripple sequence: latches armed by the strobe, cleared when
/// the up-counter matches an object's comparator; a set latch converts
/// each four-clock pulse into an extra motion clock for its object.
struct MotionSequencer {
    active: bool,
    pulse_count: u8,
    more_movement: [bool; 5],
    values: [u8; 5],
}

impl MotionSequencer {
    fn new() -> Self {
        MotionSequencer {
            active: false,
            pulse_count: 0,
            more_movement: [false; 5],
            values: [0; 5],
        }
    }

    fn strobe(&mut self) {
        self.active = true;
        self.pulse_count = 0;
        self.more_movement = [true; 5];
    }

    /// Which objects receive an extra clock on this pulse.
    fn pulse(&mut self) -> [bool; 5] {
        let mut ticks = [false; 5];
        for (i, more) in self.more_movement.iter_mut().enumerate() {
            if *more && self.pulse_count == (self.values[i] >> 4) ^ 0x08 {
                *more = false;
            }
            ticks[i] = *more;
        }
        self.pulse_count += 1;
        if self.pulse_count == 16 {
            self.active = false;
        }
        ticks
    }
}

pub struct Tia {
    beam: u16,
    vsync: bool,
    vblank: bool,
    /// Low while a WSYNC strobe holds the CPU; released at line start.
    pub cpu_ready: bool,

    player0: Player,
    player1: Player,
    missile0: Missile,
    missile1: Missile,
    ball: Ball,
    playfield: Playfield,

    color_p0: u8,
    color_p1: u8,
    color_pf: u8,
    color_bk: u8,
    playfield_priority: bool,
    score_mode: bool,

    motion: MotionSequencer,
    late_hblank: bool,

    collisions: [u8; 8],

    /// Trigger buttons, true = pressed (the pin reads low).
    pub triggers: [bool; 2],
    trigger_latch_enabled: bool,
    trigger_latches: [bool; 2],

    line: [u8; VISIBLE_CLOCKS],
    finished_line: Option<Scanline>,
}

impl Default for Tia {
    fn default() -> Self {
        Self::new()
    }
}

impl Tia {
    pub fn new() -> Self {
        Tia {
            beam: 0,
            vsync: false,
            vblank: false,
            cpu_ready: true,
            player0: Player::new(),
            player1: Player::new(),
            missile0: Missile::new(),
            missile1: Missile::new(),
            ball: Ball::new(),
            playfield: Playfield::new(),
            color_p0: 0,
            color_p1: 0,
            color_pf: 0,
            color_bk: 0,
            playfield_priority: false,
            score_mode: false,
            motion: MotionSequencer::new(),
            late_hblank: false,
            collisions: [0; 8],
            triggers: [false; 2],
            trigger_latch_enabled: false,
            trigger_latches: [true; 2],
            line: [0; VISIBLE_CLOCKS],
            finished_line: None,
        }
    }

    /// Current colour clock within the line (0..228) — inspection only.
    pub fn beam(&self) -> u16 {
        self.beam
    }

    pub fn take_line(&mut self) -> Option<Scanline> {
        self.finished_line.take()
    }

    fn tick_movable(&mut self, which: MovableIndex) -> bool {
        match which {
            MovableIndex::P0 => self.player0.tick(),
            MovableIndex::P1 => self.player1.tick(),
            MovableIndex::M0 => self.missile0.tick(),
            MovableIndex::M1 => self.missile1.tick(),
            MovableIndex::Bl => self.ball.tick(),
        }
    }

    /// Advance one colour clock; completed lines surface via `take_line`.
    pub fn step_clock(&mut self) {
        // The motion sequencer's extra clocks ride every fourth colour
        // clock, hblank included — that is where HMOVE movement happens.
        if self.motion.active && self.beam.is_multiple_of(4) {
            let ticks = self.motion.pulse();
            for (i, &tick) in ticks.iter().enumerate() {
                if tick {
                    self.tick_movable(MOVABLES[i]);
                }
            }
        }

        let hblank_end = if self.late_hblank {
            LATE_HBLANK_CLOCKS
        } else {
            HBLANK_CLOCKS
        };
        if self.beam >= hblank_end {
            self.render_clock();
        } else if self.beam >= HBLANK_CLOCKS {
            // Inside the HMOVE comb: blanked, and motion clocks gated.
            self.line[(self.beam - HBLANK_CLOCKS) as usize] = 0;
        }

        self.beam += 1;
        if self.beam == CLOCKS_PER_LINE {
            self.beam = 0;
            self.cpu_ready = true;
            self.late_hblank = false;
            self.finished_line = Some(Scanline {
                pixels: self.line,
                vsync: self.vsync,
                vblank: self.vblank,
            });
        }
    }

    fn render_clock(&mut self) {
        let x = (self.beam - HBLANK_CLOCKS) as u8;
        let px = Pixels {
            p0: self.player0.tick(),
            p1: self.player1.tick(),
            m0: self.missile0.tick(),
            m1: self.missile1.tick(),
            bl: self.ball.tick(),
            pf: self.playfield.pixel(x),
        };

        self.latch_collisions(px);

        let color = if self.vblank { 0 } else { self.compose(x, px) };
        self.line[x as usize] = color & 0xFE;
    }

    fn latch_collisions(&mut self, px: Pixels) {
        let Pixels {
            p0,
            p1,
            m0,
            m1,
            bl,
            pf,
        } = px;
        let pairs: [(usize, bool, bool); 8] = [
            (0, m0 && p1, m0 && p0),
            (1, m1 && p0, m1 && p1),
            (2, p0 && pf, p0 && bl),
            (3, p1 && pf, p1 && bl),
            (4, m0 && pf, m0 && bl),
            (5, m1 && pf, m1 && bl),
            (6, bl && pf, false),
            (7, p0 && p1, m0 && m1),
        ];
        for (index, high, low) in pairs {
            if high {
                self.collisions[index] |= 0x80;
            }
            if low {
                self.collisions[index] |= 0x40;
            }
        }
    }

    fn compose(&self, x: u8, px: Pixels) -> u8 {
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

    pub fn write(&mut self, address: u16, value: u8) {
        use registers::*;
        match address & 0x3F {
            VSYNC => self.vsync = value & 0x02 != 0,
            VBLANK => {
                self.vblank = value & 0x02 != 0;
                let latch_enable = value & 0x40 != 0;
                if !latch_enable {
                    self.trigger_latches = [true; 2];
                }
                self.trigger_latch_enabled = latch_enable;
            }
            WSYNC => self.cpu_ready = false,
            RSYNC => self.beam = 0,
            NUSIZ0 => {
                self.player0.nusiz = value;
                self.missile0.nusiz = value;
            }
            NUSIZ1 => {
                self.player1.nusiz = value;
                self.missile1.nusiz = value;
            }
            COLUP0 => self.color_p0 = value,
            COLUP1 => self.color_p1 = value,
            COLUPF => self.color_pf = value,
            COLUBK => self.color_bk = value,
            CTRLPF => {
                self.playfield.mirrored = value & 0x01 != 0;
                self.score_mode = value & 0x02 != 0;
                self.playfield_priority = value & 0x04 != 0;
                self.ball.width_exponent = (value >> 4) & 0x03;
            }
            REFP0 => self.player0.reflect = value & 0x08 != 0,
            REFP1 => self.player1.reflect = value & 0x08 != 0,
            PF0 => self.playfield.pf0 = value,
            PF1 => self.playfield.pf1 = value,
            PF2 => self.playfield.pf2 = value,
            RESP0 => self.player0.reset_position(),
            RESP1 => self.player1.reset_position(),
            RESM0 => self.missile0.reset_position(),
            RESM1 => self.missile1.reset_position(),
            RESBL => self.ball.reset_position(),
            // The vertical-delay latches cross-couple: a GRP0 write
            // freezes player 1's old graphics, a GRP1 write freezes
            // player 0's and the ball's.
            GRP0 => {
                self.player0.grp_new = value;
                self.player1.grp_old = self.player1.grp_new;
            }
            GRP1 => {
                self.player1.grp_new = value;
                self.player0.grp_old = self.player0.grp_new;
                self.ball.enabled_old = self.ball.enabled_new;
            }
            ENAM0 => self.missile0.enabled = value & 0x02 != 0,
            ENAM1 => self.missile1.enabled = value & 0x02 != 0,
            ENABL => self.ball.enabled_new = value & 0x02 != 0,
            HMP0 => self.motion.values[0] = value,
            HMP1 => self.motion.values[1] = value,
            HMM0 => self.motion.values[2] = value,
            HMM1 => self.motion.values[3] = value,
            HMBL => self.motion.values[4] = value,
            VDELP0 => self.player0.vertical_delay = value & 0x01 != 0,
            VDELP1 => self.player1.vertical_delay = value & 0x01 != 0,
            VDELBL => self.ball.vertical_delay = value & 0x01 != 0,
            RESMP0 => {
                let lock = value & 0x02 != 0;
                if self.missile0.locked_to_player && !lock {
                    self.missile0
                        .release_at(self.player0.counter(), self.player0.nusiz);
                }
                self.missile0.locked_to_player = lock;
            }
            RESMP1 => {
                let lock = value & 0x02 != 0;
                if self.missile1.locked_to_player && !lock {
                    self.missile1
                        .release_at(self.player1.counter(), self.player1.nusiz);
                }
                self.missile1.locked_to_player = lock;
            }
            HMOVE => {
                self.motion.strobe();
                self.late_hblank = true;
            }
            HMCLR => self.motion.values = [0; 5],
            CXCLR => self.collisions = [0; 8],
            _ => {}
        }
    }

    pub fn read(&mut self, address: u16) -> u8 {
        match address & 0x0F {
            reg @ 0x00..=0x07 => self.collisions[reg as usize],
            // INPT0-3 pot inputs: no RC charge model — they read as
            // permanently dumped (discharged).
            0x08..=0x0B => 0x00,
            reg @ (0x0C | 0x0D) => {
                let port = (reg - 0x0C) as usize;
                if self.trigger_latch_enabled && self.triggers[port] {
                    self.trigger_latches[port] = false;
                }
                let level = if self.trigger_latch_enabled {
                    self.trigger_latches[port]
                } else {
                    !self.triggers[port]
                };
                if level { 0x80 } else { 0x00 }
            }
            _ => 0x00,
        }
    }
}
