use bitflags::bitflags;

use crate::interrupts::Interrupt;

/// Bit of the internal counter whose falling edge clocks the serial shift register.
/// On DMG, this is bit 5 of the M-cycle counter, giving a base period of 256 T-cycles.
const CLOCK_BIT: u16 = 1 << 5;

/// A device connected to the Game Boy's serial port (link cable).
///
/// The Game Boy serial interface is a bidirectional shift register: on each
/// clock edge, one bit shifts out of SB (MSB first) while one bit shifts in.
/// The clock can be driven internally (Game Boy is master) or externally
/// (connected device is master).
pub trait SerialLink {
    /// Exchange one bit during a serial transfer.
    ///
    /// Called on each serial clock edge. `out_bit` is the bit the Game Boy
    /// shifts out (MSB of SB). Returns the bit to shift into SB (new LSB).
    fn exchange_bit(&mut self, out_bit: bool) -> bool;

    /// Poll for an external clock edge.
    ///
    /// Called each M-cycle when the Game Boy is in external clock mode with
    /// a transfer enabled. Return `true` to provide a clock edge this cycle,
    /// which will trigger one bit exchange.
    fn clock(&mut self) -> bool;

    /// Drain any captured output bytes.
    ///
    /// The default implementation returns an empty vec. Implementations that
    /// track outgoing data (e.g. for test result capture) should override this.
    fn drain_output(&mut self) -> Vec<u8> {
        Vec::new()
    }
}

/// No device connected. Incoming bits are high (floating line) and no
/// external clock is provided. Outgoing bytes are captured for test use.
#[derive(Default)]
pub struct Disconnected {
    current_byte: u8,
    bits_collected: u8,
    output: Vec<u8>,
}

impl Disconnected {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SerialLink for Disconnected {
    fn exchange_bit(&mut self, out_bit: bool) -> bool {
        self.current_byte = (self.current_byte << 1) | (out_bit as u8);
        self.bits_collected += 1;
        if self.bits_collected == 8 {
            self.output.push(self.current_byte);
            self.current_byte = 0;
            self.bits_collected = 0;
        }
        true // floating line reads high
    }

    fn clock(&mut self) -> bool {
        false
    }

    fn drain_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }
}

#[derive(Clone)]
pub struct Registers {
    pub data: u8,
    pub control: Control,
    /// Number of bits remaining to shift (0 = idle, 1-8 = transfer in progress).
    pub bits_remaining: u8,
    /// Internal serial clock state, toggled on each falling edge of the
    /// internal counter's clock bit. A shift occurs on each falling edge
    /// of this flag (i.e., every second falling edge of the clock bit).
    pub serial_clock: bool,
    /// Counter value at the previous M-cycle boundary, for edge detection.
    pub previous_counter: u16,
}

impl Registers {
    pub fn new() -> Self {
        Registers {
            data: 0,
            control: Control::from_bits_retain(0x7e),
            bits_remaining: 0,
            serial_clock: false,
            previous_counter: 0x2AF3,
        }
    }

    #[cfg(feature = "gbtrace")]
    pub fn from_snapshot(snap: &gbtrace::snapshot::SerialSnapshot) -> Self {
        Registers {
            data: snap.sb,
            control: Control::from_bits_retain(snap.sc),
            bits_remaining: snap.bits_remaining,
            serial_clock: snap.shift_clock,
            previous_counter: 0,
        }
    }

    /// Called when the SC register is written. Arms the shift register for
    /// a new transfer if ENABLE is set.
    pub fn start_transfer(&mut self) {
        self.bits_remaining = 0;
        self.serial_clock = false;

        if self.control.contains(Control::ENABLE) {
            self.bits_remaining = 8;
        }
    }

    /// Shift one bit through the serial port via the link device.
    fn shift_bit(&mut self, link: &mut dyn SerialLink) -> Option<Interrupt> {
        let out_bit = self.data & 0x80 != 0;
        let in_bit = link.exchange_bit(out_bit);
        self.data = (self.data << 1) | (in_bit as u8);
        self.bits_remaining -= 1;

        if self.bits_remaining == 0 {
            self.control.remove(Control::ENABLE);
            Some(Interrupt::Serial)
        } else {
            None
        }
    }

    /// Advance by one M-cycle. `counter` is the current internal 16-bit
    /// counter value (sampled after the 4th T-cycle tick of this M-cycle).
    pub fn mcycle(&mut self, counter: u16, link: &mut dyn SerialLink) -> Option<Interrupt> {
        let mut result = None;

        // External clock: let the link device drive the transfer.
        if self.bits_remaining > 0
            && self.control.contains(Control::ENABLE)
            && !self.control.contains(Control::INTERNAL_CLOCK)
            && link.clock()
        {
            result = self.shift_bit(link);
        }

        // Internal clock maintenance: the serial clock phase is free-running,
        // toggling on counter edges regardless of transfer mode or state.
        let old = self.previous_counter;
        self.previous_counter = counter;
        let fell = (old & CLOCK_BIT) != 0 && (counter & CLOCK_BIT) == 0;
        if fell {
            self.serial_clock = !self.serial_clock;

            // Shift on the falling edge of serial_clock (just became false),
            // but only in internal clock mode with an active transfer.
            if !self.serial_clock
                && self.bits_remaining > 0
                && self.control.contains(Control::INTERNAL_CLOCK)
            {
                result = self.shift_bit(link);
            }
        }

        result
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
