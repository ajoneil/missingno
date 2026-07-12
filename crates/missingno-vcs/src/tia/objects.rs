//! The TIA's movable objects: two players, two missiles, one ball, and the
//! playfield. Each object runs on its own MOTCK grid: a local ÷4 two-phase clock
//! (the [`Divider`]) advances a position counter through the 40 counts of a
//! visible line. Copies come from extra START decodes on that counter (the NUSIZ
//! mechanism); a decode caught at one divider wrap is delivered to the serialiser
//! at the next — the silicon's "full cycle of the two-phase clock". RESxx grounds
//! the local divider to its pinned phase and it resumes on the strobe release.

/// Visible clocks per line — the objects' colour-clock position space.
pub const COUNTER_RANGE: u8 = 160;
/// Position counts per line: the ÷4 divider turns 160 colour clocks into 40.
const COUNTS: u8 = 40;

/// The player START latch (decode NOR N1080 → /START N2279): one extra MOTCK
/// edge on the serialiser tail; missiles and the ball have no such stage.
const PLAYER_START_LATCH: u8 = 1;
/// MOTCK edges from a START delivery to the serialiser's first output — the
/// measured select-network tail (the scan register begins its walk on the
/// delivery edge; the first lit pixel follows two edges later).
const SERIAL_TAIL: u8 = 2;

/// The main-copy START decode: the counter's wrap (count 39).
const MAIN_DECODE: u8 = COUNTS - 1;

/// The local ÷4 two-phase clock (ring A→B→D→C→A, N90/MOTCK-clocked): `phase`
/// steps once per MOTCK and the position counter advances on the wrap — the
/// N1480 slave-transfer phase.
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

    /// RESxx async-grounds the ring to its pinned state — N868 high, N1480 low,
    /// two MOTCK edges short of the wrap: the release-coincident edge drops
    /// N868 and the next edge fires N1480.
    fn ground(&mut self) {
        self.phase = 2;
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

#[derive(Clone)]
struct Scan {
    /// MOTCK edges until the serialiser presents bit 0: the player START
    /// latch plus the select-network tail.
    lead: u8,
    bit: u8,
    clocks_left: u8,
    // The stretched serial clock divides down from the two-phase grid;
    // its first pulse lands 1 CLK after START (2x and 4x alike).
    serial_lag: u8,
}

#[derive(Clone)]
pub struct Player {
    pub grp_new: u8,
    pub grp_old: u8,
    pub vertical_delay: bool,
    pub reflect: bool,
    pub nusiz: u8,
    div: Divider,
    position: u8,
    /// A START decode caught at the divider wrap, riding the ÷4 cycle to the
    /// next wrap's delivery.
    start_pending: bool,
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
            start_pending: false,
            scan: None,
        }
    }

    /// RESxx: plant the counter and ground the ring to the strobe. A decode
    /// already caught in the pending latch is phase-clocked state downstream
    /// of the counter — it rides through onto the re-phased grid.
    pub fn reset_position(&mut self) {
        self.position = 0;
        self.div.ground();
    }

    /// Colour-clock position within the line (0..160) — position count × 4 plus
    /// the ÷4 divider phase.
    pub fn counter(&self) -> u8 {
        self.position * 4 + self.div.phase
    }

    /// One motion clock (MOTCK edge).
    pub fn tick(&mut self) {
        self.advance_scan();
        if self.div.tick() {
            // The wrap retimes the decode of the count-span just ended into
            // the pending latch, and delivers the previous wrap's catch.
            let deliver = self.start_pending;
            self.start_pending = copy_decodes(self.nusiz).contains(&self.position);
            self.position = (self.position + 1) % COUNTS;
            if deliver {
                let clocks = player_pixel_clocks(self.nusiz);
                self.scan = Some(Scan {
                    lead: PLAYER_START_LATCH + SERIAL_TAIL,
                    bit: 0,
                    clocks_left: clocks,
                    serial_lag: if clocks > 1 { 1 } else { 0 },
                });
            }
        }
    }

    fn advance_scan(&mut self) {
        if let Some(scan) = &mut self.scan {
            if scan.lead > 0 {
                scan.lead -= 1;
                return;
            }
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
        if scan.lead > 0 || scan.serial_lag > 0 {
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

#[derive(Clone)]
pub struct Missile {
    pub enabled: bool,
    /// While set, the missile hides and tracks its player (RESMPx).
    pub locked_to_player: bool,
    pub nusiz: u8,
    div: Divider,
    position: u8,
    start_pending: bool,
    /// MOTCK edges until the width gate opens: the select-network tail.
    lead: u8,
    scan_clocks_left: u8,
    /// The reset strobe's decoded level holds the wrap decode (no catch, no
    /// delivery) from its rise until the counter plant re-phases the ring.
    reset_decode_hold: bool,
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
            start_pending: false,
            lead: 0,
            scan_clocks_left: 0,
            reset_decode_hold: false,
        }
    }

    /// RESMx's address-decoded rise: the strobe level disturbs the START
    /// decode a clock before the plant (die window: re-home dot for
    /// alignments −2..+1, the m10_rrace runs).
    pub fn reset_rise(&mut self) {
        self.reset_decode_hold = true;
    }

    /// RESMx is level-active across the strobe: the scan-counter clear
    /// leads the counter plant by one clock, killing an unlit scan.
    pub fn reset_kill(&mut self) {
        if self.lead > 0 {
            self.lead = 0;
            self.scan_clocks_left = 0;
        }
    }

    /// RESxx: plant the counter and ground the ring to the strobe. A START
    /// still in its serialiser tail re-phases onto the new grid — back into
    /// the pending latch for the next wrap's delivery.
    pub fn reset_position(&mut self) {
        self.position = 0;
        self.div.ground();
        self.reset_decode_hold = false;
        if self.lead > 0 {
            self.lead = 0;
            self.scan_clocks_left = 0;
            self.start_pending = true;
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
        self.enabled && !self.locked_to_player && self.lead == 0 && self.scan_clocks_left > 0
    }

    /// One motion clock (MOTCK edge).
    pub fn tick(&mut self) {
        if self.lead > 0 {
            self.lead -= 1;
        } else {
            self.scan_clocks_left = self.scan_clocks_left.saturating_sub(1);
        }
        if self.div.tick() && !self.reset_decode_hold {
            let deliver = self.start_pending;
            self.start_pending = copy_decodes(self.nusiz).contains(&self.position);
            self.position = (self.position + 1) % COUNTS;
            if deliver {
                self.lead = SERIAL_TAIL;
                self.scan_clocks_left = self.width();
            }
        }
    }
}

#[derive(Clone)]
pub struct Ball {
    pub enabled_new: bool,
    pub enabled_old: bool,
    pub vertical_delay: bool,
    /// CTRLPF width bits (4-5).
    pub width_exponent: u8,
    div: Divider,
    position: u8,
    start_pending: bool,
    /// MOTCK edges until the width gate opens: the select-network tail.
    lead: u8,
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
            start_pending: false,
            lead: 0,
            scan_clocks_left: 0,
        }
    }

    /// RESBL is level-active across the strobe: the width-gate clear
    /// leads the counter plant by one clock, killing an unlit scan.
    pub fn reset_kill(&mut self) {
        if self.lead > 0 {
            self.lead = 0;
            self.scan_clocks_left = 0;
        }
    }

    /// RESxx: plant the counter and ground the ring to the strobe. Unlike the
    /// players and missiles, RESBL is itself a START — injected straight into
    /// the pending latch, delivered at the first wrap rather than a decode.
    pub fn reset_position(&mut self) {
        self.position = 0;
        self.div.ground();
        self.start_pending = true;
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
        self.enabled() && self.lead == 0 && self.scan_clocks_left > 0
    }

    /// One motion clock (MOTCK edge).
    pub fn tick(&mut self) {
        if self.lead > 0 {
            self.lead -= 1;
        } else {
            self.scan_clocks_left = self.scan_clocks_left.saturating_sub(1);
        }
        if self.div.tick() {
            let deliver = self.start_pending;
            self.start_pending = self.position == MAIN_DECODE;
            self.position = (self.position + 1) % COUNTS;
            if deliver {
                self.lead = SERIAL_TAIL;
                self.scan_clocks_left = 1 << self.width_exponent;
            }
        }
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
