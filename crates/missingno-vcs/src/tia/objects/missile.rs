//! The missile objects: a width gate opened by each START delivery, sized by
//! NUSIZ bits 4-5 and sharing the player's copy decodes. While RESMPx holds,
//! the missile hides and re-centres on its player.

use super::COUNTER_RANGE;
use super::counter::{NusizMode, PositionCounter, WidthGate, copy_decodes};

/// The one pre-edge ring phase where the MISSILE's merged stuff delivers its
/// second advance late — the ring's pulse class, the complement of the 1×
/// player's gate (console-measured: the missile's w2 dash stays full there;
/// the ball's truncates, so the ball delivers early at every class).
const MISSILE_MERGE_INERT_PHASE: u8 = 1;

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

    /// Whether a stuffed pulse merging into this MOTCK delivers its second
    /// advance ahead of the next sample (pre-edge phase, read at the merge
    /// instant).
    pub fn merge_delivery_fires(&self) -> bool {
        self.counter.ring_phase() != MISSILE_MERGE_INERT_PHASE
    }
}

/// A missile object's boundary state.
#[derive(Clone, Copy)]
pub(crate) struct MissileState {
    pub enabled: bool,
    pub locked_to_player: bool,
    pub nusiz: u8,
    pub position: u8,
    pub ring_phase: u8,
    pub start_pending: bool,
    /// Width-gate select-network lead and remaining lit width.
    pub gate_lead: u8,
    pub gate_width_left: u8,
    pub reset_decode_hold: bool,
}

impl Missile {
    pub(crate) fn capture(&self) -> MissileState {
        MissileState {
            enabled: self.enabled,
            locked_to_player: self.locked_to_player,
            nusiz: self.nusiz,
            position: self.counter.count(),
            ring_phase: self.counter.ring_phase(),
            start_pending: self.counter.start_pending(),
            gate_lead: self.gate.lead(),
            gate_width_left: self.gate.width_left(),
            reset_decode_hold: self.reset_decode_hold,
        }
    }

    pub(crate) fn restore(&mut self, s: &MissileState) {
        self.enabled = s.enabled;
        self.locked_to_player = s.locked_to_player;
        self.nusiz = s.nusiz;
        self.counter
            .restore(s.position, s.ring_phase, s.start_pending);
        self.gate.restore(s.gate_lead, s.gate_width_left);
        self.reset_decode_hold = s.reset_decode_hold;
    }
}
