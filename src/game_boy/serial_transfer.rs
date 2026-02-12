use bitflags::bitflags;
use nanoserde::{DeRon, DeRonErr, DeRonState, SerRon, SerRonState};

use crate::game_boy::interrupts::Interrupt;

/// Bit of the internal counter whose falling edge clocks the serial shift register.
/// On DMG, this is bit 7 (0x80), giving a base period of 256 T-cycles.
const CLOCK_BIT: u16 = 0x80;

#[derive(Clone, SerRon, DeRon)]
pub struct Registers {
    pub data: u8,
    pub control: Control,
    /// Number of bits remaining to shift (0 = idle, 1-8 = transfer in progress).
    bits_remaining: u8,
    /// Internal serial clock state, toggled on each falling edge of the
    /// internal counter's clock bit. A shift occurs on each falling edge
    /// of this flag (i.e., every second falling edge of the clock bit).
    serial_clock: bool,
    /// Counter value at the previous M-cycle boundary, for edge detection.
    previous_counter: u16,
    #[nserde(skip)]
    pub output: Vec<u8>,
}

impl Registers {
    pub fn new() -> Self {
        Registers {
            data: 0,
            control: Control::from_bits_retain(0x7e),
            bits_remaining: 0,
            serial_clock: false,
            previous_counter: 0xABCC,
            output: Vec::new(),
        }
    }

    /// Called when the SC register is written. Resets serial state and starts
    /// a new transfer if ENABLE and INTERNAL_CLOCK are both set.
    pub fn start_transfer(&mut self) {
        // Reset bit counter and force serial_clock to false, matching hardware
        // behavior where writing SC resets the serial clock phase.
        self.bits_remaining = 0;
        self.serial_clock = false;

        if self
            .control
            .contains(Control::ENABLE | Control::INTERNAL_CLOCK)
        {
            self.output.push(self.data);
            self.bits_remaining = 8;
        }
    }

    /// Advance by one M-cycle. `counter` is the current internal 16-bit
    /// counter value (sampled after the 4th T-cycle tick of this M-cycle).
    pub fn mcycle(&mut self, counter: u16) -> Option<Interrupt> {
        let old = self.previous_counter;
        self.previous_counter = counter;
        let fell = (old & CLOCK_BIT) != 0 && (counter & CLOCK_BIT) == 0;
        if !fell {
            return None;
        }

        // Falling edge of clock bit: toggle the serial clock.
        // This runs continuously, even when no transfer is active,
        // so the phase is aligned to boot time, not transfer start.
        self.serial_clock = !self.serial_clock;

        // Shift on the falling edge of serial_clock (just became false).
        if self.serial_clock || self.bits_remaining == 0 {
            return None;
        }

        // Shift one bit out, shift 1 in (no connected device = all 1s received)
        self.data = (self.data << 1) | 1;
        self.bits_remaining -= 1;

        if self.bits_remaining == 0 {
            self.control.remove(Control::ENABLE);
            Some(Interrupt::Serial)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub enum Register {
    Data,
    Control,
}

bitflags! {
    #[derive(Copy, Clone, Debug)]
    pub struct Control: u8 {
        const ENABLE         = 0b10000000;
        const INTERNAL_CLOCK = 0b00000001;

        const _OTHER = !0;
    }
}

impl SerRon for Control {
    fn ser_ron(&self, _indent_level: usize, state: &mut SerRonState) {
        self.bits().ser_ron(_indent_level, state);
    }
}

impl DeRon for Control {
    fn de_ron(state: &mut DeRonState, input: &mut std::str::Chars<'_>) -> Result<Self, DeRonErr> {
        Ok(Self::from_bits_retain(u8::de_ron(state, input)?))
    }
}
