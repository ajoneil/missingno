use bitflags::bitflags;
use ppu::Mode;
use screen::Screen;
use sprites::{Sprite, SpriteId};
use tile_maps::{TileMap, TileMapId};

use control::{Control, ControlFlags};
use memory::VideoMemory;
use palette::{PaletteMap, Palettes};
use ppu::PixelProcessingUnit;
use tiles::{TileBlock, TileBlockId};

pub struct VideoTickResult {
    pub screen: Option<Screen>,
    pub request_vblank: bool,
    pub request_stat: bool,
}

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
    InterruptOnScanline,
    BackgroundPalette,
    Sprite0Palette,
    Sprite1Palette,
}

struct BackgroundViewportPosition {
    x: u8,
    y: u8,
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
    stat_line_was_high: bool,
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
            stat_line_was_high: false,
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
                    (self.interrupts.flags.bits() & 0b01111000) | line_compare | ppu.mode() as u8
                } else {
                    (self.interrupts.flags.bits() & 0b01111000) | ppu::Mode::BetweenFrames as u8
                }
            }
            Register::BackgroundViewportY => self.ppu_accessible.background_viewport.y,
            Register::BackgroundViewportX => self.ppu_accessible.background_viewport.x,
            Register::WindowY => self.ppu_accessible.window.y,
            Register::WindowX => self.ppu_accessible.window.x_plus_7,
            Register::CurrentScanline => {
                if let Some(ppu) = &self.ppu {
                    ppu.current_line()
                } else {
                    0
                }
            }
            Register::InterruptOnScanline => self.interrupts.current_line_compare,
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
            Register::InterruptOnScanline => self.interrupts.current_line_compare = value,
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

    fn stat_line_active(&self) -> bool {
        let ppu = match &self.ppu {
            Some(ppu) => ppu,
            None => return false,
        };

        let mode = ppu.mode();
        let ly_eq_lyc = ppu.current_line() == self.interrupts.current_line_compare;

        (self
            .interrupts
            .flags
            .contains(InterruptFlags::FINISHING_SCANLINE)
            && mode == Mode::BetweenLines)
            || (self
                .interrupts
                .flags
                .contains(InterruptFlags::BETWEEN_FRAMES)
                && mode == Mode::BetweenFrames)
            || (self
                .interrupts
                .flags
                .contains(InterruptFlags::PREPARING_SCANLINE)
                && mode == Mode::PreparingScanline)
            || (self
                .interrupts
                .flags
                .contains(InterruptFlags::CURRENT_LINE_COMPARE)
                && ly_eq_lyc)
    }

    pub fn tick(&mut self) -> VideoTickResult {
        let mut result = VideoTickResult {
            screen: None,
            request_vblank: false,
            request_stat: false,
        };

        if self.control().video_enabled() {
            if self.ppu.is_none() {
                let ppu = PixelProcessingUnit::new();
                self.ppu = Some(ppu);
                self.stat_line_was_high = false;
            }

            if let Some(screen) = self.ppu.as_mut().unwrap().tick(&self.ppu_accessible) {
                result.screen = Some(screen);
                result.request_vblank = true;
            }
        } else {
            if self.ppu.is_some() {
                self.ppu = None;
                self.stat_line_was_high = false;
                result.screen = Some(Screen::new());
            }
            return result;
        }

        // Detect rising edge of STAT interrupt line
        let stat_line_high = self.stat_line_active();
        if stat_line_high && !self.stat_line_was_high {
            result.request_stat = true;
        }
        self.stat_line_was_high = stat_line_high;

        result
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
