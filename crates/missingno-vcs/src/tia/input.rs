//! The TIA's input ports: the two trigger pins with their VBLANK-enabled
//! latches, and the four paddle pots with their dump/charge cycle.

/// Full-scale paddle charge time; the readable range games sweep.
const POT_CHARGE_LINES: f32 = 380.0;

/// One paddle's charge state: knob position (0.0–1.0) and the RC-charge
/// countdown in scanlines that software times.
#[derive(Clone, Copy)]
pub(super) struct Pot {
    pub(super) position: f32,
    pub(super) countdown: u16,
}

pub(super) struct InputPorts {
    /// Trigger buttons, true = pressed (the pin reads low).
    pub(super) triggers: [bool; 2],
    pub(super) trigger_latch_enabled: bool,
    pub(super) trigger_latches: [bool; 2],
    /// Paddle knob positions, 0.0 (instant charge) to 1.0 (slowest).
    pub(super) pots: [Pot; 4],
    pub(super) pot_dumped: bool,
}

impl InputPorts {
    pub(super) fn new() -> Self {
        InputPorts {
            triggers: [false; 2],
            trigger_latch_enabled: false,
            trigger_latches: [true; 2],
            pots: [Pot {
                position: 0.5,
                countdown: 0,
            }; 4],
            pot_dumped: false,
        }
    }

    pub(super) fn set_trigger(&mut self, port: usize, pressed: bool) {
        self.triggers[port] = pressed;
        // The I4/I5 latches capture any low level while enabled, read or
        // no read — the feature's point for once-a-frame pollers.
        if self.trigger_latch_enabled && pressed {
            self.trigger_latches[port] = false;
        }
    }

    pub(super) fn set_paddle(&mut self, index: usize, position: f32) {
        self.pots[index].position = position.clamp(0.0, 1.0);
    }

    /// VBLANK's two input-port bits: D6 enables the trigger latches, D7
    /// grounds the pot capacitors.
    pub(super) fn write_vblank(&mut self, value: u8) {
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
        // Releasing D7 starts the RC charge, measured by software in scanlines.
        let dump = value & 0x80 != 0;
        if self.pot_dumped && !dump {
            for pot in &mut self.pots {
                pot.countdown = (pot.position.clamp(0.0, 1.0) * POT_CHARGE_LINES) as u16;
            }
        }
        self.pot_dumped = dump;
    }

    /// One scanline of RC charge, unless the capacitors are held grounded.
    pub(super) fn step_pot_charge(&mut self) {
        if !self.pot_dumped {
            for pot in &mut self.pots {
                pot.countdown = pot.countdown.saturating_sub(1);
            }
        }
    }

    /// INPT0-3's D7: high once that pot's capacitor has charged.
    pub(super) fn pot_level(&self, index: usize) -> u8 {
        if !self.pot_dumped && self.pots[index].countdown == 0 {
            0x80
        } else {
            0x00
        }
    }

    /// INPT4/5's D7 — latched mode reads the latch, unlatched the pin.
    pub(super) fn trigger_level(&self, port: usize) -> u8 {
        let high = if self.trigger_latch_enabled {
            self.trigger_latches[port]
        } else {
            !self.triggers[port]
        };
        if high { 0x80 } else { 0x00 }
    }
}
