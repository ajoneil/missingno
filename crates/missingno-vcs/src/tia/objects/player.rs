//! The player objects: an 8-bit graphics pattern serialised out of each START
//! delivery, in NUSIZ copies and 1×/2×/4× stretch, with the GRP double buffer
//! selected by VDELP and the pattern optionally mirrored by REFP.

use super::counter::{PositionCounter, SERIAL_TAIL, copy_decodes, player_pixel_clocks};

/// The player START latch (decode NOR N1080 → /START N2279): one extra MOTCK
/// edge on the serialiser tail; missiles and the ball have no such stage.
const PLAYER_START_LATCH: u8 = 1;

/// Pre-edge ring phase classes whose merged stuff delivers its second advance
/// ahead of the sample (console-measured: 1× collapses one row per movement
/// cycle, 2× reshapes its own single row, 4× shows nothing).
const MERGE_DELIVERY_PHASE_1X: u8 = 1;
const MERGE_DELIVERY_PHASE_STRETCHED: u8 = 3;

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
    /// The reset strobe's decoded level holds the wrap decode (no catch, no
    /// delivery) from its rise until the counter plant re-phases the ring
    /// (PAL console merge-delivery leg 3: a level spanning the delivery wrap
    /// kills that line's scan).
    reset_decode_hold: bool,
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
            reset_decode_hold: false,
        }
    }

    /// RESPx's address-decoded rise: the strobe level disturbs the START
    /// decode ahead of the plant.
    pub fn reset_rise(&mut self) {
        self.reset_decode_hold = true;
    }

    /// RESxx: plant the counter and ground the ring to the strobe. A decode
    /// already caught in the pending latch is phase-clocked state downstream
    /// of the counter — it rides through onto the re-phased grid.
    pub fn reset_position(&mut self) {
        self.counter.plant();
        self.reset_decode_hold = false;
    }

    /// Colour-clock position within the line (0..160).
    pub fn counter(&self) -> u8 {
        self.counter.position_clk()
    }

    /// Whether a stuffed pulse merging into this MOTCK delivers its second
    /// advance ahead of the next sample. At 1×, a scan that has not yet
    /// presented bit 0 takes the transfer at any ring phase, then the class
    /// gate (PAL console, merge-delivery latch legs). Stretched modes: the
    /// lead and serial-lag stages take it at any phase; at 2×, through the
    /// scan's first three bits a cell's first clock takes it only on the
    /// odd-index cell — the ÷2 stretch stage gating the transfer (PAL
    /// console: six latch ROM profiles and the priority-extinguish leg,
    /// video and latch agreeing on both surfaces); otherwise the class gate
    /// (stretched source phase per TIA_HW_Notes). The three-bit scope is
    /// console-measured with its mechanism open; at 4× every candidate
    /// clause is observation-equivalent, so the class gate alone stands.
    /// The decap sim fires at every class, refuted on silicon. State is
    /// pre-edge, read at the merge instant.
    pub fn merge_delivery_fires(&self) -> bool {
        let pixel_clocks = player_pixel_clocks(self.nusiz);
        let class = if pixel_clocks == 1 {
            MERGE_DELIVERY_PHASE_1X
        } else {
            MERGE_DELIVERY_PHASE_STRETCHED
        };
        let at_class = self.counter.ring_phase() == class;
        let Some(scan) = &self.scan else {
            return at_class;
        };
        if pixel_clocks == 1 {
            return scan.lead > 0 || scan.bit == 0 || at_class;
        }
        if scan.lead > 0 || scan.serial_lag > 0 {
            return true;
        }
        // The odd-cell grant is the ÷2 stretch stage's; it has no 4×
        // counterpart (single mask leg, ph1 dead — netlist-traced), and
        // every 4× candidate clause is observation-equivalent there.
        if pixel_clocks == 2 && scan.bit < 3 && scan.clocks_left == pixel_clocks {
            return scan.bit % 2 == 1;
        }
        at_class
    }

    /// Whether the merge's second transfer would carry the serialiser off
    /// bit 0 before bit 0 was sampled — the transfer is then consumed by
    /// the latch chain: no advance, no subsume (PAL captures: the lead-1
    /// hold, line-end and mid-line alike).
    pub fn merge_second_transfer_blocked(&self) -> bool {
        self.scan.as_ref().is_some_and(|scan| {
            scan.lead == 1 && scan.serial_lag == 0 && scan.clocks_left == 1 && scan.bit == 0
        })
    }

    /// One motion clock (MOTCK edge).
    pub fn tick(&mut self) {
        self.advance_scan();
        if self
            .counter
            .advance(copy_decodes(self.nusiz), self.reset_decode_hold)
        {
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

/// A player's serialiser scan: MOTCK lead, the walked bit, the per-bit clock
/// remainder, and the stretched-clock lag.
#[derive(Clone, Copy)]
pub(crate) struct ScanState {
    pub lead: u8,
    pub bit: u8,
    pub clocks_left: u8,
    pub serial_lag: u8,
}

/// A player object's boundary state.
#[derive(Clone, Copy)]
pub(crate) struct PlayerState {
    pub graphics_new: u8,
    pub graphics_old: u8,
    pub vertical_delay: bool,
    pub reflect: bool,
    pub nusiz: u8,
    /// ÷4 position count (0..40).
    pub position: u8,
    /// ÷4 ring sub-phase (0..3).
    pub ring_phase: u8,
    /// The one-wrap START-pending latch.
    pub start_pending: bool,
    pub reset_decode_hold: bool,
    pub scan: Option<ScanState>,
}

impl Player {
    pub(crate) fn capture(&self) -> PlayerState {
        PlayerState {
            graphics_new: self.graphics_new,
            graphics_old: self.graphics_old,
            vertical_delay: self.vertical_delay,
            reflect: self.reflect,
            nusiz: self.nusiz,
            position: self.counter.count(),
            ring_phase: self.counter.ring_phase(),
            start_pending: self.counter.start_pending(),
            reset_decode_hold: self.reset_decode_hold,
            scan: self.scan.as_ref().map(|s| ScanState {
                lead: s.lead,
                bit: s.bit,
                clocks_left: s.clocks_left,
                serial_lag: s.serial_lag,
            }),
        }
    }

    pub(crate) fn restore(&mut self, s: &PlayerState) {
        self.graphics_new = s.graphics_new;
        self.graphics_old = s.graphics_old;
        self.vertical_delay = s.vertical_delay;
        self.reflect = s.reflect;
        self.nusiz = s.nusiz;
        self.counter
            .restore(s.position, s.ring_phase, s.start_pending);
        self.reset_decode_hold = s.reset_decode_hold;
        self.scan = s.scan.map(|s| Scan {
            lead: s.lead,
            bit: s.bit,
            clocks_left: s.clocks_left,
            serial_lag: s.serial_lag,
        });
    }
}
