//! The ball object: the missile's width gate again, sized by CTRLPF bits 4-5
//! and enabled through the VDELBL double buffer. It has no copy decodes — only
//! the main one — and RESBL is itself a START.

use super::counter::{MAIN_DECODE, PositionCounter, WidthGate};

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

/// The ball object's boundary state.
#[derive(Clone, Copy)]
pub(crate) struct BallState {
    pub enabled_new: bool,
    pub enabled_old: bool,
    pub vertical_delay: bool,
    pub width_exponent: u8,
    pub position: u8,
    pub ring_phase: u8,
    pub start_pending: bool,
    pub gate_lead: u8,
    pub gate_width_left: u8,
    /// START delivered, no pixel of this scan sampled yet.
    pub gate_start_unshown: bool,
}

impl Ball {
    pub(crate) fn capture(&self) -> BallState {
        BallState {
            enabled_new: self.enabled_new,
            enabled_old: self.enabled_old,
            vertical_delay: self.vertical_delay,
            width_exponent: self.width_exponent,
            position: self.counter.count(),
            ring_phase: self.counter.ring_phase(),
            start_pending: self.counter.start_pending(),
            gate_lead: self.gate.lead(),
            gate_width_left: self.gate.width_left(),
            gate_start_unshown: self.gate.start_unshown(),
        }
    }

    pub(crate) fn restore(&mut self, s: &BallState) {
        self.enabled_new = s.enabled_new;
        self.enabled_old = s.enabled_old;
        self.vertical_delay = s.vertical_delay;
        self.width_exponent = s.width_exponent;
        self.counter
            .restore(s.position, s.ring_phase, s.start_pending);
        self.gate
            .restore(s.gate_lead, s.gate_width_left, s.gate_start_unshown);
    }
}
