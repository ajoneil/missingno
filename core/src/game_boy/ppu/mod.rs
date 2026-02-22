use bitflags::bitflags;
use pixel_pipeline::Mode;
use screen::Screen;
use sprites::{Sprite, SpriteId};

use control::{Control, ControlFlags};
use memory::{Oam, OamAddress, Vram};
use palette::{PaletteMap, Palettes};
use pixel_pipeline::PixelPipeline;

pub struct PpuTickResult {
    pub screen: Option<Screen>,
    pub request_vblank: bool,
    pub request_stat: bool,
}

pub mod control;
pub mod memory;
pub mod palette;
pub mod pixel_pipeline;
pub mod screen;
pub mod sprites;
pub mod tile_maps;
pub mod tiles;

#[derive(Debug, Clone, Copy)]
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

/// Resolution phase of a register write that hasn't fully settled.
///
/// On hardware, some DFF cells expose a transitional output during the
/// CPU write pulse before settling to the final value. DFF8 (palette)
/// cells have a master-slave structure: the master latches on the rising
/// edge while the slave still holds the old value, producing `old | new`.
/// LCDC has a similar transitional: `old | (new & BG_EN)`. The phase
/// enum tracks how far through this process the register has progressed.
enum WriteConflictPhase {
    /// Transitional value is visible to the pixel pipeline.
    /// Next tcycle advances to Settling.
    Transitional,
    /// Last dot of transitional visibility.
    /// Next tcycle writes the final value and clears the pending state.
    Settling,
}

/// A register write that is resolving over multiple dots during Mode 3.
///
/// Created when the CPU writes a register whose DFF cell produces a
/// transitional output (palettes, LCDC). The tcycle resolution advances
/// Transitional → Settling → resolved, writing the final value when done.
struct PendingWrite {
    register: Register,
    final_value: u8,
    phase: WriteConflictPhase,
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

/// The PPU register file: values the pixel pipeline reads each dot.
///
/// On hardware, these are DFF cells (pages 23, 36) whose outputs are
/// routed as signals to the pipeline blocks. The CPU writes them via
/// the register bus; the pixel pipeline only reads.
pub struct Registers {
    control: Control,
    background_viewport: BackgroundViewportPosition,
    window: Window,
    palettes: Palettes,
}

pub struct Ppu {
    pixel_pipeline: Option<PixelPipeline>,
    registers: Registers,
    pub(super) oam: Oam,
    interrupts: Interrupts,
    /// Cached LY=LYC comparison result, updated each M-cycle while the
    /// PPU is on. Frozen when the PPU is off (comparison clock stops).
    ly_eq_lyc: bool,
    stat_line_was_high: bool,
    pending_write: Option<PendingWrite>,
}

pub struct Window {
    y: u8,
    x_plus_7: u8,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            registers: Registers {
                control: Control::default(),
                background_viewport: BackgroundViewportPosition { x: 0, y: 0 },
                window: Window { y: 0, x_plus_7: 0 },
                palettes: Palettes::default(),
            },
            oam: Oam::new(),

            pixel_pipeline: Some(PixelPipeline::new()),
            interrupts: Interrupts {
                // The first bit is unused, but is set at boot time
                flags: InterruptFlags::DUMMY,
                current_line_compare: 0,
            },
            ly_eq_lyc: true,
            stat_line_was_high: false,
            pending_write: None,
        }
    }

    pub fn read_register(&self, register: Register) -> u8 {
        match register {
            Register::Control => self.registers.control.bits(),
            Register::Status => {
                let mode = if let Some(ppu) = &self.pixel_pipeline {
                    ppu.stat_mode() as u8
                } else {
                    0
                };
                let ly_eq_lyc = if let Some(ppu) = &self.pixel_pipeline {
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
            Register::BackgroundViewportY => self.registers.background_viewport.y,
            Register::BackgroundViewportX => self.registers.background_viewport.x,
            Register::WindowY => self.registers.window.y,
            Register::WindowX => self.registers.window.x_plus_7,
            Register::CurrentScanline => {
                if let Some(ppu) = &self.pixel_pipeline {
                    ppu.current_line()
                } else {
                    0
                }
            }
            Register::InterruptOnScanline => self.interrupts.current_line_compare,
            Register::BackgroundPalette => self.registers.palettes.background.0,
            Register::Sprite0Palette => self.registers.palettes.sprite0.0,
            Register::Sprite1Palette => self.registers.palettes.sprite1.0,
        }
    }

    /// Write a value directly to the register backing store.
    ///
    /// Returns true if the write triggered a STAT interrupt request
    /// (DMG STAT write quirk: writing to FF41 briefly sets all enable
    /// bits high, which can produce a rising edge on the STAT line).
    fn write_register_immediate(&mut self, register: &Register, value: u8) -> bool {
        match register {
            Register::Control => {
                self.registers.control = Control::new(ControlFlags::from_bits_retain(value))
            }
            Register::Status => {
                // DMG STAT write quirk: briefly set all enable bits high.
                // If any condition is active, this produces a rising edge.
                self.interrupts.flags = InterruptFlags::all();
                let glitch_line = self.stat_line_active();
                let glitch_edge = glitch_line && !self.stat_line_was_high;
                self.stat_line_was_high = glitch_line;

                // Now apply the real value.
                self.interrupts.flags = InterruptFlags::from_bits_truncate(value);
                let final_line = self.stat_line_active();
                let final_edge = final_line && !self.stat_line_was_high;
                self.stat_line_was_high = final_line;

                return glitch_edge || final_edge;
            }
            Register::BackgroundViewportY => self.registers.background_viewport.y = value,
            Register::BackgroundViewportX => self.registers.background_viewport.x = value,
            Register::WindowY => self.registers.window.y = value,
            Register::WindowX => self.registers.window.x_plus_7 = value,
            Register::InterruptOnScanline => self.interrupts.current_line_compare = value,
            Register::BackgroundPalette => self.registers.palettes.background = PaletteMap(value),
            Register::Sprite0Palette => self.registers.palettes.sprite0 = PaletteMap(value),
            Register::Sprite1Palette => self.registers.palettes.sprite1 = PaletteMap(value),
            Register::CurrentScanline => {} // writes to LY are ignored on DMG
        }
        false
    }

    pub fn write_register(&mut self, register: Register, value: u8, _vram: &Vram) -> bool {
        let is_drawing = self
            .pixel_pipeline
            .as_ref()
            .map_or(false, |p| p.is_rendering());

        match register {
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                if is_drawing {
                    // DFF8 master-slave transparency: output is `old | new`
                    // (transitional) until the slave settles.
                    let old = match register {
                        Register::BackgroundPalette => self.registers.palettes.background.0,
                        Register::Sprite0Palette => self.registers.palettes.sprite0.0,
                        Register::Sprite1Palette => self.registers.palettes.sprite1.0,
                        _ => unreachable!(),
                    };
                    self.write_register_immediate(&register, old | value);
                    self.pending_write = Some(PendingWrite {
                        register,
                        final_value: value,
                        phase: WriteConflictPhase::Transitional,
                    });
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            Register::Control => {
                if is_drawing {
                    // LCDC write conflict: BG_EN bit of new value OR'd with
                    // old LCDC is visible as a transitional before settling.
                    let old = self.registers.control.bits();
                    let transitional =
                        old | (value & ControlFlags::BACKGROUND_AND_WINDOW_ENABLE.bits());
                    self.write_register_immediate(&Register::Control, transitional);
                    self.pending_write = Some(PendingWrite {
                        register,
                        final_value: value,
                        phase: WriteConflictPhase::Transitional,
                    });
                    false
                } else {
                    self.write_register_immediate(&register, value)
                }
            }
            _ => {
                // DFF9 registers with no transitional: atomic latch at
                // the write point (G→H boundary, after all 4 PPU dots).
                self.write_register_immediate(&register, value)
            }
        }
    }

    pub fn read_oam(&self, address: OamAddress) -> u8 {
        self.oam.read(address)
    }

    pub fn write_oam(&mut self, address: OamAddress, value: u8) {
        self.oam.write(address, value);
    }

    pub fn mode(&self) -> pixel_pipeline::Mode {
        if let Some(ppu) = &self.pixel_pipeline {
            ppu.mode()
        } else {
            pixel_pipeline::Mode::BetweenFrames
        }
    }

    pub fn oam_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.oam_locked())
    }

    pub fn vram_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.vram_locked())
    }

    pub fn oam_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.oam_write_locked())
    }

    pub fn vram_write_locked(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |ppu| ppu.vram_write_locked())
    }

    pub fn control(&self) -> Control {
        self.registers.control
    }

    pub fn is_rendering(&self) -> bool {
        self.pixel_pipeline
            .as_ref()
            .map_or(false, |p| p.is_rendering())
    }

    /// Stage a DFF8 palette write at the rising-edge latch point.
    ///
    // --- OAM corruption bug ---
    //
    // On DMG hardware, a design flaw in the OAM SRAM clock generation
    // causes corruption when the CPU accesses OAM during Mode 2
    // (scanning). The OAM clock signal CUFE is derived from the CPU's
    // internal address bus — not the OAM address bus. ASAM blocks the
    // CPU from driving the OAM *address* bus during scanning, but CUFE
    // still sees the CPU address and generates spurious SRAM clock edges.
    // This clocks the SRAM while the scanner owns the address/data
    // buses, producing garbled reads and writes.
    //
    // The corruption formulas below are empirical — they describe the
    // analog result of SRAM cells being disturbed during bus contention.
    // The exact formulas depend on the physical SRAM cell layout (bit
    // line routing, parasitic capacitance) and vary by die revision.
    // They cannot be derived from a digital gate-level model; GateBoy's
    // tri_bus asserts on the collision and fails the oam_bug tests.
    //
    // OAM is organized as 20 rows of 8 bytes (4 words of 16 bits).
    // The scanner advances through one row pair (2 entries = 8 bytes)
    // per M-cycle. Corruption targets the row the scanner is currently
    // accessing, with effects spilling into adjacent rows.
    //
    // Sources:
    //   Trigger mechanism: GateBoy die analysis (CUFE, BYCU, ASAM)
    //   Corruption formulas: Pan Docs "OAM Corruption Bug"
    //   Position-dependent read variants: SameBoy (Core/memory.c)

    /// Trigger OAM bug write corruption during Mode 2.
    ///
    /// Fires when the CPU's IDU or a CPU write places an OAM-range
    /// address on the bus while the scanner owns the OAM SRAM.
    /// The spurious SRAM clock causes a garbled write to the
    /// scanner's current row.
    pub fn oam_bug_write(&mut self) {
        let row = match self.corrupted_oam_row() {
            Some(row) if row >= 8 && row < 160 => row,
            _ => return,
        };

        let oam = &mut self.oam;

        // Corruption of the first word in the row. The three inputs
        // are the row's own first word and two words from the
        // preceding row (its first and third words). The formula
        // models the SRAM cell output under bus contention.
        let row_word0 = oam.oam_word(row);
        let prev_word0 = oam.oam_word(row - 8);
        let prev_word2 = oam.oam_word(row - 4);

        let glitched = ((row_word0 ^ prev_word2) & (prev_word0 ^ prev_word2)) ^ prev_word2;
        oam.set_oam_word(row, glitched);

        // The last 3 words of the row are overwritten with the
        // preceding row's last 3 words (bytes 2–7 copied).
        for i in 2..8u8 {
            let val = oam.oam_byte(row - 8 + i);
            oam.set_oam_byte(row + i, val);
        }
    }

    /// Trigger OAM bug read corruption during Mode 2.
    ///
    /// Read corruption has position-dependent variants because
    /// different SRAM row positions have different physical bit line
    /// routing, producing different parasitic coupling patterns.
    /// The variant is selected by `row & 0x18` (which 8-row group
    /// the row falls into within the SRAM array).
    ///
    /// These variants are revision-specific and even unit-specific.
    /// The formulas here target DMG behaviour.
    pub fn oam_bug_read(&mut self) {
        let row = match self.corrupted_oam_row() {
            Some(row) if row >= 8 && row < 160 => row,
            _ => return,
        };

        let oam = &mut self.oam;

        match row & 0x18 {
            0x10 => {
                // Secondary read corruption.
                // The 4-input formula corrupts the preceding row's
                // first word, then the preceding row is copied to
                // both the current row and two rows back.
                if row < 0x98 {
                    let two_back_word0 = oam.oam_word(row - 16);
                    let prev_word0 = oam.oam_word(row - 8);
                    let row_word0 = oam.oam_word(row);
                    let prev_word2 = oam.oam_word(row - 4);

                    let glitched = (prev_word0 & (two_back_word0 | row_word0 | prev_word2))
                        | (two_back_word0 & row_word0 & prev_word2);
                    oam.set_oam_word(row - 8, glitched);

                    for i in 0..8u8 {
                        let val = oam.oam_byte(row - 8 + i);
                        oam.set_oam_byte(row - 16 + i, val);
                        oam.set_oam_byte(row + i, val);
                    }
                }
            }
            0x00 => {
                // Tertiary/quaternary read corruption.
                // These involve more distant rows due to the SRAM
                // physical layout at these addresses. The formulas
                // are DMG-specific and vary even between DMG units.
                if row < 0x98 {
                    if row == 0x40 {
                        // Quaternary (8 inputs). Some DMG units produce
                        // non-deterministic results here; we emulate
                        // the units that produce deterministic output.
                        let row_word0 = oam.oam_word(row);
                        let prev_word2 = oam.oam_word(row - 4);
                        let prev_word1 = oam.oam_word(row - 6);
                        let prev_word0 = oam.oam_word(row - 8);
                        let two_back_word3 = oam.oam_word(row - 14);
                        let two_back_word0 = oam.oam_word(row - 16);
                        let four_back_word0 = oam.oam_word(row - 32);

                        let glitched = (prev_word0
                            & (four_back_word0
                                | two_back_word0
                                | (!prev_word1 & two_back_word3)
                                | prev_word2
                                | row_word0))
                            | (prev_word2 & two_back_word0 & four_back_word0);
                        oam.set_oam_word(row - 8, glitched);
                    } else {
                        // Tertiary (5 inputs). The exact formula varies
                        // by row position within the SRAM array.
                        let row_word0 = oam.oam_word(row);
                        let prev_word2 = oam.oam_word(row - 4);
                        let prev_word0 = oam.oam_word(row - 8);
                        let two_back_word0 = oam.oam_word(row - 16);
                        let four_back_word0 = oam.oam_word(row - 32);

                        let glitched = match row {
                            0x20 => {
                                (prev_word0
                                    & (row_word0 | prev_word2 | two_back_word0 | four_back_word0))
                                    | (row_word0 & prev_word2 & two_back_word0 & four_back_word0)
                            }
                            0x60 => {
                                (prev_word0
                                    & (row_word0 | prev_word2 | two_back_word0 | four_back_word0))
                                    | (prev_word2 & two_back_word0 & four_back_word0)
                            }
                            _ => {
                                prev_word0
                                    | (row_word0 & prev_word2 & two_back_word0 & four_back_word0)
                            }
                        };
                        oam.set_oam_word(row - 8, glitched);
                    }

                    for i in 0..8u8 {
                        let val = oam.oam_byte(row - 8 + i);
                        oam.set_oam_byte(row - 16 + i, val);
                        oam.set_oam_byte(row + i, val);
                    }
                }
            }
            _ => {
                // Simple read corruption (rows where `row & 0x18`
                // is 0x08 or 0x18). This is the Pan Docs "read"
                // formula — the simplest coupling pattern.
                let row_word0 = oam.oam_word(row);
                let prev_word0 = oam.oam_word(row - 8);
                let prev_word2 = oam.oam_word(row - 4);

                let glitched = prev_word0 | (row_word0 & prev_word2);
                oam.set_oam_word(row - 8, glitched);
                oam.set_oam_word(row, glitched);

                for i in 0..8u8 {
                    let val = oam.oam_byte(row - 8 + i);
                    oam.set_oam_byte(row + i, val);
                }
            }
        }

        // Row 0x80 additionally copies to row 0 — an SRAM array
        // wraparound effect at the physical layout boundary.
        if row == 0x80 {
            for i in 0..8u8 {
                let val = oam.oam_byte(row + i);
                oam.set_oam_byte(i, val);
            }
        }
    }

    /// Which OAM row the scanner is currently accessing.
    ///
    /// OAM is organized as 8-byte rows (2 entries per row). The
    /// scanner's byte address is rounded to the next row boundary.
    /// The corruption fires at T2 of the M-cycle (matching the
    /// hardware CUFE clock).
    fn corrupted_oam_row(&self) -> Option<u8> {
        self.pixel_pipeline
            .as_ref()
            .and_then(|ppu| ppu.scanner_oam_address())
            .map(|address| (address / 8 + 1) * 8)
    }

    fn stat_line_active(&self) -> bool {
        let ppu = match &self.pixel_pipeline {
            Some(ppu) => ppu,
            None => return false,
        };

        let mode = ppu.interrupt_mode();

        // On real hardware, the mode 2 (OAM) STAT condition also triggers
        // at line 144 when VBlank starts.
        let vblank_line_144 = matches!(ppu, PixelPipeline::BetweenFrames(dots) if *dots < 4);

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

    /// Advance PPU by one dot. Call once per T-cycle.
    ///
    /// Interrupt edge detection and LYC comparison only run on
    /// M-cycle boundaries (when `is_mcycle` is true).
    pub fn tcycle(&mut self, is_mcycle: bool, vram: &Vram) -> PpuTickResult {
        let mut result = PpuTickResult {
            screen: None,
            request_vblank: false,
            request_stat: false,
        };

        if self.control().video_enabled() {
            if self.pixel_pipeline.is_none() {
                self.pixel_pipeline = Some(PixelPipeline::new_lcd_on());
            }

            // Advance write-conflict resolution before pixel output.
            // Transitional → Settling: transitional value remains visible.
            // Settling → resolved: final value written, pending cleared.
            match self.pending_write {
                Some(PendingWrite {
                    phase: WriteConflictPhase::Transitional,
                    ..
                }) => {
                    self.pending_write.as_mut().unwrap().phase = WriteConflictPhase::Settling;
                }
                Some(PendingWrite {
                    phase: WriteConflictPhase::Settling,
                    ..
                }) => {
                    let reg = self.pending_write.as_ref().unwrap().register;
                    let val = self.pending_write.as_ref().unwrap().final_value;
                    self.pending_write = None;
                    self.write_register_immediate(&reg, val);
                }
                None => {}
            }

            // Normal path: tick PPU immediately, one dot per T-cycle.
            if let Some(screen) =
                self.pixel_pipeline
                    .as_mut()
                    .unwrap()
                    .tcycle(&self.registers, &self.oam, vram)
            {
                result.screen = Some(screen);
                result.request_vblank = true;
            }

            if !is_mcycle {
                return result;
            }

            // Update comparison clock (runs while PPU is on)
            self.ly_eq_lyc = self.pixel_pipeline.as_ref().unwrap().current_line()
                == self.interrupts.current_line_compare;
        } else {
            if !is_mcycle {
                return result;
            }
            if self.pixel_pipeline.is_some() {
                self.pixel_pipeline = None;
                self.pending_write = None;
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
        &self.registers.palettes
    }

    pub fn sprite(&self, sprite: SpriteId) -> &Sprite {
        self.oam.sprite(sprite)
    }
}
