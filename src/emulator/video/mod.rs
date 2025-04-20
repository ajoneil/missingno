use bitflags::bitflags;
use screen::Screen;
use sprites::{Sprite, SpriteId};
use tile_maps::{TileMap, TileMapId};

use control::{Control, ControlFlags};
use memory::VideoMemory;
use palette::{PaletteMap, Palettes};
use ppu::PixelProcessingUnit;
use tiles::{TileBlock, TileBlockId};

pub mod control;
pub mod memory;
pub mod palette;
pub mod ppu;
pub mod screen;
pub mod sprites;
pub mod tile_maps;
pub mod tiles;

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

pub struct PpuAccessible {
    control: Control,
    background_viewport: BackgroundViewportPosition,
    window: Window,
    palettes: Palettes,
    memory: VideoMemory,
}

pub struct Video {
    ppu: Option<PixelProcessingUnit>,
    ppu_accessible: PpuAccessible,
    interrupts: Interrupts,
}

pub struct Window {
    y: u8,
    x_plus_7: u8,
}

impl Video {
    pub fn new() -> Self {
        Self {
            ppu_accessible: PpuAccessible {
                control: Control::default(),
                background_viewport: BackgroundViewportPosition { x: 0, y: 0 },
                window: Window { y: 0, x_plus_7: 0 },
                palettes: Palettes::default(),
                memory: VideoMemory::new(),
            },

            ppu: Some(PixelProcessingUnit::new()),
            interrupts: Interrupts {
                // The first bit is unused, but is set at boot time
                flags: InterruptFlags::DUMMY,
                current_line_compare: 0,
            },
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.ppu_accessible.control.bits(),
            Register::Status => {
                if let Some(ppu) = &self.ppu {
                    let line_compare = if self.interrupts.current_line_compare == ppu.current_line()
                    {
                        0b00000100
                    } else {
                        0
                    };
                    self.interrupts.flags.bits() & line_compare & ppu.mode() as u8
                } else {
                    self.interrupts.flags.bits() & ppu::Mode::BetweenFrames as u8
                }
            }
            Register::BackgroundViewportY => self.ppu_accessible.background_viewport.y,
            Register::BackgroundViewportX => self.ppu_accessible.background_viewport.x,
            Register::WindowY => self.ppu_accessible.window.y,
            Register::WindowX => self.ppu_accessible.window.x_plus_7 - 7,
            Register::CurrentScanline => {
                if let Some(ppu) = &self.ppu {
                    ppu.current_line()
                } else {
                    0
                }
            }
            Register::BackgroundPalette => self.ppu_accessible.palettes.background.0,
            Register::Sprite0Palette => self.ppu_accessible.palettes.sprite0.0,
            Register::Sprite1Palette => self.ppu_accessible.palettes.sprite1.0,
        }
    }

    pub fn write_register(&mut self, register: Register, value: u8) {
        match register {
            Register::Control => {
                self.ppu_accessible.control = Control::new(ControlFlags::from_bits_retain(value))
            }
            Register::Status => self.interrupts.flags = InterruptFlags::from_bits_truncate(value),
            Register::BackgroundViewportY => self.ppu_accessible.background_viewport.y = value,
            Register::BackgroundViewportX => self.ppu_accessible.background_viewport.x = value,
            Register::WindowY => self.ppu_accessible.window.y = value,
            Register::WindowX => self.ppu_accessible.window.x_plus_7 = value,
            Register::BackgroundPalette => {
                self.ppu_accessible.palettes.background = PaletteMap(value)
            }
            Register::Sprite0Palette => self.ppu_accessible.palettes.sprite0 = PaletteMap(value),
            Register::Sprite1Palette => self.ppu_accessible.palettes.sprite1 = PaletteMap(value),
            Register::CurrentScanline => unreachable!(),
        }
    }

    pub fn read_memory(&self, address: memory::MappedAddress) -> u8 {
        self.ppu_accessible.memory.read(address)
    }

    pub fn write_memory(&mut self, address: memory::MappedAddress, value: u8) {
        self.ppu_accessible.memory.write(address, value);
    }

    pub fn mode(&self) -> ppu::Mode {
        if let Some(ppu) = &self.ppu {
            ppu.mode()
        } else {
            ppu::Mode::BetweenFrames
        }
    }

    pub fn control(&self) -> Control {
        self.ppu_accessible.control
    }

    pub fn tick(&mut self) -> Option<Screen> {
        if self.control().video_enabled() {
            if self.ppu.is_none() {
                let ppu = PixelProcessingUnit::new();
                self.ppu = Some(ppu);
            }

            self.ppu.as_mut().unwrap().tick(&self.ppu_accessible)
        } else {
            if self.ppu.is_some() {
                self.ppu = None;
                Some(Screen::new())
            } else {
                None
            }
        }
    }

    pub fn palettes(&self) -> &Palettes {
        &self.ppu_accessible.palettes
    }

    pub fn tile_block(&self, block: TileBlockId) -> &TileBlock {
        self.ppu_accessible.memory.tile_block(block)
    }

    pub fn tile_map(&self, map: TileMapId) -> &TileMap {
        self.ppu_accessible.memory.tile_map(map)
    }

    pub fn sprite(&self, sprite: SpriteId) -> &Sprite {
        self.ppu_accessible.memory.sprite(sprite)
    }
}
