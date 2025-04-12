use super::{
    Instruction,
    cpu::instructions::{Address, jump},
};
use bitflags::bitflags;

pub enum Register {
    EnabledInterrupts,
    RequestedInterrupts,
}

#[derive(Clone, Copy)]
pub enum Interrupt {
    VideoBetweenFrames,
    VideoStatus,
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
            Interrupt::VideoBetweenFrames => 0x40,
            Interrupt::VideoStatus => 0x48,
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
    pub fn new() -> Self {
        Self {
            enabled: InterruptFlags::empty(),
            requested: InterruptFlags::empty(),
        }
    }

    pub fn triggered(&self) -> Option<Interrupt> {
        Some(
            if self.enabled.contains(InterruptFlags::VIDEO_BETWEEN_FRAMES)
                && self
                    .requested
                    .contains(InterruptFlags::VIDEO_BETWEEN_FRAMES)
            {
                Interrupt::VideoBetweenFrames
            } else if self.enabled.contains(InterruptFlags::VIDEO_STATUS)
                && self.requested.contains(InterruptFlags::VIDEO_STATUS)
            {
                Interrupt::VideoStatus
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
            Interrupt::VideoBetweenFrames => self.requested.remove(InterruptFlags::VIDEO_STATUS),
            Interrupt::VideoStatus => self.requested.remove(InterruptFlags::VIDEO_BETWEEN_FRAMES),
            Interrupt::Timer => self.requested.remove(InterruptFlags::TIMER),
            Interrupt::Serial => self.requested.remove(InterruptFlags::SERIAL),
            Interrupt::Joypad => self.requested.remove(InterruptFlags::JOYPAD),
        }
    }
}

bitflags! {
    #[derive(Copy, Clone)]
    pub struct InterruptFlags: u8 {
        const JOYPAD               = 0b00010000;
        const SERIAL               = 0b00001000;
        const TIMER                = 0b00000100;
        const VIDEO_STATUS         = 0b00000010;
        const VIDEO_BETWEEN_FRAMES = 0b00000001;

        const _OTHER = !0;
    }
}
