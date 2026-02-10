use bitflags::bitflags;
use nanoserde::{DeRon, DeRonErr, DeRonState, SerRon, SerRonState};

use crate::game_boy::interrupts::Interrupt;

const TRANSFER_CYCLES: u16 = 512;

#[derive(Clone, SerRon, DeRon)]
pub struct Registers {
    pub data: u8,
    pub control: Control,
    cycles_remaining: u16,
    #[nserde(skip)]
    pub output: Vec<u8>,
}

impl Registers {
    pub fn new() -> Self {
        Registers {
            data: 0,
            control: Control::from_bits_retain(0x7e),
            cycles_remaining: 0,
            output: Vec::new(),
        }
    }

    pub fn start_transfer(&mut self) {
        if self
            .control
            .contains(Control::ENABLE | Control::INTERNAL_CLOCK)
        {
            self.output.push(self.data);
            self.cycles_remaining = TRANSFER_CYCLES;
        }
    }

    pub fn tick(&mut self) -> Option<Interrupt> {
        if self.cycles_remaining == 0 {
            return None;
        }

        self.cycles_remaining -= 1;
        if self.cycles_remaining == 0 {
            self.data = 0xff;
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
