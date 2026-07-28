//! The position machinery every movable object shares. A local ÷4 two-phase
//! clock (the [`Divider`]) advances a position counter through the 40 counts of
//! a visible line. Copies come from extra START decodes on that counter (the
//! NUSIZ mechanism); a decode caught at one divider wrap is delivered to the
//! serialiser at the next — the silicon's "full cycle of the two-phase clock".
//! RESxx grounds the local divider to its pinned phase and it resumes on the
//! strobe release.

/// Position counts per line: the ÷4 divider turns 160 colour clocks into 40.
const COUNTS: u8 = 40;

/// MOTCK edges from a START delivery to the serialiser's first output — the
/// select-network tail (Sim2600 live-seam park calibration: first lit column
/// = delivery column + 2; the sample precedes its clock's edge, so one model
/// edge realises it).
pub(super) const SERIAL_TAIL: u8 = 1;

/// The main-copy START decode: the counter's wrap (count 39).
pub(super) const MAIN_DECODE: u8 = COUNTS - 1;

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
pub(super) struct PositionCounter {
    div: Divider,
    position: u8,
    start_pending: bool,
}

impl PositionCounter {
    pub(super) fn new() -> Self {
        PositionCounter {
            div: Divider::new(),
            position: 0,
            start_pending: false,
        }
    }

    /// One MOTCK edge. `decodes` are the counts that arm the next delivery;
    /// `suppress_delivery` holds the wrap body while a reset strobe grips it
    /// (the divider still advances). Returns true when a caught START delivers.
    pub(super) fn advance(&mut self, decodes: &[u8], suppress_delivery: bool) -> bool {
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
    pub(super) fn position_clk(&self) -> u8 {
        self.position * 4 + self.div.phase()
    }

    /// The ÷4 ring's sub-phase (0–3).
    pub(super) fn ring_phase(&self) -> u8 {
        self.div.phase()
    }

    /// The ÷4 position count (0..40).
    pub(super) fn count(&self) -> u8 {
        self.position
    }

    /// Whether a decode is caught in the one-wrap START-pending latch.
    pub(super) fn start_pending(&self) -> bool {
        self.start_pending
    }

    /// RESxx: plant the count at zero and ground the ring to the strobe.
    pub(super) fn plant(&mut self) {
        self.position = 0;
        self.div.ground();
    }

    /// Inject a START into the pending latch (RESBL is itself a START; a missile
    /// reset re-homes a START still in its serialiser tail).
    pub(super) fn inject_start(&mut self) {
        self.start_pending = true;
    }

    /// RESMPx re-centre: land the counter at an absolute MOTCK position.
    pub(super) fn rehome(&mut self, clk: u8) {
        self.position = clk / 4;
        self.div.rephase(clk);
    }

    pub(super) fn restore(&mut self, position: u8, ring_phase: u8, start_pending: bool) {
        self.position = position % COUNTS;
        self.div.rephase(ring_phase);
        self.start_pending = start_pending;
    }
}

/// The missile/ball serialiser: after a delivered START, a select-network
/// `lead` of dead MOTCK edges, then the object shows for `width` counts. (The
/// player's serialiser walks an 8-bit pattern instead.)
#[derive(Clone, Copy)]
pub(super) struct WidthGate {
    lead: u8,
    width_left: u8,
}

impl WidthGate {
    pub(super) fn new() -> Self {
        WidthGate {
            lead: 0,
            width_left: 0,
        }
    }

    /// One MOTCK edge: burn the select-network tail, then the width.
    pub(super) fn advance(&mut self) {
        if self.lead > 0 {
            self.lead -= 1;
        } else {
            self.width_left = self.width_left.saturating_sub(1);
        }
    }

    /// A delivered START opens the gate `width` counts after the tail.
    pub(super) fn start(&mut self, width: u8) {
        self.lead = SERIAL_TAIL;
        self.width_left = width;
    }

    /// Lit once the tail has burned and width remains.
    pub(super) fn lit(&self) -> bool {
        self.lead == 0 && self.width_left > 0
    }

    pub(super) fn leading(&self) -> bool {
        self.lead > 0
    }

    /// A reset strobe clears an unlit scan still in its tail.
    pub(super) fn kill(&mut self) {
        if self.lead > 0 {
            self.lead = 0;
            self.width_left = 0;
        }
    }

    pub(super) fn lead(&self) -> u8 {
        self.lead
    }

    pub(super) fn width_left(&self) -> u8 {
        self.width_left
    }

    pub(super) fn restore(&mut self, lead: u8, width_left: u8) {
        self.lead = lead;
        self.width_left = width_left;
    }
}

/// The eight NUSIZ player/copy modes (bits 0–2); missile width is separate.
#[derive(Clone, Copy)]
pub(super) enum NusizMode {
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
    pub(super) fn from_nusiz(nusiz: u8) -> Self {
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
pub(super) fn copy_decodes(mode: u8) -> &'static [u8] {
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
pub(super) fn player_pixel_clocks(mode: u8) -> u8 {
    match NusizMode::from_nusiz(mode) {
        NusizMode::DoubleSizePlayer => 2,
        NusizMode::QuadSizePlayer => 4,
        _ => 1,
    }
}
