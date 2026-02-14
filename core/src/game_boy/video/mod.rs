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
                let ly_eq_lyc = if let Some(ppu) = &self.ppu {
                    if ppu.ly_transitioning() {
                        false
                    } else {
                        ppu.current_line() == self.interrupts.current_line_compare
                    }
                } else {
                    self.ly_eq_lyc
                };
                let line_compare = if ly_eq_lyc { 0b00000100 } else { 0 };
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

    /// Mode for OAM/VRAM memory gating. Reports Mode 0 during LCD-on
    /// startup (like stat_mode) but transitions to Mode 0 immediately
    /// when Mode 3 ends (no 1-dot delay).
    pub fn gating_mode(&self) -> ppu::Mode {
        if let Some(ppu) = &self.ppu {
            ppu.gating_mode()
        } else {
            ppu::Mode::BetweenFrames
        }
    }

    pub fn write_gating_mode(&self) -> ppu::Mode {
        if let Some(ppu) = &self.ppu {
            ppu.write_gating_mode()
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
    /// Read corruption has multiple variants depending on which OAM row
    /// the PPU is currently scanning. The row offset modulo 0x18 selects
    /// the variant:
    ///   0x10 → secondary (4-input formula, corrupts preceding row, copies to row±1)
    ///   0x00 → tertiary/quaternary (complex, model-specific)
    ///   0x08, 0x18 → simple (2 rows, `b | (a & c)`)
    pub fn oam_bug_read(&mut self) {
        let r = match self.accessed_oam_row() {
            Some(offset) if offset >= 8 && offset < 160 => offset,
            _ => return,
        };

        let mem = &mut self.ppu_accessible.memory;

        match r & 0x18 {
            0x10 => {
                // Secondary read corruption: affects row r-1, copies to r-2 and r.
                // Guard: row must be < 0x98 (SameBoy check).
                if r < 0x98 {
                    let a = mem.oam_word(r - 16); // two rows back
                    let b = mem.oam_word(r - 8); // preceding row (corrupted)
                    let c = mem.oam_word(r); // current row
                    let d = mem.oam_word(r - 4); // third word of preceding row

                    let glitched = (b & (a | c | d)) | (a & c & d);
                    mem.set_oam_word(r - 8, glitched);

                    // Copy preceding row to both two-rows-back and current row
                    for i in 0..8u8 {
                        let val = mem.oam_byte(r - 8 + i);
                        mem.set_oam_byte(r - 16 + i, val);
                        mem.set_oam_byte(r + i, val);
                    }
                }
            }
            0x00 => {
                // Tertiary/quaternary read corruption (DMG-specific).
                if r < 0x98 {
                    if r == 0x40 {
                        // Quaternary: 8 inputs (DMG ignores first word of OAM)
                        let b = mem.oam_word(r); // current row
                        let c = mem.oam_word(r - 4); // third word of preceding row
                        let d = mem.oam_word(r - 6); // second word of preceding row (reversed endian offset)
                        let e = mem.oam_word(r - 8); // preceding row
                        let f = mem.oam_word(r - 14); // fourth word of two-rows-back (offset)
                        let g = mem.oam_word(r - 16); // two rows back
                        let h = mem.oam_word(r - 32); // four rows back

                        // DMG quaternary: `(e & (h | g | (~d & f) | c | b)) | (c & g & h)`
                        let glitched = (e & (h | g | (!d & f) | c | b)) | (c & g & h);
                        mem.set_oam_word(r - 8, glitched);
                    } else {
                        // Tertiary read corruption
                        let a = mem.oam_word(r); // current row
                        let b = mem.oam_word(r - 4); // third word of preceding row
                        let c = mem.oam_word(r - 8); // preceding row (corrupted)
                        let d = mem.oam_word(r - 16); // two rows back
                        let e = mem.oam_word(r - 32); // four rows back

                        let glitched = match r {
                            // read_2: `(c & (a | b | d | e)) | (a & b & d & e)`
                            0x20 => (c & (a | b | d | e)) | (a & b & d & e),
                            // read_3: `(c & (a | b | d | e)) | (b & d & e)`
                            0x60 => (c & (a | b | d | e)) | (b & d & e),
                            // read_1: `c | (a & b & d & e)`
                            _ => c | (a & b & d & e),
                        };
                        mem.set_oam_word(r - 8, glitched);
                    }

                    // Copy preceding row to both two-rows-back and current row
                    for i in 0..8u8 {
                        let val = mem.oam_byte(r - 8 + i);
                        mem.set_oam_byte(r - 16 + i, val);
                        mem.set_oam_byte(r + i, val);
                    }
                }
            }
            _ => {
                // Simple read corruption (0x08, 0x18): affects current row
                // and preceding row's first word.
                let a = mem.oam_word(r);
                let b = mem.oam_word(r - 8);
                let c = mem.oam_word(r - 4);

                let glitched = b | (a & c);
                mem.set_oam_word(r - 8, glitched);
                mem.set_oam_word(r, glitched);

                // Copy preceding row to current row
                for i in 0..8u8 {
                    let val = mem.oam_byte(r - 8 + i);
                    mem.set_oam_byte(r + i, val);
                }
            }
        }

        // Special case: row 0x80 copies to row 0
        if r == 0x80 {
            for i in 0..8u8 {
                let val = mem.oam_byte(r + i);
                mem.set_oam_byte(i, val);
            }
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

        let mode = ppu.interrupt_mode();

        // On real hardware, the mode 2 (OAM) STAT condition also triggers
        // at line 144 when VBlank starts.
        let vblank_line_144 = matches!(ppu, PixelProcessingUnit::BetweenFrames(dots) if *dots < 4);

        // Mode 0 interrupt fires on the actual mode transition, not the
        // early stat_mode prediction (which is only for STAT register reads).
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
                && (ppu.mode2_interrupt_active() || vblank_line_144))
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
