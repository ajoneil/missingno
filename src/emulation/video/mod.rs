mod ppu;

use bitflags::bitflags;
use ppu::PixelProcessingUnit;

struct BackgroundViewportPosition {
    x: u8,
    y: u8,
}

pub enum Register {
    BackgroundViewportX,
    BackgroundViewportY,
    Status,
}

pub enum Interrupt {
    YCoordinate,
    PreparingScanline,
    BetweenFrames,
    FinishingScanline,
}

bitflags! {
    pub struct InterruptFlags: u8 {
        const DUMMY                = 0b10000000;
        const CURRENT_LINE_COMPARE = 0b01000000;
        const PREPARING_SCANLINE   = 0b00100000;
        const BETWEEN_FRAMES       = 0b00010000;
        const FINISHING_SCANLINE   = 0b00001000;
    }
}

struct Interrupts {
    flags: InterruptFlags,
    current_line_compare: u8,
}

pub struct Video {
    ppu: PixelProcessingUnit,
    background_viewport: BackgroundViewportPosition,
    interrupts: Interrupts,
}

impl Video {
    pub fn new() -> Self {
        Self {
            ppu: PixelProcessingUnit::new(),
            interrupts: Interrupts {
                // The first bit is unused, but is set at boot time
                flags: InterruptFlags::DUMMY,
                current_line_compare: 0,
            },
            background_viewport: BackgroundViewportPosition { x: 0, y: 0 },
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::BackgroundViewportX => self.background_viewport.x,
            Register::BackgroundViewportY => self.background_viewport.y,
            Register::Status => {
                let line_compare =
                    if self.interrupts.current_line_compare == self.ppu.current_line() {
                        0b00000100
                    } else {
                        0
                    };

                self.interrupts.flags.bits() & line_compare & self.ppu.mode() as u8
            }
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::BackgroundViewportX => self.background_viewport.x = value,
            Register::BackgroundViewportY => self.background_viewport.y = value,
            Register::Status => self.interrupts.flags = InterruptFlags::from_bits_truncate(value),
        }
    }
}
