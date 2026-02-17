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

/// How the PPU observes a mid-rendering CPU write to this register.
///
/// On hardware, the CPU write pulse spans the first 3 of 8 sub-dot
/// phases per M-cycle. The PPU may sample the register before the
/// DFF latches the new value, observing the old value for 1-2 extra
/// dots. Some registers also exhibit a 1-dot transitional value
/// where old and new bits are OR'd together.
enum WriteConflict {
    /// PPU sees the new value immediately (0 dots early).
    /// Used by: WY, WX, LYC, STAT (sampled at or after the DFF latch point).
    Immediate,

    /// PPU sees the old value for 1 extra dot, then the new value.
    /// Used by: SCY (sampled 1 dot before the DFF latch point).
    OneDotEarly,

    /// PPU sees the old value for 2 extra dots, then the new value.
    /// Used by: SCX (sampled 2 dots before the DFF latch point).
    TwoDotsEarly,

    /// PPU sees a transitional `old | new` value for 1 dot, then the
    /// new value, starting 2 dots early.
    /// Used by: BGP, OBP0, OBP1 (palette registers use DFF8 cells
    /// whose master-slave transition is visible through the pixel pipeline).
    PaletteDmg,

    /// PPU sees a transitional value for 1 dot (old OR'd with the
    /// BG_EN bit of the new value), then the new value, starting
    /// 2 dots early.
    /// Used by: LCDC (similar master-slave visibility as palettes,
    /// but only the BG_EN bit propagates through the master stage).
    LcdcDmg,
}

impl Register {
    fn write_conflict(&self) -> WriteConflict {
        match self {
            Register::BackgroundPalette | Register::Sprite0Palette | Register::Sprite1Palette => {
                WriteConflict::PaletteDmg
            }

            Register::Control => WriteConflict::LcdcDmg,

            Register::BackgroundViewportX => WriteConflict::TwoDotsEarly,

            Register::BackgroundViewportY => WriteConflict::OneDotEarly,

            Register::WindowY
            | Register::WindowX
            | Register::InterruptOnScanline
            | Register::Status
            | Register::CurrentScanline => WriteConflict::Immediate,
        }
    }
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
    /// PPU dots accumulated but not yet processed. When
    /// `accumulating` is true, T-cycle ticks increment this counter
    /// instead of advancing the PPU, deferring dots for write
    /// conflict splitting.
    pending_dots: u8,
    /// When true, `tcycle()` accumulates dots instead of ticking the
    /// PPU. Set by the execute loop when it knows a PPU register
    /// write is coming and needs deferred dots for conflict splitting.
    accumulating: bool,

    /// Screen completed during a deferred PPU flush, delivered on
    /// the next `tcycle()` return.
    pending_screen: Option<Screen>,
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
            pending_dots: 0,
            accumulating: false,
            pending_screen: None,
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

    /// Flush `count` pending PPU dots. Ticks the PPU that many times,
    /// stashing any completed screen. Does NOT run interrupt edge
    /// detection or LYC comparison — those only run at M-cycle
    /// boundaries in the main `tcycle()` path.
    fn flush_dots(&mut self, count: u8, vram: &Vram) {
        if let Some(ppu) = self.pixel_pipeline.as_mut() {
            for _ in 0..count {
                if let Some(screen) = ppu.tcycle(&self.registers, &self.oam, vram) {
                    self.pending_screen = Some(screen);
                }
            }
        }
        self.pending_dots -= count;
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

    /// Returns true if the PPU is actively drawing pixels (Mode 3),
    /// meaning register writes may conflict with PPU reads.
    pub fn ppu_is_drawing(&self) -> bool {
        matches!(&self.pixel_pipeline, Some(ppu) if ppu.is_rendering())
    }

    pub fn write_register(&mut self, register: Register, value: u8, vram: &Vram) -> bool {
        // Write conflict splitting requires enough deferred dots
        // (pending_dots >= 5) and the PPU actively drawing (Mode 3).
        // The execute loop sets accumulating=true for the M-cycle
        // before a PPU register write, giving 4 deferred dots + 1
        // from T0 of the write M-cycle = 5 pending at T1.
        if self.pending_dots < 5 || !self.ppu_is_drawing() {
            let stat = self.write_register_immediate(&register, value);
            // Stop accumulating — the write is done.
            self.accumulating = false;
            return stat;
        }

        // Stop accumulating — all pending dots will be flushed during
        // the split below. After the split, pending_dots is 0 and
        // normal per-T-cycle ticking resumes.
        self.accumulating = false;

        // Split pending dots around the register write. With 5 pending
        // (4 from opcode fetch + 1 from write T0), the split matches
        // SameBoy's cycle_write advance(pending_cycles - N). Our PPU
        // position matches SameBoy's (both at the dot before the opcode
        // fetch), so we flush the same count: 4-N = pending_dots-1-N.
        //
        // After the split, flush all remaining pending dots with the
        // final value so the PPU is caught up before normal ticking.
        let stat = match register.write_conflict() {
            WriteConflict::Immediate => {
                // SameBoy READ_OLD (N=0): advance(4). flush(4).
                self.flush_dots(self.pending_dots - 1, vram);
                let stat = self.write_register_immediate(&register, value);
                self.flush_dots(self.pending_dots, vram);
                stat
            }

            WriteConflict::OneDotEarly => {
                // SameBoy READ_NEW (N=1): advance(3). flush(3).
                self.flush_dots(self.pending_dots - 2, vram);
                let stat = self.write_register_immediate(&register, value);
                self.flush_dots(self.pending_dots, vram);
                stat
            }

            WriteConflict::TwoDotsEarly => {
                // SameBoy SCX_DMG (N=2): advance(2). flush(2).
                self.flush_dots(self.pending_dots - 3, vram);
                let stat = self.write_register_immediate(&register, value);
                self.flush_dots(self.pending_dots, vram);
                stat
            }

            WriteConflict::PaletteDmg => {
                // SameBoy PALETTE_DMG (N=2): advance(2), write
                // transitional (old|new), advance(1), write final.
                let old = match &register {
                    Register::BackgroundPalette => self.registers.palettes.background.0,
                    Register::Sprite0Palette => self.registers.palettes.sprite0.0,
                    Register::Sprite1Palette => self.registers.palettes.sprite1.0,
                    _ => unreachable!(),
                };
                self.flush_dots(self.pending_dots - 3, vram);
                self.write_register_immediate(&register, old | value);
                self.flush_dots(1, vram);
                let stat = self.write_register_immediate(&register, value);
                self.flush_dots(self.pending_dots, vram);
                stat
            }

            WriteConflict::LcdcDmg => {
                // SameBoy DMG_LCDC (N=2): same as PALETTE_DMG but
                // transitional = old | BG_EN bit of new.
                let old = self.registers.control.bits();
                let transitional =
                    old | (value & ControlFlags::BACKGROUND_AND_WINDOW_ENABLE.bits());
                self.flush_dots(self.pending_dots - 3, vram);
                self.write_register_immediate(&register, transitional);
                self.flush_dots(1, vram);
                let stat = self.write_register_immediate(&register, value);
                self.flush_dots(self.pending_dots, vram);
                stat
            }
        };
        stat
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

        let oam = &mut self.oam;
        let a = oam.oam_word(row_offset);
        let b = oam.oam_word(row_offset - 8);
        let c = oam.oam_word(row_offset - 4);

        let glitched = ((a ^ c) & (b ^ c)) ^ c;
        oam.set_oam_word(row_offset, glitched);

        for i in 2..8u8 {
            let val = oam.oam_byte(row_offset - 8 + i);
            oam.set_oam_byte(row_offset + i, val);
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

        let oam = &mut self.oam;

        match r & 0x18 {
            0x10 => {
                // Secondary read corruption: affects row r-1, copies to r-2 and r.
                // Guard: row must be < 0x98 (SameBoy check).
                if r < 0x98 {
                    let a = oam.oam_word(r - 16); // two rows back
                    let b = oam.oam_word(r - 8); // preceding row (corrupted)
                    let c = oam.oam_word(r); // current row
                    let d = oam.oam_word(r - 4); // third word of preceding row

                    let glitched = (b & (a | c | d)) | (a & c & d);
                    oam.set_oam_word(r - 8, glitched);

                    // Copy preceding row to both two-rows-back and current row
                    for i in 0..8u8 {
                        let val = oam.oam_byte(r - 8 + i);
                        oam.set_oam_byte(r - 16 + i, val);
                        oam.set_oam_byte(r + i, val);
                    }
                }
            }
            0x00 => {
                // Tertiary/quaternary read corruption (DMG-specific).
                if r < 0x98 {
                    if r == 0x40 {
                        // Quaternary: 8 inputs (DMG ignores first word of OAM)
                        let b = oam.oam_word(r); // current row
                        let c = oam.oam_word(r - 4); // third word of preceding row
                        let d = oam.oam_word(r - 6); // second word of preceding row (reversed endian offset)
                        let e = oam.oam_word(r - 8); // preceding row
                        let f = oam.oam_word(r - 14); // fourth word of two-rows-back (offset)
                        let g = oam.oam_word(r - 16); // two rows back
                        let h = oam.oam_word(r - 32); // four rows back

                        // DMG quaternary: `(e & (h | g | (~d & f) | c | b)) | (c & g & h)`
                        let glitched = (e & (h | g | (!d & f) | c | b)) | (c & g & h);
                        oam.set_oam_word(r - 8, glitched);
                    } else {
                        // Tertiary read corruption
                        let a = oam.oam_word(r); // current row
                        let b = oam.oam_word(r - 4); // third word of preceding row
                        let c = oam.oam_word(r - 8); // preceding row (corrupted)
                        let d = oam.oam_word(r - 16); // two rows back
                        let e = oam.oam_word(r - 32); // four rows back

                        let glitched = match r {
                            // read_2: `(c & (a | b | d | e)) | (a & b & d & e)`
                            0x20 => (c & (a | b | d | e)) | (a & b & d & e),
                            // read_3: `(c & (a | b | d | e)) | (b & d & e)`
                            0x60 => (c & (a | b | d | e)) | (b & d & e),
                            // read_1: `c | (a & b & d & e)`
                            _ => c | (a & b & d & e),
                        };
                        oam.set_oam_word(r - 8, glitched);
                    }

                    // Copy preceding row to both two-rows-back and current row
                    for i in 0..8u8 {
                        let val = oam.oam_byte(r - 8 + i);
                        oam.set_oam_byte(r - 16 + i, val);
                        oam.set_oam_byte(r + i, val);
                    }
                }
            }
            _ => {
                // Simple read corruption (0x08, 0x18): affects current row
                // and preceding row's first word.
                let a = oam.oam_word(r);
                let b = oam.oam_word(r - 8);
                let c = oam.oam_word(r - 4);

                let glitched = b | (a & c);
                oam.set_oam_word(r - 8, glitched);
                oam.set_oam_word(r, glitched);

                // Copy preceding row to current row
                for i in 0..8u8 {
                    let val = oam.oam_byte(r - 8 + i);
                    oam.set_oam_byte(r + i, val);
                }
            }
        }

        // Special case: row 0x80 copies to row 0
        if r == 0x80 {
            for i in 0..8u8 {
                let val = oam.oam_byte(r + i);
                oam.set_oam_byte(i, val);
            }
        }
    }

    fn accessed_oam_row(&self) -> Option<u8> {
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

    /// Begin accumulating PPU dots instead of ticking. Called by the
    /// execute loop when it knows a PPU register write is coming and
    /// needs deferred dots for write conflict splitting.
    pub fn start_accumulating(&mut self) {
        self.accumulating = true;
    }

    /// Stop accumulating and flush all pending dots. Called by the
    /// execute loop when a tentative accumulation is cancelled (the
    /// instruction turned out not to write a PPU register).
    pub fn stop_accumulating_and_flush(&mut self, vram: &Vram) {
        self.accumulating = false;
        if self.pending_dots > 0 {
            self.flush_dots(self.pending_dots, vram);
        }
    }

    /// Advance PPU by one dot. Call once per T-cycle.
    ///
    /// When `accumulating` is true, dots are counted but not
    /// processed — they stay pending for `write_register()` to split
    /// around a register write. When false, the PPU ticks normally
    /// (one dot per call).
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

            if self.accumulating {
                // Dots are deferred for write conflict splitting.
                self.pending_dots += 1;
            } else if let Some(screen) =
                self.pixel_pipeline
                    .as_mut()
                    .unwrap()
                    .tcycle(&self.registers, &self.oam, vram)
            {
                // Normal path: tick PPU immediately.
                result.screen = Some(screen);
                result.request_vblank = true;
            }

            if !is_mcycle {
                return result;
            }

            // M-cycle boundary: flush any pending dots and deliver
            // deferred results. Accumulating boundaries skip the
            // flush — the execute loop will flush via write_register
            // or stop_accumulating_and_flush.
            if !self.accumulating && self.pending_dots > 0 {
                self.flush_dots(self.pending_dots, vram);
            }

            // Deliver any screen completed during flush.
            if let Some(screen) = self.pending_screen.take() {
                result.screen = Some(screen);
                result.request_vblank = true;
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
                self.pending_dots = 0;
                self.accumulating = false;
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
