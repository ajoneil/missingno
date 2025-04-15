pub mod control;
pub mod memory;
pub mod palette;
pub mod ppu;
pub mod sprites;
pub mod tile_maps;
pub mod tiles;

use bitflags::bitflags;
use control::{Control, ControlFlags};
use memory::VideoMemory;
use palette::{PaletteMap, Palettes};
use ppu::PixelProcessingUnit;
use tiles::{TileBlock, TileBlockId};

use super::cpu::cycles::Cycles;

#[derive(Debug)]
pub enum Register {
    Control,
    Status,
    BackgroundViewportY,
    BackgroundViewportX,
    WindowY,
    WindowX,
    CurrentScanline,
    BackgroundPalette,
    Sprite0Palette,
    Sprite1Palette,
}

struct BackgroundViewportPosition {
    x: u8,
    y: u8,
}

// pub enum Interrupt {
//     YCoordinate,
//     PreparingScanline,
//     BetweenFrames,
//     FinishingScanline,
// }

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
    control: Control,
    ppu: PixelProcessingUnit,
    interrupts: Interrupts,
    background_viewport: BackgroundViewportPosition,
    window: Window,
    palettes: Palettes,
    memory: VideoMemory,
}

pub struct Window {
    y: u8,
    x_plus_7: u8,
}

impl Video {
    pub fn new() -> Self {
        Self {
            control: Control::default(),
            ppu: PixelProcessingUnit::new(),
            interrupts: Interrupts {
                // The first bit is unused, but is set at boot time
                flags: InterruptFlags::DUMMY,
                current_line_compare: 0,
            },
            background_viewport: BackgroundViewportPosition { x: 0, y: 0 },
            window: Window { y: 0, x_plus_7: 0 },
            palettes: Palettes::default(),
            memory: VideoMemory::new(),
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.control.bits(),
            Register::Status => {
                let line_compare =
                    if self.interrupts.current_line_compare == self.ppu.current_line() {
                        0b00000100
                    } else {
                        0
                    };

                self.interrupts.flags.bits() & line_compare & self.ppu.mode() as u8
            }
            Register::BackgroundViewportY => self.background_viewport.y,
            Register::BackgroundViewportX => self.background_viewport.x,
            Register::WindowY => self.window.y,
            Register::WindowX => self.window.x_plus_7 - 7,
            Register::CurrentScanline => self.ppu.current_line(),
            Register::BackgroundPalette => self.palettes.background.0,
            Register::Sprite0Palette => self.palettes.sprite0.0,
            Register::Sprite1Palette => self.palettes.sprite1.0,
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Control => self.control = Control::new(ControlFlags::from_bits_retain(value)),
            Register::Status => self.interrupts.flags = InterruptFlags::from_bits_truncate(value),
            Register::BackgroundViewportY => self.background_viewport.y = value,
            Register::BackgroundViewportX => self.background_viewport.x = value,
            Register::WindowY => self.window.y = value,
            Register::WindowX => self.window.x_plus_7 = value,
            Register::BackgroundPalette => self.palettes.background = PaletteMap(value),
            Register::Sprite0Palette => self.palettes.sprite0 = PaletteMap(value),
            Register::Sprite1Palette => self.palettes.sprite1 = PaletteMap(value),
            Register::CurrentScanline => unreachable!(),
        }
    }

    pub fn read_memory(&self, address: memory::MappedAddress) -> u8 {
        self.memory.read(address)
    }

    pub fn write_memory(&mut self, address: memory::MappedAddress, value: u8) {
        self.memory.write(address, value);
    }

    pub fn mode(&self) -> ppu::Mode {
        self.ppu.mode()
    }

    pub fn control(&self) -> Control {
        self.control
    }

    pub fn step(&mut self, cycles: Cycles) -> Option<super::interrupts::Interrupt> {
        self.ppu.step(cycles)
    }

    pub fn palettes(&self) -> &Palettes {
        &self.palettes
    }

    pub fn tile_block(&self, block: TileBlockId) -> &TileBlock {
        self.memory.tile_block(block)
    }
}
