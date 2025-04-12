use bitflags::bitflags;

use super::{
    Instruction,
    cpu::instructions::{Address, jump},
};

pub enum Register {
    EnabledInterrupts,
    RequestedInterrupts,
}

#[derive(Clone, Copy)]
pub enum Interrupt {
    VBlank,
    Lcd,
    Timer,
    Serial,
    Joypad,
}

impl Interrupt {
    pub fn call_instruction(&self) -> Instruction {
        let call = jump::Jump::Call(None, jump::Location::Address(self.address()));
        call.into()
    }

    pub fn address(&self) -> Address {
        Address::Fixed(match self {
            Interrupt::VBlank => 0x40,
            Interrupt::Lcd => 0x48,
            Interrupt::Timer => 0x50,
            Interrupt::Serial => 0x58,
            Interrupt::Joypad => 0x60,
        })
    }
}

pub struct Registers {
    pub enabled: InterruptFlags,
    pub requested: InterruptFlags,
}

impl Registers {
    pub fn triggered(&self) -> Option<Interrupt> {
        Some(
            if self.enabled.contains(InterruptFlags::VBLANK)
                && self.requested.contains(InterruptFlags::VBLANK)
            {
                Interrupt::VBlank
            } else if self.enabled.contains(InterruptFlags::LCD)
                && self.requested.contains(InterruptFlags::LCD)
            {
                Interrupt::Lcd
            } else if self.enabled.contains(InterruptFlags::TIMER)
                && self.requested.contains(InterruptFlags::TIMER)
            {
                Interrupt::Timer
            } else if self.enabled.contains(InterruptFlags::SERIAL)
                && self.requested.contains(InterruptFlags::SERIAL)
            {
                Interrupt::Serial
            } else if self.enabled.contains(InterruptFlags::JOYPAD)
                && self.requested.contains(InterruptFlags::JOYPAD)
            {
                Interrupt::Joypad
            } else {
                return None;
            },
        )
    }

    pub fn clear(&mut self, interrupt: Interrupt) {
        match interrupt {
            Interrupt::VBlank => self.requested.remove(InterruptFlags::VBLANK),
            Interrupt::Lcd => self.requested.remove(InterruptFlags::LCD),
            Interrupt::Timer => self.requested.remove(InterruptFlags::TIMER),
            Interrupt::Serial => self.requested.remove(InterruptFlags::SERIAL),
            Interrupt::Joypad => self.requested.remove(InterruptFlags::JOYPAD),
        }
    }
}

bitflags! {
    #[derive(Copy,Clone,Debug)]
    pub struct InterruptFlags: u8 {
        const JOYPAD = 0b00010000;
        const SERIAL = 0b00001000;
        const TIMER  = 0b00000100;
        const LCD    = 0b00000010;
        const VBLANK = 0b00000001;

        const _OTHER = !0;
    }
}
