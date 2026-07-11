//! The TIA's movable objects: two players, two missiles, one ball, and the
//! playfield. Each object runs on its own MOTCK grid: a local ÷4 two-phase clock
//! (the [`Divider`]) drives a position counter through the 40 counts of a visible
//! line. Copies come from extra START decodes on that counter (the NUSIZ
//! mechanism); after a decode the START passes through one full ÷4 cycle (+ a
//! player-only latch) before the graphics scan counter serialises. RESxx
//! re-phases the local divider to the strobe.

/// Visible clocks per line — the objects' colour-clock position space.
pub const COUNTER_RANGE: u8 = 160;
/// Position counts per line: the ÷4 divider turns 160 colour clocks into 40.
const COUNTS: u8 = 40;

/// A START decode passes through one full ÷4 two-phase cycle (the divider's
/// N868/N1480 phases) before it reaches the scan counter.
const DECODE_CYCLE: u8 = 4;
/// The extra MOTCK edge (N90) to latch the player START; missiles/ball omit it.
const PLAYER_START_LATCH: u8 = 1;
/// The graphics scan register's serialiser tail: the N90-clocked scan cells
/// (N2517→N410→N2267) walk to their first output this many MOTCK edges after
/// START — the missile/ball hblank landing (x=2); the player adds the latch.
const SCAN_STARTUP: u8 = 2;

/// A mid-visible reset re-phases the ÷4 divider one count later than a hblank
/// reset: N868 falls fresh on the live release edge, vs held pre-loaded through
/// the gated hblank (mid-line N1480 at x≡1 vs hblank at x≡0).
const VISIBLE_PLANT_SETTLE: u8 = 1;

/// The main-copy START decode: the counter's wrap (count 39).
const MAIN_DECODE: u8 = COUNTS - 1;

/// The local ÷4 two-phase clock (ring A→B→D→C→A, N90/MOTCK-clocked): `phase`
/// steps once per MOTCK and the position counter advances when it wraps.
#[derive(Clone, Copy)]
struct Divider {
    phase: u8,
}

impl Divider {
    fn new() -> Self {
        Divider { phase: 0 }
    }

    /// One MOTCK edge; returns true on the wrap that advances the position count.
    fn tick(&mut self) -> bool {
        self.phase = (self.phase + 1) & 3;
        self.phase == 0
    }

    /// RESxx grounds the ring: the counter re-phases to the strobe.
    fn rephase(&mut self) {
        self.phase = 0;
    }
}

/// START decode counts per NUSIZ player mode: the main copy (the wrap, count 39)
/// plus the close (count 3), medium (count 7) and far (count 15) copies the mode
/// enables — the LFSR decode counts (wired-NOR on the position counter).
fn copy_decodes(mode: u8) -> &'static [u8] {
    match mode & 0x07 {
        1 => &[MAIN_DECODE, 3],
        2 => &[MAIN_DECODE, 7],
        3 => &[MAIN_DECODE, 3, 7],
        4 => &[MAIN_DECODE, 15],
        6 => &[MAIN_DECODE, 7, 15],
        _ => &[MAIN_DECODE],
    }
}

/// Clocks per pixel for player modes (double/quad stretch).
fn player_pixel_clocks(mode: u8) -> u8 {
    match mode & 0x07 {
        5 => 2,
        7 => 4,
        _ => 1,
    }
}

struct Scan {
    bit: u8,
    clocks_left: u8,
    // The stretched serial clock divides down from the two-phase grid;
    // its first pulse lands 1 CLK after START (2x and 4x alike).
    serial_lag: u8,
}

pub struct Player {
    pub grp_new: u8,
    pub grp_old: u8,
    pub vertical_delay: bool,
    pub reflect: bool,
    pub nusiz: u8,
    div: Divider,
    position: u8,
    /// The ÷4 divider's half-CLK sub-phase: set when a reset planted the counter
    /// mid-visible (off the visible grid), cleared when it planted in hblank.
    half_offset: bool,
    start_countdown: Option<u8>,
    scan: Option<Scan>,
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Player {
            grp_new: 0,
            grp_old: 0,
            vertical_delay: false,
            reflect: false,
            nusiz: 0,
            div: Divider::new(),
            position: 0,
            half_offset: false,
            start_countdown: None,
            scan: None,
        }
    }

    /// MOTCK edges from the latched START to the first pixel: the player START
    /// latch, the scan-counter spin-up, and the divider's sub-phase settle.
    fn start_to_pixel(&self) -> u8 {
        PLAYER_START_LATCH
            + SCAN_STARTUP
            + if self.half_offset {
                VISIBLE_PLANT_SETTLE
            } else {
                0
            }
    }

    /// MOTCK edges from a fresh START decode to the first pixel: the full ÷4
    /// decode cycle ahead of the latch/scan.
    fn decode_to_pixel(&self) -> u8 {
        DECODE_CYCLE + self.start_to_pixel()
    }

    pub fn reset_position(&mut self, grid_aligned: bool) {
        self.position = 0;
        self.div.rephase();
        // RESxx re-phases the local ÷4 divider to the strobe; a mid-visible
        // strobe lands it off the visible grid.
        self.half_offset = !grid_aligned;
        // A start already decoded and in flight re-phases onto the new grid past
        // the decode cycle; with none in flight the main copy waits for the wrap.
        if self.start_countdown.is_some() {
            self.start_countdown = Some(self.start_to_pixel() - 1);
        }
    }

    /// Colour-clock position within the line (0..160) — position count × 4 plus
    /// the ÷4 divider phase.
    pub fn counter(&self) -> u8 {
        self.position * 4 + self.div.phase
    }

    /// One motion clock; returns this clock's pixel.
    pub fn tick(&mut self) -> bool {
        let pixel = self.output();
        self.advance_scan();
        if self.div.tick() {
            self.position = (self.position + 1) % COUNTS;
            if copy_decodes(self.nusiz).contains(&self.position) {
                self.start_countdown = Some(self.decode_to_pixel());
            }
        }
        if let Some(remaining) = self.start_countdown {
            if remaining == 0 {
                self.start_countdown = None;
                let clocks = player_pixel_clocks(self.nusiz);
                self.scan = Some(Scan {
                    bit: 0,
                    clocks_left: clocks,
                    serial_lag: if clocks > 1 { 1 } else { 0 },
                });
            } else {
                self.start_countdown = Some(remaining - 1);
            }
        }
        pixel
    }

    fn advance_scan(&mut self) {
        if let Some(scan) = &mut self.scan {
            if scan.serial_lag > 0 {
                scan.serial_lag -= 1;
                return;
            }
            scan.clocks_left -= 1;
            if scan.clocks_left == 0 {
                scan.bit += 1;
                if scan.bit == 8 {
                    self.scan = None;
                } else {
                    scan.clocks_left = player_pixel_clocks(self.nusiz);
                }
            }
        }
    }

    /// Combinational serialiser output for the current scan state.
    pub fn output(&self) -> bool {
        let Some(scan) = &self.scan else {
            return false;
        };
        if scan.serial_lag > 0 {
            return false;
        }
        let graphics = if self.vertical_delay {
            self.grp_old
        } else {
            self.grp_new
        };
        let bit = if self.reflect { scan.bit } else { 7 - scan.bit };
        graphics & (1 << bit) != 0
    }
}

pub struct Missile {
    pub enabled: bool,
    /// While set, the missile hides and tracks its player (RESMPx).
    pub locked_to_player: bool,
    pub nusiz: u8,
    div: Divider,
    position: u8,
    half_offset: bool,
    start_countdown: Option<u8>,
    scan_clocks_left: u8,
}

impl Default for Missile {
    fn default() -> Self {
        Self::new()
    }
}

impl Missile {
    pub fn new() -> Self {
        Missile {
            enabled: false,
            locked_to_player: false,
            nusiz: 0,
            div: Divider::new(),
            position: 0,
            half_offset: false,
            start_countdown: None,
            scan_clocks_left: 0,
        }
    }

    fn start_to_pixel(&self) -> u8 {
        SCAN_STARTUP
            + if self.half_offset {
                VISIBLE_PLANT_SETTLE
            } else {
                0
            }
    }

    fn decode_to_pixel(&self) -> u8 {
        DECODE_CYCLE + self.start_to_pixel()
    }

    /// RESMx is level-active across the strobe: the scan-counter clear
    /// leads the counter plant by one clock, killing an unlit scan.
    pub fn reset_kill(&mut self) {
        if self.scan_clocks_left > 0 && self.scan_clocks_left == self.width() {
            self.scan_clocks_left = 0;
        }
    }

    pub fn reset_position(&mut self, grid_aligned: bool) {
        self.position = 0;
        self.div.rephase();
        self.half_offset = !grid_aligned;
        // Reset re-phases an in-flight (already-decoded) start onto the new
        // grid past the decode cycle. A dot already emitting survives unmoved;
        // with no start in flight, a decode not yet fired is pre-empted.
        if self.start_countdown.is_some()
            || (self.scan_clocks_left > 0 && self.scan_clocks_left == self.width())
        {
            self.start_countdown = Some(self.start_to_pixel() - 1);
            self.scan_clocks_left = 0;
        }
    }

    /// RESMPx released: park at the re-centre landing — the missile's
    /// first pixel lands 4/6/10 clocks right of the player's per size.
    pub fn release_at(&mut self, player_counter: u8, player_mode: u8) {
        let centre: u16 = match player_mode & 0x07 {
            5 => 8,
            7 => 12,
            _ => 5,
        };
        let clk = ((u16::from(player_counter) + u16::from(COUNTER_RANGE) - centre)
            % u16::from(COUNTER_RANGE)) as u8;
        self.position = clk / 4;
        self.div.phase = clk & 3;
    }

    fn width(&self) -> u8 {
        1 << ((self.nusiz >> 4) & 0x03)
    }

    /// Combinational serialiser output for the current scan state.
    pub fn output(&self) -> bool {
        self.enabled && !self.locked_to_player && self.scan_clocks_left > 0
    }

    pub fn tick(&mut self) -> bool {
        let pixel = self.output();
        self.scan_clocks_left = self.scan_clocks_left.saturating_sub(1);
        if self.div.tick() {
            self.position = (self.position + 1) % COUNTS;
            if copy_decodes(self.nusiz).contains(&self.position) {
                self.start_countdown = Some(self.decode_to_pixel());
            }
        }
        if let Some(remaining) = self.start_countdown {
            if remaining == 0 {
                self.start_countdown = None;
                self.scan_clocks_left = self.width();
            } else {
                self.start_countdown = Some(remaining - 1);
            }
        }
        pixel
    }
}

pub struct Ball {
    pub enabled_new: bool,
    pub enabled_old: bool,
    pub vertical_delay: bool,
    /// CTRLPF width bits (4-5).
    pub width_exponent: u8,
    div: Divider,
    position: u8,
    half_offset: bool,
    start_countdown: Option<u8>,
    scan_clocks_left: u8,
}

impl Default for Ball {
    fn default() -> Self {
        Self::new()
    }
}

impl Ball {
    pub fn new() -> Self {
        Ball {
            enabled_new: false,
            enabled_old: false,
            vertical_delay: false,
            width_exponent: 0,
            div: Divider::new(),
            position: 0,
            half_offset: false,
            start_countdown: None,
            scan_clocks_left: 0,
        }
    }

    fn start_to_pixel(&self) -> u8 {
        SCAN_STARTUP
            + if self.half_offset {
                VISIBLE_PLANT_SETTLE
            } else {
                0
            }
    }

    fn decode_to_pixel(&self) -> u8 {
        DECODE_CYCLE + self.start_to_pixel()
    }

    /// RESBL is level-active across the strobe: the width-gate clear
    /// leads the counter plant by one clock, killing an unlit scan.
    pub fn reset_kill(&mut self) {
        if self.scan_clocks_left > 0 && self.scan_clocks_left == (1 << self.width_exponent) {
            self.scan_clocks_left = 0;
        }
    }

    pub fn reset_position(&mut self, grid_aligned: bool) {
        self.position = 0;
        self.div.rephase();
        self.half_offset = !grid_aligned;
        // Unlike the players and missiles, RESBL is itself a START — it draws
        // immediately, latching past the decode cycle rather than waiting a wrap.
        self.start_countdown = Some(self.start_to_pixel() - 1);
    }

    fn enabled(&self) -> bool {
        if self.vertical_delay {
            self.enabled_old
        } else {
            self.enabled_new
        }
    }

    /// Combinational serialiser output for the current scan state.
    pub fn output(&self) -> bool {
        self.enabled() && self.scan_clocks_left > 0
    }

    pub fn tick(&mut self) -> bool {
        let pixel = self.output();
        self.scan_clocks_left = self.scan_clocks_left.saturating_sub(1);
        if self.div.tick() {
            self.position = (self.position + 1) % COUNTS;
            if self.position == MAIN_DECODE {
                self.start_countdown = Some(self.decode_to_pixel());
            }
        }
        if let Some(remaining) = self.start_countdown {
            if remaining == 0 {
                self.start_countdown = None;
                self.scan_clocks_left = 1 << self.width_exponent;
            } else {
                self.start_countdown = Some(remaining - 1);
            }
        }
        pixel
    }
}

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
