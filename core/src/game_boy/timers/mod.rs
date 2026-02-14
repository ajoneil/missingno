use crate::game_boy::interrupts::Interrupt;
use registers::Control;
pub use registers::Register;

pub mod registers;

#[derive(Clone)]
pub struct Timers {
    internal_counter: u16,
    counter: u8,
    modulo: u8,
    control: Control,
    overflow_pending: bool,
    /// Set when TIMA is in the reload cycle (TMA being loaded into TIMA).
    /// Writes to TIMA during this cycle are ignored.
    reloading: bool,
}

impl Timers {
    pub fn new() -> Self {
        Self {
            internal_counter: 0xABCC,
            counter: 0,
            modulo: 0,
            control: Control(0xf8),
            overflow_pending: false,
            reloading: false,
        }
    }

    fn selected_bit_set(&self) -> bool {
        self.control.enabled() && (self.internal_counter & self.control.selected_bit()) != 0
    }

    fn increment_tima(&mut self) {
        if self.counter == 0xFF {
            self.counter = 0;
            self.overflow_pending = true;
        } else {
            self.counter += 1;
        }
    }

    /// Advance by one T-cycle. Call this every T-cycle (4 times per M-cycle).
    /// `is_mcycle_boundary` should be true on the 4th T-cycle of each M-cycle,
    /// when overflow/reload processing should occur.
    pub fn tcycle(&mut self, is_mcycle_boundary: bool) -> Option<Interrupt> {
        let mut interrupt = None;

        // Handle delayed reload only at M-cycle boundaries
        if is_mcycle_boundary {
            self.reloading = false;
            if self.overflow_pending {
                self.overflow_pending = false;
                self.reloading = true;
                self.counter = self.modulo;
                interrupt = Some(Interrupt::Timer);
            }
        }

        let was_set = self.selected_bit_set();
        self.internal_counter = self.internal_counter.wrapping_add(1);
        let is_set = self.selected_bit_set();

        if was_set && !is_set {
            self.increment_tima();
        }

        interrupt
    }

    pub fn internal_counter(&self) -> u16 {
        self.internal_counter
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Divider => (self.internal_counter >> 8) as u8,
            Register::Counter => self.counter,
            Register::Modulo => self.modulo,
            Register::Control => self.control.0 | 0xF8,
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Divider => {
                let was_set = self.selected_bit_set();
                self.internal_counter = 0;
                if was_set {
                    self.increment_tima();
                }
            }
            Register::Counter => {
                if !self.reloading {
                    // Writing to TIMA during the overflow delay cancels the reload and interrupt
                    self.overflow_pending = false;
                    self.counter = value;
                }
                // Writing to TIMA during the reload cycle is ignored (TMA wins)
            }
            Register::Modulo => {
                self.modulo = value;
                // Writing to TMA during the reload cycle also updates TIMA
                if self.reloading {
                    self.counter = value;
                }
            }
            Register::Control => {
                let was_set = self.selected_bit_set();
                self.control = Control(value);
                let is_set = self.selected_bit_set();
                if was_set && !is_set {
                    self.increment_tima();
                }
            }
        }
    }
}
