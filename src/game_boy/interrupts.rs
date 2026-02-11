use bitflags::bitflags;
use nanoserde::{DeRon, DeRonErr, DeRonState, SerRon, SerRonState};

#[derive(Debug)]
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

impl From<Interrupt> for InterruptFlags {
    fn from(interrupt: Interrupt) -> Self {
        match interrupt {
            Interrupt::VideoBetweenFrames => InterruptFlags::VIDEO_BETWEEN_FRAMES,
            Interrupt::VideoStatus => InterruptFlags::VIDEO_STATUS,
            Interrupt::Timer => InterruptFlags::TIMER,
            Interrupt::Serial => InterruptFlags::SERIAL,
            Interrupt::Joypad => InterruptFlags::JOYPAD,
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

impl SerRon for InterruptFlags {
    fn ser_ron(&self, _indent_level: usize, state: &mut SerRonState) {
        self.bits().ser_ron(_indent_level, state);
    }
}

impl DeRon for InterruptFlags {
    fn de_ron(state: &mut DeRonState, input: &mut std::str::Chars<'_>) -> Result<Self, DeRonErr> {
        Ok(Self::from_bits_retain(u8::de_ron(state, input)?))
    }
}

impl Interrupt {
    pub fn vector(&self) -> u16 {
        match self {
            Interrupt::VideoBetweenFrames => 0x40,
            Interrupt::VideoStatus => 0x48,
            Interrupt::Timer => 0x50,
            Interrupt::Serial => 0x58,
            Interrupt::Joypad => 0x60,
        }
    }

    pub fn priority_order() -> &'static [Self] {
        &[
            Interrupt::VideoBetweenFrames,
            Interrupt::VideoStatus,
            Interrupt::Timer,
            Interrupt::Serial,
            Interrupt::Joypad,
        ]
    }
}

#[derive(Clone, SerRon, DeRon)]
pub struct Registers {
    pub enabled: InterruptFlags,
    pub requested: InterruptFlags,
}

impl Registers {
    pub fn new() -> Self {
        Self {
            enabled: InterruptFlags::empty(),
            requested: InterruptFlags::VIDEO_BETWEEN_FRAMES,
        }
    }

    pub fn enabled(&self, interrupt: Interrupt) -> bool {
        self.enabled.contains(interrupt.into())
    }

    pub fn requested(&self, interrupt: Interrupt) -> bool {
        self.requested.contains(interrupt.into())
    }

    pub fn triggered(&self) -> Option<Interrupt> {
        for interrupt in Interrupt::priority_order() {
            if self.enabled(*interrupt) && self.requested(*interrupt) {
                return Some(*interrupt);
            }
        }

        None
    }

    pub fn request(&mut self, interrupt: Interrupt) {
        self.requested.insert(interrupt.into());
    }

    pub fn clear(&mut self, interrupt: Interrupt) {
        self.requested.remove(interrupt.into());
    }
}
