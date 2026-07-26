//! The TIA's input ports: the two trigger pins with their VBLANK-enabled
//! latches, and the four paddle pots with their dump/charge cycle.

/// Full-scale paddle charge time; the readable range games sweep.
const POT_CHARGE_LINES: f32 = 380.0;

/// One pot pin. A paddle's potentiometer completes the RC path; with an empty
/// jack behind the pin there is no path at all, so the capacitor never charges.
/// A keypad instead holds the pin at a logic level through its own pull-up.
#[derive(Clone, Copy)]
pub(super) enum Pot {
    Disconnected,
    /// Knob position (0.0–1.0) and the RC-charge countdown in scanlines that
    /// software times.
    Knob {
        position: f32,
        countdown: u16,
    },
    /// Held by the controller rather than swept: low while it grounds the pin,
    /// otherwise pulled up.
    Driven {
        low: bool,
    },
}

impl Pot {
    pub(super) fn position(&self) -> f32 {
        match self {
            Pot::Knob { position, .. } => *position,
            _ => 0.0,
        }
    }

    pub(super) fn countdown(&self) -> u16 {
        match self {
            Pot::Knob { countdown, .. } => *countdown,
            _ => 0,
        }
    }
}

/// Scanlines of charge a knob at `position` needs before its INPT bit rises.
fn charge_lines(position: f32) -> u16 {
    (position.clamp(0.0, 1.0) * POT_CHARGE_LINES) as u16
}

pub(super) struct InputPorts {
    /// Trigger buttons, true = pressed (the pin reads low).
    pub(super) triggers: [bool; 2],
    pub(super) trigger_latch_enabled: bool,
    pub(super) trigger_latches: [bool; 2],
    pub(super) pots: [Pot; 4],
    pub(super) pot_dumped: bool,
}

impl InputPorts {
    pub(super) fn new() -> Self {
        InputPorts {
            triggers: [false; 2],
            trigger_latch_enabled: false,
            trigger_latches: [true; 2],
            pots: [Pot::Disconnected; 4],
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

    /// Point a knob; a pin with no paddle behind it has none to point.
    pub(super) fn set_paddle(&mut self, index: usize, position: f32) {
        if let Pot::Knob { position: knob, .. } = &mut self.pots[index] {
            *knob = position.clamp(0.0, 1.0);
        }
    }

    /// A paddle arrives: its capacitor starts discharged, so a charge already
    /// in progress begins again from this knob's full ramp.
    pub(super) fn connect_pot(&mut self, index: usize, position: f32) {
        let position = position.clamp(0.0, 1.0);
        self.pots[index] = Pot::Knob {
            position,
            countdown: charge_lines(position),
        };
    }

    pub(super) fn disconnect_pot(&mut self, index: usize) {
        self.pots[index] = Pot::Disconnected;
    }

    /// A controller holding the pin at a level instead of sweeping it.
    pub(super) fn drive_pot(&mut self, index: usize, low: bool) {
        self.pots[index] = Pot::Driven { low };
    }

    /// Reseat a pot's charge from a save. Port configuration is not chip state,
    /// so a pin left open by the current wiring stays open.
    pub(super) fn restore_pot(&mut self, index: usize, saved: f32, saved_countdown: u16) {
        if let Pot::Knob {
            position,
            countdown,
        } = &mut self.pots[index]
        {
            *position = saved;
            *countdown = saved_countdown;
        }
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
                if let Pot::Knob {
                    position,
                    countdown,
                } = pot
                {
                    *countdown = charge_lines(*position);
                }
            }
        }
        self.pot_dumped = dump;
    }

    /// One scanline of RC charge, unless the capacitors are held grounded.
    pub(super) fn step_pot_charge(&mut self) {
        if !self.pot_dumped {
            for pot in &mut self.pots {
                if let Pot::Knob { countdown, .. } = pot {
                    *countdown = countdown.saturating_sub(1);
                }
            }
        }
    }

    /// INPT0-3's D7: high once that pot's capacitor has charged. An open pin
    /// has no charge path, so it never rises; a driven pin follows its level,
    /// and the dump grounds either way.
    pub(super) fn pot_level(&self, index: usize) -> u8 {
        if self.pot_dumped {
            return 0x00;
        }
        match self.pots[index] {
            Pot::Knob { countdown: 0, .. } => 0x80,
            Pot::Driven { low } if !low => 0x80,
            _ => 0x00,
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
