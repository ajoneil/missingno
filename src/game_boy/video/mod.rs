use super::save_state::Base64Bytes;
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
    /// Cached LY=LYC comparison result, updated each M-cycle while the
    /// PPU is on. Frozen when the PPU is off (comparison clock stops).
    ly_eq_lyc: bool,
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
            ly_eq_lyc: true,
            stat_line_was_high: false,
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.ppu_accessible.control.bits(),
            Register::Status => {
                let mode = if let Some(ppu) = &self.ppu {
                    ppu.stat_mode() as u8
                } else {
                    0
                };
                let line_compare = if self.ly_eq_lyc { 0b00000100 } else { 0 };
                0x80 | (self.interrupts.flags.bits() & 0b01111000) | line_compare | mode
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
            Register::CurrentScanline => {} // writes to LY are ignored on DMG
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

    /// Trigger OAM bug write corruption if the PPU is in Mode 2.
    ///
    /// Called when INC/DEC rr, PUSH, CALL, JR, RST, or interrupt dispatch
    /// place an OAM-range address on the bus, or when the CPU writes to
    /// OAM during Mode 2.
    pub fn oam_bug_write(&mut self) {
        let row_offset = match self.accessed_oam_row() {
            Some(offset) if offset >= 8 && offset < 160 => offset,
            _ => return,
        };

        let mem = &mut self.ppu_accessible.memory;
        let a = mem.oam_word(row_offset);
        let b = mem.oam_word(row_offset - 8);
        let c = mem.oam_word(row_offset - 4);

        let glitched = ((a ^ c) & (b ^ c)) ^ c;
        mem.set_oam_word(row_offset, glitched);

        for i in 2..8u8 {
            let val = mem.oam_byte(row_offset - 8 + i);
            mem.set_oam_byte(row_offset + i, val);
        }
    }

    /// Trigger OAM bug read corruption if the PPU is in Mode 2.
    ///
    /// Called when the CPU reads from OAM during Mode 2. Uses a different
    /// bitwise formula and copies all 8 bytes from the previous row.
    pub fn oam_bug_read(&mut self) {
        let row_offset = match self.accessed_oam_row() {
            Some(offset) if offset >= 8 && offset < 160 => offset,
            _ => return,
        };

        let mem = &mut self.ppu_accessible.memory;
        let a = mem.oam_word(row_offset);
        let b = mem.oam_word(row_offset - 8);
        let c = mem.oam_word(row_offset - 4);

        let glitched = b | (a & c);
        mem.set_oam_word(row_offset, glitched);

        for i in 0..8u8 {
            let val = mem.oam_byte(row_offset - 8 + i);
            mem.set_oam_byte(row_offset + i, val);
        }
    }

    fn accessed_oam_row(&self) -> Option<u8> {
        self.ppu.as_ref().and_then(|ppu| ppu.accessed_oam_row())
    }

    fn stat_line_active(&self) -> bool {
        let ppu = match &self.ppu {
            Some(ppu) => ppu,
            None => return false,
        };

        let mode = ppu.mode();

        // On real hardware, the mode 2 (OAM) STAT condition also triggers
        // at line 144 when VBlank starts.
        let vblank_line_144 = matches!(ppu, PixelProcessingUnit::BetweenFrames(dots) if *dots < 4);

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
                && (mode == Mode::PreparingScanline || vblank_line_144))
            || (self
                .interrupts
                .flags
                .contains(InterruptFlags::CURRENT_LINE_COMPARE)
                && self.ly_eq_lyc)
    }

    /// Advance PPU by one dot. Call once per T-cycle. Interrupt edge
    /// detection only runs on M-cycle boundaries (when `is_mcycle` is true)
    /// to match hardware behavior.
    pub fn tcycle(&mut self, is_mcycle: bool) -> VideoTickResult {
        let mut result = VideoTickResult {
            screen: None,
            request_vblank: false,
            request_stat: false,
        };

        if self.control().video_enabled() {
            if self.ppu.is_none() {
                self.ppu = Some(PixelProcessingUnit::new_lcd_on());
            }

            if let Some(screen) = self.ppu.as_mut().unwrap().tcycle(&self.ppu_accessible) {
                result.screen = Some(screen);
                result.request_vblank = true;
            }

            if !is_mcycle {
                return result;
            }

            // Update comparison clock (runs while PPU is on)
            self.ly_eq_lyc =
                self.ppu.as_ref().unwrap().current_line() == self.interrupts.current_line_compare;
        } else {
            if !is_mcycle {
                return result;
            }
            if self.ppu.is_some() {
                self.ppu = None;
                result.screen = Some(Screen::new());
            }
            // ly_eq_lyc is intentionally NOT updated — comparison clock
            // stops when the PPU is off, freezing the last result.
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

    pub(crate) fn save_state(&self) -> super::save_state::VideoState {
        use super::save_state::{PpuState, VideoState};

        // Serialize video memory as flat byte arrays
        let mut tiles = Vec::with_capacity(3 * 0x800);
        for block in self.ppu_accessible.memory.tile_blocks() {
            tiles.extend_from_slice(&block.data);
        }

        let mut tile_maps = Vec::with_capacity(2 * 0x400);
        for map in self.ppu_accessible.memory.tile_map_data() {
            for idx in &map.data {
                tile_maps.push(idx.0);
            }
        }

        let mut sprites_data = Vec::with_capacity(40 * 4);
        for sprite in self.ppu_accessible.memory.sprites() {
            sprites_data.push(sprite.position.y_plus_16);
            sprites_data.push(sprite.position.x_plus_8);
            sprites_data.push(sprite.tile.0);
            sprites_data.push(sprite.attributes.0);
        }

        let ppu = match &self.ppu {
            None => PpuState::Off,
            Some(ppu) => ppu.save_state(),
        };

        VideoState {
            control: self.ppu_accessible.control.bits(),
            background_viewport_x: self.ppu_accessible.background_viewport.x,
            background_viewport_y: self.ppu_accessible.background_viewport.y,
            window_y: self.ppu_accessible.window.y,
            window_x_plus_7: self.ppu_accessible.window.x_plus_7,
            background_palette: self.ppu_accessible.palettes.background.0,
            sprite0_palette: self.ppu_accessible.palettes.sprite0.0,
            sprite1_palette: self.ppu_accessible.palettes.sprite1.0,
            interrupt_flags: self.interrupts.flags.bits(),
            current_line_compare: self.interrupts.current_line_compare,
            stat_line_was_high: self.stat_line_was_high,
            tiles: Base64Bytes(tiles),
            tile_maps: Base64Bytes(tile_maps),
            sprites: Base64Bytes(sprites_data),
            ppu,
        }
    }

    pub(crate) fn from_state(state: super::save_state::VideoState) -> Self {
        use super::save_state::PpuState;

        let mut memory = VideoMemory::new();
        memory.load_state(&state.tiles, &state.tile_maps, &state.sprites);

        Self {
            ppu_accessible: PpuAccessible {
                control: Control::new(ControlFlags::from_bits_retain(state.control)),
                background_viewport: BackgroundViewportPosition {
                    x: state.background_viewport_x,
                    y: state.background_viewport_y,
                },
                window: Window {
                    y: state.window_y,
                    x_plus_7: state.window_x_plus_7,
                },
                palettes: Palettes {
                    background: PaletteMap(state.background_palette),
                    sprite0: PaletteMap(state.sprite0_palette),
                    sprite1: PaletteMap(state.sprite1_palette),
                },
                memory,
            },
            ppu: match state.ppu {
                PpuState::Off => None,
                _ => Some(PixelProcessingUnit::from_state(state.ppu)),
            },
            interrupts: Interrupts {
                flags: InterruptFlags::from_bits_truncate(state.interrupt_flags),
                current_line_compare: state.current_line_compare,
            },
            ly_eq_lyc: false,
            stat_line_was_high: state.stat_line_was_high,
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
