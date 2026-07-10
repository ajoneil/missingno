//! The TIA's movable objects: two players, two missiles, one ball, and
//! the playfield. Position is a free-running counter per object, wrapped
//! at the 160 visible clocks; copies come from extra start decodes on the
//! same counter (the NUSIZ mechanism), and drawing begins a fixed pipeline
//! delay after a decode fires.

/// Visible clocks per line — the objects' position space.
pub const COUNTER_RANGE: u8 = 160;

/// Start decodes per NUSIZ player mode: main copy plus the close (+16),
/// medium (+32) and far (+64) copy decodes the mode enables.
fn copy_decodes(mode: u8) -> &'static [u8] {
    match mode & 0x07 {
        1 => &[0, 16],
        2 => &[0, 32],
        3 => &[0, 16, 32],
        4 => &[0, 64],
        6 => &[0, 32, 64],
        _ => &[0],
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
    counter: u8,
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
            counter: 0,
            start_countdown: None,
            scan: None,
        }
    }

    pub fn reset_position(&mut self) {
        self.counter = 0;
        if self.start_countdown.is_some() {
            // A start decode in flight re-phases onto the new counter
            // grid; its first pipeline stage clocks on the decode tick.
            self.start_countdown = Some(START_DELAY_PLAYER - 1);
        } else {
            // No start in flight: the main copy waits for the wrap.
            self.start_countdown = None;
        }
    }

    pub fn counter(&self) -> u8 {
        self.counter
    }

    /// One motion clock; returns this clock's pixel.
    pub fn tick(&mut self) -> bool {
        let pixel = self.output();
        self.advance_scan();
        self.counter = (self.counter + 1) % COUNTER_RANGE;
        if copy_decodes(self.nusiz).contains(&self.counter) {
            self.start_countdown = Some(START_DELAY_PLAYER);
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

    fn output(&self) -> bool {
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

// Pipeline clocks between a start decode and the first pixel, pinned by
// the hblank-reset landing positions (player x=3, missile/ball x=2).
const START_DELAY_PLAYER: u8 = 3;
const START_DELAY_MISSILE: u8 = 2;
const START_DELAY_BALL: u8 = 2;

pub struct Missile {
    pub enabled: bool,
    /// While set, the missile hides and tracks its player (RESMPx).
    pub locked_to_player: bool,
    pub nusiz: u8,
    counter: u8,
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
            counter: 0,
            start_countdown: None,
            scan_clocks_left: 0,
        }
    }

    /// RESMx is level-active across the strobe: the scan-counter clear
    /// leads the counter plant by one clock, killing an unlit scan.
    pub fn reset_kill(&mut self) {
        if self.scan_clocks_left > 0 && self.scan_clocks_left == self.width() {
            self.scan_clocks_left = 0;
        }
    }

    pub fn reset_position(&mut self) {
        self.counter = 0;
        if self.start_countdown.is_some()
            || (self.scan_clocks_left > 0 && self.scan_clocks_left == self.width())
        {
            // Reset re-phases an in-flight start onto the new counter
            // grid; like the wrap decode, its first stage clocks on the
            // decode tick. A dot already emitting survives unmoved.
            self.start_countdown = Some(START_DELAY_MISSILE - 1);
            self.scan_clocks_left = 0;
        } else {
            // No start in flight: a decode not yet fired is pre-empted.
            self.start_countdown = None;
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
        self.counter = ((u16::from(player_counter) + u16::from(COUNTER_RANGE) - centre)
            % u16::from(COUNTER_RANGE)) as u8;
    }

    fn width(&self) -> u8 {
        1 << ((self.nusiz >> 4) & 0x03)
    }

    pub fn tick(&mut self) -> bool {
        let pixel = self.enabled && !self.locked_to_player && self.scan_clocks_left > 0;
        self.scan_clocks_left = self.scan_clocks_left.saturating_sub(1);
        self.counter = (self.counter + 1) % COUNTER_RANGE;
        if copy_decodes(self.nusiz).contains(&self.counter) {
            self.start_countdown = Some(START_DELAY_MISSILE);
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
    counter: u8,
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
            counter: 0,
            start_countdown: None,
            scan_clocks_left: 0,
        }
    }

    pub fn reset_position(&mut self) {
        self.counter = 0;
        // Unlike the players and missiles, the ball's reset decode is
        // also a start decode; like the wrap decode, its first pipeline
        // stage clocks on the decode tick.
        self.start_countdown = Some(START_DELAY_BALL - 1);
    }

    fn enabled(&self) -> bool {
        if self.vertical_delay {
            self.enabled_old
        } else {
            self.enabled_new
        }
    }

    pub fn tick(&mut self) -> bool {
        let pixel = self.enabled() && self.scan_clocks_left > 0;
        self.scan_clocks_left = self.scan_clocks_left.saturating_sub(1);
        self.counter = (self.counter + 1) % COUNTER_RANGE;
        if self.counter == 0 {
            self.start_countdown = Some(START_DELAY_BALL);
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
