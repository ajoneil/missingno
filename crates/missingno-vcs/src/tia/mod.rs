//! TIA: beam timing, the five movable objects, playfield, collisions, and
//! the HMOVE motion mechanism.
//!
//! Motion is modelled as the hardware does it: HMOVE arms a per-object
//! "more movement" latch and a ripple counter delivers extra motion clocks
//! every four colour clocks until each object's comparator (HM value ^ 8)
//! matches; the strobe also latches an 8-clock hblank extension (the HMOVE
//! comb). Late/mid-line strobes reuse the same machinery, so the classic
//! "illegal HMOVE" positions emerge rather than being special-cased.

pub(crate) mod audio;
pub(crate) mod objects;

use audio::Channel;
use objects::{Ball, Missile, Player, Playfield};

use crate::TvStandard;

pub const CLOCKS_PER_LINE: u16 = 228;
pub const HBLANK_CLOCKS: u16 = 68;
pub const VISIBLE_CLOCKS: usize = 160;
const LATE_HBLANK_CLOCKS: u16 = HBLANK_CLOCKS + 8;
/// Colour clocks from an HMOVE write reaching the TIA to its SEC decode.
const HBLANK_EXTENSION_DECODE_CLOCKS: u8 = 3;
/// The RHB/LRHB choice: hblank ends at 68, or 76 when SEC is holding.
const RESET_SELECT_CLOCK: u16 = 64;
/// SEC decode + latch set: the strobe's first stuffed pulse lands this
/// many colour clocks after the write reaches the TIA.
const MOTION_START_CLOCKS: u8 = 9;
/// The motion ripple counter's value between sequences (%1111).
const RESTING_RIPPLE: u8 = 15;
const AUDIO_CLOCK_A: u16 = 10;
const AUDIO_CLOCK_B: u16 = 124;
/// Full-scale paddle charge time; the readable range games sweep.
const POT_CHARGE_LINES: f32 = 380.0;

pub(crate) mod registers {
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
    pub const AUDC0: u16 = 0x15;
    pub const AUDC1: u16 = 0x16;
    pub const AUDF0: u16 = 0x17;
    pub const AUDF1: u16 = 0x18;
    pub const AUDV0: u16 = 0x19;
    pub const AUDV1: u16 = 0x1A;
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

/// One finished scanline: 160 TIA colour indices plus its VSYNC state.
#[derive(Clone)]
pub(crate) struct Scanline {
    pub pixels: [u8; VISIBLE_CLOCKS],
    pub vsync: bool,
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

/// The HMOVE motion sequencer. Its comparators read the live HM values, so
/// a mid-sequence rewrite that never matches leaves the latch stuffing
/// clocks every pulse until the next HMOVE — HMCLR clears only the HM values.
struct MotionSequencer {
    /// Colour clocks until the first stuffed pulse after a strobe.
    start_countdown: Option<u8>,
    /// Descending comparator ripple; comparisons stop when it runs out.
    ripple: Option<u8>,
    pulse_phase: u8,
    more_movement: [bool; 5],
    /// HM values, indexed in MOVABLES order: P0, P1, M0, M1, BL.
    values: [u8; 5],
}

impl MotionSequencer {
    fn new() -> Self {
        MotionSequencer {
            start_countdown: None,
            ripple: None,
            pulse_phase: 0,
            more_movement: [false; 5],
            values: [0; 5],
        }
    }

    fn strobe(&mut self) {
        self.start_countdown = Some(MOTION_START_CLOCKS);
        self.more_movement = [true; 5];
    }

    fn any_movement(&self) -> bool {
        self.more_movement.iter().any(|&m| m)
    }

    /// Advance one colour clock; `Some(ticks)` on a pulse, where a set
    /// latch requests an extra motion clock for its object.
    fn step(&mut self) -> Option<[bool; 5]> {
        if let Some(remaining) = self.start_countdown {
            if remaining > 0 {
                self.start_countdown = Some(remaining - 1);
                return None;
            }
            self.start_countdown = None;
            self.ripple = Some(15);
            self.pulse_phase = 0;
        } else {
            if !self.any_movement() {
                return None;
            }
            self.pulse_phase = (self.pulse_phase + 1) % 4;
            if self.pulse_phase != 0 {
                return None;
            }
        }

        let mut ticks = [false; 5];
        // The exhausted ripple rests at %1111 with the comparator still
        // wired: rewriting HM to $8x clears a latch stuck past the ripple.
        let ripple = self.ripple.unwrap_or(RESTING_RIPPLE);
        for (i, more) in self.more_movement.iter_mut().enumerate() {
            if *more && ripple == (self.values[i] >> 4) ^ 0x07 {
                *more = false;
            }
            ticks[i] = *more;
        }
        self.ripple = match self.ripple {
            Some(0) | None => None,
            Some(r) => Some(r - 1),
        };
        Some(ticks)
    }
}

pub struct Tia {
    beam: u16,
    vsync: bool,
    vblank: bool,
    /// Low while a WSYNC strobe holds the CPU; released at line start.
    pub(crate) cpu_ready: bool,

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
    /// HMOVE's SEC decode on its way to (or holding at) the reset-select.
    hblank_extension_pending: Option<u8>,
    hblank_extension_armed: bool,

    collisions: [u8; 8],

    audio: [Channel; 2],

    /// Trigger buttons, true = pressed (the pin reads low).
    triggers: [bool; 2],
    trigger_latch_enabled: bool,
    trigger_latches: [bool; 2],

    /// Paddle knob positions, 0.0 (instant charge) to 1.0 (slowest).
    pot_positions: [f32; 4],
    pot_dumped: bool,
    pot_countdown: [u16; 4],

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
            hblank_extension_pending: None,
            hblank_extension_armed: false,
            collisions: [0; 8],
            audio: [Channel::new(), Channel::new()],
            triggers: [false; 2],
            trigger_latch_enabled: false,
            trigger_latches: [true; 2],
            pot_positions: [0.5; 4],
            pot_dumped: false,
            pot_countdown: [0; 4],
            line: [0; VISIBLE_CLOCKS],
            finished_line: None,
        }
    }

    /// Point a paddle knob: 0.0 charges instantly, 1.0 slowest.
    pub fn set_paddle(&mut self, index: usize, position: f32) {
        self.pot_positions[index] = position.clamp(0.0, 1.0);
    }

    /// A trigger button's state into INPT4/5, true = pressed.
    pub fn set_trigger(&mut self, port: usize, pressed: bool) {
        self.triggers[port] = pressed;
        // The I4/I5 latches capture any low level while enabled, read or
        // no read — the feature's point for once-a-frame pollers.
        if self.trigger_latch_enabled && pressed {
            self.trigger_latches[port] = false;
        }
    }

    /// The two channels' summed output, 0.0-1.0.
    pub fn audio_level(&self) -> f32 {
        (self.audio[0].level() + self.audio[1].level()) as f32 / 30.0
    }

    /// Current colour clock within the line (0..228) — inspection only.
    pub fn beam(&self) -> u16 {
        self.beam
    }

    pub(crate) fn take_line(&mut self) -> Option<Scanline> {
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

    /// RHB/LRHB: hblank ends at 68, or 76 when the extension latched.
    fn hblank_end(&self) -> u16 {
        if self.late_hblank {
            LATE_HBLANK_CLOCKS
        } else {
            HBLANK_CLOCKS
        }
    }

    /// Advance one colour clock; completed lines surface via `take_line`.
    pub(crate) fn step_clock(&mut self) {
        // SEC decode: the reset-select at CLK 64 samples it, choosing the
        // extended hblank; a countdown straddling the wrap arms next line.
        if let Some(remaining) = self.hblank_extension_pending {
            if remaining == 0 {
                self.hblank_extension_pending = None;
                self.hblank_extension_armed = true;
            } else {
                self.hblank_extension_pending = Some(remaining - 1);
            }
        }
        if self.beam == RESET_SELECT_CLOCK {
            self.late_hblank = self.hblank_extension_armed;
        }

        // Stuffed motion clocks only move an object while the beam is
        // blanked; visible-region pulses advance the ripple but no object.
        if let Some(ticks) = self.motion.step()
            && self.beam < self.hblank_end()
        {
            for (i, &tick) in ticks.iter().enumerate() {
                if tick {
                    self.tick_movable(MOVABLES[i]);
                }
            }
        }

        if self.beam >= self.hblank_end() {
            self.render_clock();
        } else if self.beam >= HBLANK_CLOCKS {
            // Inside the HMOVE comb: blanked, and motion clocks gated.
            self.line[(self.beam - HBLANK_CLOCKS) as usize] = 0;
        }

        // The audio circuits clock twice per scanline (~31.4 kHz).
        if self.beam == AUDIO_CLOCK_A || self.beam == AUDIO_CLOCK_B {
            self.audio[0].tick();
            self.audio[1].tick();
        }

        self.beam += 1;
        if self.beam == CLOCKS_PER_LINE {
            self.end_line();
        }
    }

    /// The HSync-counter wrap: one mechanism with two triggers — the
    /// natural end of line, and RSYNC forcing it early.
    fn end_line(&mut self) {
        self.beam = 0;
        self.cpu_ready = true;
        self.late_hblank = false;
        self.hblank_extension_armed = false;
        if !self.pot_dumped {
            for countdown in &mut self.pot_countdown {
                *countdown = countdown.saturating_sub(1);
            }
        }
        self.finished_line = Some(Scanline {
            pixels: self.line,
            vsync: self.vsync,
        });
    }

    fn render_clock(&mut self) {
        let x = (self.beam - HBLANK_CLOCKS) as u8;
        if x.is_multiple_of(4) {
            self.playfield.latch_cell();
        }
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

    /// The reset strobe's leading scan-kill, one clock before it applies.
    pub(crate) fn missile_reset_kill(&mut self, which: usize) {
        match which {
            0 => self.missile0.reset_kill(),
            _ => self.missile1.reset_kill(),
        }
    }

    pub(crate) fn ball_reset_kill(&mut self) {
        self.ball.reset_kill();
    }

    pub(crate) fn write(&mut self, address: u16, value: u8) {
        use registers::*;
        match address & 0x3F {
            VSYNC => self.vsync = value & 0x02 != 0,
            VBLANK => {
                self.vblank = value & 0x02 != 0;
                let latch_enable = value & 0x40 != 0;
                if !latch_enable {
                    self.trigger_latches = [true; 2];
                } else if !self.trigger_latch_enabled {
                    // Enabling captures a button already held.
                    for port in 0..2 {
                        if self.triggers[port] {
                            self.trigger_latches[port] = false;
                        }
                    }
                }
                self.trigger_latch_enabled = latch_enable;
                // D7 grounds the pot capacitors; releasing it starts the
                // RC charge, measured by software in scanlines.
                let dump = value & 0x80 != 0;
                if self.pot_dumped && !dump {
                    for (countdown, position) in
                        self.pot_countdown.iter_mut().zip(self.pot_positions)
                    {
                        *countdown = (position.clamp(0.0, 1.0) * POT_CHARGE_LINES) as u16;
                    }
                }
                self.pot_dumped = dump;
            }
            WSYNC => self.cpu_ready = false,
            RSYNC => {
                // The forced wrap ends the line where it stands: the TV
                // gets a short line — undrawn pixels never left the gun.
                let drawn = (self.beam.saturating_sub(HBLANK_CLOCKS) as usize).min(VISIBLE_CLOCKS);
                self.line[drawn..].fill(0);
                self.end_line();
            }
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
            AUDC0 => self.audio[0].control = value,
            AUDC1 => self.audio[1].control = value,
            AUDF0 => self.audio[0].frequency = value & 0x1F,
            AUDF1 => self.audio[1].frequency = value & 0x1F,
            AUDV0 => self.audio[0].volume = value & 0x0F,
            AUDV1 => self.audio[1].volume = value & 0x0F,
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
                self.hblank_extension_pending = Some(HBLANK_EXTENSION_DECODE_CLOCKS);
            }
            HMCLR => self.motion.values = [0; 5],
            CXCLR => self.collisions = [0; 8],
            _ => {}
        }
    }

    fn pot_level(&self, index: usize) -> u8 {
        if !self.pot_dumped && self.pot_countdown[index] == 0 {
            0x80
        } else {
            0x00
        }
    }

    /// What a read returns with `floating` held on the data bus: the TIA
    /// drives D7-D6 on collision reads and D7 on input reads; every
    /// undriven line keeps the bus's retained byte. Side-effect-free.
    pub fn read(&self, address: u16, floating: u8) -> u8 {
        match address & 0x0F {
            reg @ 0x00..=0x07 => self.collisions[reg as usize] | (floating & 0x3F),
            reg @ 0x08..=0x0B => self.pot_level((reg - 0x08) as usize) | (floating & 0x7F),
            reg @ (0x0C | 0x0D) => {
                let port = (reg - 0x0C) as usize;
                // Latched mode reads the latch; unlatched, the pin.
                let level = if self.trigger_latch_enabled {
                    self.trigger_latches[port]
                } else {
                    !self.triggers[port]
                };
                (if level { 0x80 } else { 0x00 }) | (floating & 0x7F)
            }
            _ => floating,
        }
    }
}

/// The 128-colour TIA palette for a standard (colour byte bits 7-1: hue 4,
/// luma 3) — the canonical TIA colour decode, a display-side calibratable
/// stage, not a hardware claim. PAL collapses hue codes 0/1/14/15 to the
/// grey ramp (colour loss). Frame pixels index into it via [`palette_index`].
pub fn palette(standard: TvStandard) -> &'static [(u8, u8, u8); 128] {
    match standard {
        TvStandard::Ntsc => &NTSC_PALETTE,
        TvStandard::Pal => &PAL_PALETTE,
    }
}

/// TIA colour bytes drop bit 0; the palette is 7-bit indexed.
pub fn palette_index(colour_byte: u8) -> usize {
    (colour_byte >> 1) as usize
}

const NTSC_PALETTE: [(u8, u8, u8); 128] = [
    (0, 0, 0),
    (74, 74, 74),
    (111, 111, 111),
    (142, 142, 142),
    (170, 170, 170),
    (192, 192, 192),
    (214, 214, 214),
    (236, 236, 236),
    (72, 72, 0),
    (105, 105, 15),
    (134, 134, 29),
    (162, 162, 42),
    (187, 187, 53),
    (210, 210, 64),
    (232, 232, 74),
    (252, 252, 84),
    (124, 44, 0),
    (144, 72, 17),
    (162, 98, 33),
    (180, 122, 48),
    (195, 144, 61),
    (210, 164, 74),
    (223, 183, 85),
    (236, 200, 96),
    (144, 28, 0),
    (163, 57, 21),
    (181, 83, 40),
    (198, 108, 58),
    (213, 130, 74),
    (227, 151, 89),
    (240, 170, 103),
    (252, 188, 116),
    (148, 0, 0),
    (167, 26, 26),
    (184, 50, 50),
    (200, 72, 72),
    (214, 92, 92),
    (228, 111, 111),
    (240, 128, 128),
    (252, 144, 144),
    (132, 0, 100),
    (151, 25, 122),
    (168, 48, 143),
    (184, 70, 162),
    (198, 89, 179),
    (212, 108, 195),
    (224, 124, 210),
    (236, 140, 224),
    (80, 0, 132),
    (104, 25, 154),
    (125, 48, 173),
    (146, 70, 192),
    (164, 89, 208),
    (181, 108, 224),
    (197, 124, 238),
    (212, 140, 252),
    (20, 0, 144),
    (51, 26, 163),
    (78, 50, 181),
    (104, 72, 198),
    (127, 92, 213),
    (149, 111, 227),
    (169, 128, 240),
    (188, 144, 252),
    (0, 0, 148),
    (24, 26, 167),
    (45, 50, 184),
    (66, 72, 200),
    (84, 92, 214),
    (101, 111, 228),
    (117, 128, 240),
    (132, 144, 252),
    (0, 28, 136),
    (24, 59, 157),
    (45, 87, 176),
    (66, 114, 194),
    (84, 138, 210),
    (101, 160, 225),
    (117, 181, 239),
    (132, 200, 252),
    (0, 48, 100),
    (24, 80, 128),
    (45, 109, 152),
    (66, 136, 176),
    (84, 160, 197),
    (101, 183, 217),
    (117, 204, 235),
    (132, 224, 252),
    (0, 64, 48),
    (24, 98, 78),
    (45, 129, 105),
    (66, 158, 130),
    (84, 184, 153),
    (101, 209, 174),
    (117, 231, 194),
    (132, 252, 212),
    (0, 68, 0),
    (26, 102, 26),
    (50, 132, 50),
    (72, 160, 72),
    (92, 186, 92),
    (111, 210, 111),
    (128, 232, 128),
    (144, 252, 144),
    (20, 60, 0),
    (53, 95, 24),
    (82, 126, 45),
    (110, 156, 66),
    (135, 183, 84),
    (158, 208, 101),
    (180, 231, 117),
    (200, 252, 132),
    (48, 56, 0),
    (80, 89, 22),
    (109, 118, 43),
    (136, 146, 62),
    (160, 171, 79),
    (183, 194, 95),
    (204, 216, 110),
    (224, 236, 124),
    (72, 44, 0),
    (105, 77, 20),
    (134, 106, 38),
    (162, 134, 56),
    (187, 159, 71),
    (210, 182, 86),
    (232, 204, 99),
    (252, 224, 112),
];

const PAL_PALETTE: [(u8, u8, u8); 128] = [
    (0, 0, 0),
    (51, 51, 51),
    (89, 89, 89),
    (123, 123, 123),
    (153, 153, 153),
    (182, 182, 182),
    (207, 207, 207),
    (230, 230, 230),
    (11, 11, 11),
    (51, 51, 51),
    (89, 89, 89),
    (123, 123, 123),
    (153, 153, 153),
    (182, 182, 182),
    (207, 207, 207),
    (230, 230, 230),
    (59, 36, 0),
    (102, 71, 0),
    (139, 112, 0),
    (172, 146, 0),
    (197, 174, 54),
    (222, 200, 94),
    (247, 226, 127),
    (255, 241, 158),
    (0, 69, 0),
    (0, 111, 0),
    (59, 146, 0),
    (101, 176, 9),
    (133, 202, 61),
    (163, 227, 100),
    (191, 252, 132),
    (213, 255, 165),
    (89, 0, 0),
    (128, 39, 0),
    (161, 87, 0),
    (188, 121, 55),
    (214, 152, 95),
    (238, 179, 129),
    (255, 206, 158),
    (255, 220, 189),
    (0, 73, 0),
    (0, 114, 0),
    (22, 146, 22),
    (69, 175, 69),
    (107, 201, 107),
    (139, 227, 139),
    (169, 251, 169),
    (197, 255, 197),
    (100, 0, 18),
    (137, 8, 33),
    (167, 61, 77),
    (194, 100, 114),
    (220, 132, 145),
    (244, 163, 174),
    (255, 190, 202),
    (255, 218, 224),
    (0, 61, 41),
    (0, 106, 72),
    (4, 142, 99),
    (60, 170, 132),
    (98, 197, 162),
    (131, 223, 190),
    (161, 248, 217),
    (190, 255, 233),
    (85, 0, 70),
    (136, 0, 110),
    (165, 49, 141),
    (193, 89, 170),
    (218, 124, 197),
    (243, 154, 223),
    (255, 185, 243),
    (255, 212, 246),
    (0, 54, 81),
    (0, 90, 125),
    (17, 126, 156),
    (66, 156, 184),
    (104, 183, 210),
    (136, 210, 235),
    (166, 235, 255),
    (195, 255, 255),
    (76, 0, 124),
    (117, 0, 157),
    (147, 46, 184),
    (175, 87, 210),
    (202, 122, 235),
    (228, 153, 255),
    (236, 183, 255),
    (243, 212, 255),
    (0, 45, 131),
    (0, 62, 164),
    (45, 101, 191),
    (86, 133, 218),
    (121, 162, 242),
    (153, 191, 255),
    (183, 219, 255),
    (211, 245, 255),
    (34, 0, 150),
    (82, 0, 182),
    (117, 56, 207),
    (148, 95, 232),
    (177, 129, 255),
    (197, 160, 255),
    (214, 189, 255),
    (232, 218, 255),
    (0, 0, 154),
    (36, 29, 182),
    (80, 74, 208),
    (116, 111, 233),
    (146, 142, 255),
    (177, 173, 255),
    (206, 202, 255),
    (233, 229, 255),
    (11, 11, 11),
    (51, 51, 51),
    (89, 89, 89),
    (123, 123, 123),
    (153, 153, 153),
    (182, 182, 182),
    (207, 207, 207),
    (230, 230, 230),
    (11, 11, 11),
    (51, 51, 51),
    (89, 89, 89),
    (123, 123, 123),
    (153, 153, 153),
    (182, 182, 182),
    (207, 207, 207),
    (230, 230, 230),
];
