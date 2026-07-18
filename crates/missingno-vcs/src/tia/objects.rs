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

    /// The current sub-phase within the ÷4 cycle (0–3).
    fn phase(&self) -> u8 {
        self.phase
    }

    /// Re-phase the ring to an absolute MOTCK position (a RESxx landing).
    fn rephase(&mut self, clk: u8) {
        self.phase = clk & 3;
    }

    /// RESxx async-grounds the ring to its pinned state — N868 high, N1480 low,
    /// two MOTCK edges short of the wrap: the release-coincident edge drops
    /// N868 and the next edge fires N1480.
    fn ground(&mut self) {
        self.phase = 2;
    }
}

/// The position counter every movable object shares: a ÷4 divider driving a
/// 40-count position with a one-wrap START-pending latch. On each wrap it
/// retimes the just-ended span's decode into the latch and delivers the
/// previous wrap's catch.
#[derive(Clone, Copy)]
struct PositionCounter {
    div: Divider,
    position: u8,
    start_pending: bool,
}

impl PositionCounter {
    fn new() -> Self {
        PositionCounter {
            div: Divider::new(),
            position: 0,
            start_pending: false,
        }
    }

    /// One MOTCK edge. `decodes` are the counts that arm the next delivery;
    /// `suppress_delivery` holds the wrap body while a reset strobe grips it
    /// (the divider still advances). Returns true when a caught START delivers.
    fn advance(&mut self, decodes: &[u8], suppress_delivery: bool) -> bool {
        if self.div.tick() && !suppress_delivery {
            let deliver = self.start_pending;
            self.start_pending = decodes.contains(&self.position);
            self.position = (self.position + 1) % COUNTS;
            deliver
        } else {
            false
        }
    }

    /// Colour-clock position within the line (0..160): count × 4 plus the phase.
    fn position_clk(&self) -> u8 {
        self.position * 4 + self.div.phase()
    }

    /// The ÷4 ring's sub-phase (0–3).
    fn ring_phase(&self) -> u8 {
        self.div.phase()
    }

    /// RESxx: plant the count at zero and ground the ring to the strobe.
    fn plant(&mut self) {
        self.position = 0;
        self.div.ground();
    }

    /// Inject a START into the pending latch (RESBL is itself a START; a missile
    /// reset re-homes a START still in its serialiser tail).
    fn inject_start(&mut self) {
        self.start_pending = true;
    }

    /// RESMPx re-centre: land the counter at an absolute MOTCK position.
    fn rehome(&mut self, clk: u8) {
        self.position = clk / 4;
        self.div.rephase(clk);
    }
}

/// The missile/ball serialiser: after a delivered START, a select-network
/// `lead` of dead MOTCK edges, then the object shows for `width` counts. (The
/// player's serialiser walks an 8-bit pattern instead — see [`Scan`].)
#[derive(Clone, Copy)]
struct WidthGate {
    lead: u8,
    width_left: u8,
}

impl WidthGate {
    fn new() -> Self {
        WidthGate {
            lead: 0,
            width_left: 0,
        }
    }

    /// One MOTCK edge: burn the select-network tail, then the width.
    fn advance(&mut self) {
        if self.lead > 0 {
            self.lead -= 1;
        } else {
            self.width_left = self.width_left.saturating_sub(1);
        }
    }

    /// A delivered START opens the gate `width` counts after the tail.
    fn start(&mut self, width: u8) {
        self.lead = SERIAL_TAIL;
        self.width_left = width;
    }

    /// Lit once the tail has burned and width remains.
    fn lit(&self) -> bool {
        self.lead == 0 && self.width_left > 0
    }

    fn leading(&self) -> bool {
        self.lead > 0
    }

    /// A reset strobe clears an unlit scan still in its tail.
    fn kill(&mut self) {
        if self.lead > 0 {
            self.lead = 0;
            self.width_left = 0;
        }
    }
}

/// The eight NUSIZ player/copy modes (bits 0–2); missile width is separate.
#[derive(Clone, Copy)]
enum NusizMode {
    OneCopy,
    TwoCopiesClose,
    TwoCopiesMedium,
    ThreeCopiesClose,
    TwoCopiesWide,
    DoubleSizePlayer,
    ThreeCopiesMedium,
    QuadSizePlayer,
}

impl NusizMode {
    fn from_nusiz(nusiz: u8) -> Self {
        match nusiz & 0x07 {
            1 => Self::TwoCopiesClose,
            2 => Self::TwoCopiesMedium,
            3 => Self::ThreeCopiesClose,
            4 => Self::TwoCopiesWide,
            5 => Self::DoubleSizePlayer,
            6 => Self::ThreeCopiesMedium,
            7 => Self::QuadSizePlayer,
            _ => Self::OneCopy,
        }
    }
}

/// START decode counts per NUSIZ player mode: the main copy (the wrap, count 39)
/// plus the close (count 3), medium (count 7) and far (count 15) copies the mode
/// enables — the LFSR decode counts (wired-NOR on the position counter).
fn copy_decodes(mode: u8) -> &'static [u8] {
    match NusizMode::from_nusiz(mode) {
        NusizMode::TwoCopiesClose => &[MAIN_DECODE, 3],
        NusizMode::TwoCopiesMedium => &[MAIN_DECODE, 7],
        NusizMode::ThreeCopiesClose => &[MAIN_DECODE, 3, 7],
        NusizMode::TwoCopiesWide => &[MAIN_DECODE, 15],
        NusizMode::ThreeCopiesMedium => &[MAIN_DECODE, 7, 15],
        _ => &[MAIN_DECODE],
    }
}

/// Clocks per pixel for player modes (double/quad stretch).
fn player_pixel_clocks(mode: u8) -> u8 {
    match NusizMode::from_nusiz(mode) {
        NusizMode::DoubleSizePlayer => 2,
        NusizMode::QuadSizePlayer => 4,
        _ => 1,
    }
}

/// Pre-tick ring phase classes whose merged stuff previews the player
/// serialiser (console-measured: 1× collapses one row per movement cycle,
/// 2× reshapes its own single row, 4× shows nothing).
const SEAM_PREVIEW_PHASE_1X: u8 = 1;
const SEAM_PREVIEW_PHASE_STRETCHED: u8 = 3;
/// The one pre-tick ring phase where the MISSILE's merged stuff previews
/// nothing — the ring's pulse class, the complement of the 1× player's gate
/// (console-measured: the missile's w2 dash stays full there; the ball's
/// truncates, so the ball previews at every class).
const MISSILE_SEAM_INERT_PHASE: u8 = 1;

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
    /// GRP double buffer: the live write and its VDEL-delayed copy.
    pub graphics_new: u8,
    pub graphics_old: u8,
    /// VDELP: draw the delayed copy instead of the live write.
    pub vertical_delay: bool,
    /// REFP: mirror the 8-bit pattern.
    pub reflect: bool,
    pub nusiz: u8,
    counter: PositionCounter,
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
            graphics_new: 0,
            graphics_old: 0,
            vertical_delay: false,
            reflect: false,
            nusiz: 0,
            counter: PositionCounter::new(),
            scan: None,
        }
    }

    /// RESxx: plant the counter and ground the ring to the strobe. A decode
    /// already caught in the pending latch is phase-clocked state downstream
    /// of the counter — it rides through onto the re-phased grid.
    pub fn reset_position(&mut self) {
        self.counter.plant();
    }

    /// Colour-clock position within the line (0..160).
    pub fn counter(&self) -> u8 {
        self.counter.position_clk()
    }

    /// Whether a stuffed pulse merging into this MOTCK visibly previews the
    /// serialiser. Fires at one ring phase class per stretch mode — the class
    /// the scan clock derives from (console-measured stuck-train schedules;
    /// the stretched scan clock's source phase per TIA_HW_Notes). The decap
    /// sim previews at every class, refuted on silicon. Phases are pre-tick,
    /// read at the merge instant before this clock's ring advance. At the
    /// line's final stuff slot a merge catching a 1× scan still in its lead
    /// does NOT preview — no committing MOTCK edge remains, so the stretched
    /// pulse reads back the undelivered load (console-measured wrap-seam
    /// straddle; mid-line lead merges still advance, e.g. the deform drop).
    pub fn seam_preview_fires(&self, final_stuff_slot: bool) -> bool {
        let one_x = player_pixel_clocks(self.nusiz) == 1;
        if final_stuff_slot && one_x && self.scan_in_lead() {
            return false;
        }
        let class = if one_x {
            SEAM_PREVIEW_PHASE_1X
        } else {
            SEAM_PREVIEW_PHASE_STRETCHED
        };
        self.counter.ring_phase() == class
    }

    fn scan_in_lead(&self) -> bool {
        self.scan.as_ref().is_some_and(|scan| scan.lead > 0)
    }

    /// One motion clock (MOTCK edge).
    pub fn tick(&mut self) {
        self.advance_scan();
        if self.counter.advance(copy_decodes(self.nusiz), false) {
            let clocks = player_pixel_clocks(self.nusiz);
            self.scan = Some(Scan {
                lead: PLAYER_START_LATCH + SERIAL_TAIL,
                bit: 0,
                clocks_left: clocks,
                serial_lag: if clocks > 1 { 1 } else { 0 },
            });
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
            self.graphics_old
        } else {
            self.graphics_new
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
    counter: PositionCounter,
    gate: WidthGate,
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
            counter: PositionCounter::new(),
            gate: WidthGate::new(),
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
        self.gate.kill();
    }

    /// RESxx: plant the counter and ground the ring to the strobe. A START
    /// still in its serialiser tail re-phases onto the new grid — back into
    /// the pending latch for the next wrap's delivery.
    pub fn reset_position(&mut self) {
        self.counter.plant();
        self.reset_decode_hold = false;
        if self.gate.leading() {
            self.gate.kill();
            self.counter.inject_start();
        }
    }

    /// RESMPx released: park at the re-centre landing — the missile's
    /// first pixel lands 4/6/10 clocks right of the player's per size.
    pub fn release_at(&mut self, player_counter: u8, player_mode: u8) {
        let centre: u16 = match NusizMode::from_nusiz(player_mode) {
            NusizMode::DoubleSizePlayer => 8,
            NusizMode::QuadSizePlayer => 12,
            _ => 5,
        };
        let clk = ((u16::from(player_counter) + u16::from(COUNTER_RANGE) - centre)
            % u16::from(COUNTER_RANGE)) as u8;
        self.counter.rehome(clk);
    }

    fn width(&self) -> u8 {
        1 << ((self.nusiz >> 4) & 0x03)
    }

    /// Combinational serialiser output for the current scan state.
    pub fn output(&self) -> bool {
        self.enabled && !self.locked_to_player && self.gate.lit()
    }

    /// One motion clock (MOTCK edge).
    pub fn tick(&mut self) {
        self.gate.advance();
        if self
            .counter
            .advance(copy_decodes(self.nusiz), self.reset_decode_hold)
        {
            self.gate.start(self.width());
        }
    }

    /// Whether a stuffed pulse merging into this MOTCK visibly previews the
    /// width gate (pre-tick phase, read at the merge instant).
    pub fn seam_preview_fires(&self) -> bool {
        self.counter.ring_phase() != MISSILE_SEAM_INERT_PHASE
    }
}

#[derive(Clone)]
pub struct Ball {
    pub enabled_new: bool,
    pub enabled_old: bool,
    pub vertical_delay: bool,
    /// CTRLPF width bits (4-5).
    pub width_exponent: u8,
    counter: PositionCounter,
    gate: WidthGate,
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
            counter: PositionCounter::new(),
            gate: WidthGate::new(),
        }
    }

    /// RESBL is level-active across the strobe: the width-gate clear
    /// leads the counter plant by one clock, killing an unlit scan.
    pub fn reset_kill(&mut self) {
        self.gate.kill();
    }

    /// RESxx: plant the counter and ground the ring to the strobe. Unlike the
    /// players and missiles, RESBL is itself a START — injected straight into
    /// the pending latch, delivered at the first wrap rather than a decode.
    pub fn reset_position(&mut self) {
        self.counter.plant();
        self.counter.inject_start();
    }

    fn enabled(&self) -> bool {
        if self.vertical_delay {
            self.enabled_old
        } else {
            self.enabled_new
        }
    }

    /// The ENABL bit the beam draws (VDELBL-selected) — inspection only.
    pub fn effective_enabled(&self) -> bool {
        self.enabled()
    }

    /// Combinational serialiser output for the current scan state.
    pub fn output(&self) -> bool {
        self.enabled() && self.gate.lit()
    }

    /// One motion clock (MOTCK edge).
    pub fn tick(&mut self) {
        self.gate.advance();
        if self.counter.advance(&[MAIN_DECODE], false) {
            self.gate.start(1 << self.width_exponent);
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
