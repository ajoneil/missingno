use nanoserde::{DeRon, SerRon};

use crate::game_boy::interrupts::Interrupt;
use registers::Control;
pub use registers::Register;

pub mod registers;

#[derive(Clone, SerRon, DeRon)]
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
            internal_counter: 0xAB00,
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

    pub fn tick(&mut self) -> Option<Interrupt> {
        self.reloading = false;

        // Handle delayed reload from previous tick's overflow
        let interrupt = if self.overflow_pending {
            self.overflow_pending = false;
            self.reloading = true;
            self.counter = self.modulo;
            Some(Interrupt::Timer)
        } else {
            None
        };

        let was_set = self.selected_bit_set();
        self.internal_counter = self.internal_counter.wrapping_add(4);
        let is_set = self.selected_bit_set();

        if was_set && !is_set {
            self.increment_tima();
        }

        interrupt
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Divider => (self.internal_counter >> 8) as u8,
            Register::Counter => self.counter,
            Register::Modulo => self.modulo,
            Register::Control => self.control.0 | 0xF8,
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) -> Option<Interrupt> {
        match register {
            Register::Divider => {
                let was_set = self.selected_bit_set();
                self.internal_counter = 0;
                if was_set {
                    self.increment_tima();
                }
            }
            Register::Counter => {
                if self.reloading {
                    // Writing to TIMA during the reload cycle is ignored (TMA wins)
                } else {
                    // Writing to TIMA during the overflow delay cancels the reload and interrupt
                    self.overflow_pending = false;
                    self.counter = value;
                }
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
        None
    }
}
