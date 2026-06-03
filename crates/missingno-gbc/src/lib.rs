//! Game Boy Color emulation.
//!
//! The CGB reuses the shared SM83-based hardware modules from
//! `missingno-gb` through the generic [`Console`](missingno_gb::Console)
//! core; this crate supplies only the CGB-specific [`Model`] seams.
//! CGB behaviour (color palette memory, VRAM/WRAM banking, double-speed,
//! HDMA, object priority) attaches there.
//!
//! No SGB co-processor and no DMG OAM corruption bug — both are
//! DMG-only hardware quirks.
//!
//! ## Target SoC revision
//!
//! The CGB went through several CPU-SoC revisions (CPU-CGB-A through
//! CPU-CGB-E). Behaviour differs subtly between them — STOP/double-speed
//! wakeup timing, PPU mode-boundary alignment, STAT IRQ edges, APU
//! envelope retrigger, and so on. This crate targets **CPU-CGB-C**:
//! the most commonly-targeted revision across emulators (Gambatte's
//! `cgb04c`), the best-documented in test ROMs, and behaviourally
//! representative of the mainstream CGB hardware run.
//!
//! Test suites filter their ROM selection accordingly — CGB-E-only or
//! CGB-B-only ROMs are excluded from the CGB-C-passing set.

pub mod screen;

use missingno_gb::ppu::memory::{Vram, VramAddress, VramBank};
use missingno_gb::ppu::types::palette::{PaletteIndex, PaletteMap};
use missingno_gb::ppu::{ColorRegister, PipelineRegisters, PixelMux, PpuModel};
use missingno_gb::{Console, Model, StopAction, cartridge::Cartridge, cpu::Cpu};

use crate::screen::{Color555, GREYSCALE, Screen};

/// One CGB colour-palette RAM (BG or OBJ): 8 palettes × 4 colours × 2 bytes,
/// addressed by a 6-bit index that auto-increments on data writes (BCPS/OCPS
/// bit 7). Data writes during mode 3 are dropped but still advance the index.
pub struct ColorRam {
    data: [u8; 64],
    index: u8,
    auto_increment: bool,
}

impl Default for ColorRam {
    fn default() -> Self {
        Self {
            data: [0; 64],
            index: 0,
            auto_increment: false,
        }
    }
}

impl ColorRam {
    fn read_index(&self) -> u8 {
        0x40 | ((self.auto_increment as u8) << 7) | self.index
    }

    fn write_index(&mut self, value: u8) {
        self.index = value & 0x3F;
        self.auto_increment = value & 0x80 != 0;
    }

    fn read_data(&self) -> u8 {
        self.data[self.index as usize]
    }

    fn write_data(&mut self, value: u8) {
        self.data[self.index as usize] = value;
        self.advance();
    }

    /// Mode-3 blocked write: the colour byte is dropped, but the index still advances.
    fn skip_data(&mut self) {
        self.advance();
    }

    /// The RGB555 colour for (palette 0-7, colour index 0-3): a little-endian
    /// 2-byte entry at `(palette*4 + index)*2`. Bit 15 is unused.
    fn color(&self, palette: u8, index: u8) -> Color555 {
        let base = (palette as usize * 4 + index as usize) * 2;
        let value = self.data[base] as u16 | ((self.data[base + 1] as u16) << 8);
        Color555(value & 0x7FFF)
    }

    fn advance(&mut self) {
        if self.auto_increment {
            self.index = (self.index + 1) & 0x3F;
        }
    }
}

/// A CGB BG map attribute byte (VRAM bank 1, one per tile-map cell): bits 2-0
/// BG palette, bit 3 tile VRAM bank, bit 5 X-flip, bit 6 Y-flip, bit 7 BG-to-OBJ
/// priority (bit 4 unused). Rides the BG shifter across its tile's 8 pixels.
#[derive(Copy, Clone, Default)]
pub struct BgAttribute(pub u8);

impl BgAttribute {
    fn palette(self) -> u8 {
        self.0 & 0x07
    }

    fn tile_bank(self) -> u8 {
        (self.0 >> 3) & 0x01
    }

    fn flip_x(self) -> bool {
        self.0 & 0x20 != 0
    }

    fn flip_y(self) -> bool {
        self.0 & 0x40 != 0
    }
}

/// CGB video RAM: two 8 KiB banks selected by VBK ($FF4F). Bank 1 additionally
/// carries the BG map attributes (read by the colour fetch as it lands).
#[derive(Default)]
pub struct CgbVram {
    banks: [VramBank; 2],
    /// VBK bit 0 — the bank the CPU sees at $8000-$9FFF.
    selected: u8,
}

impl Vram for CgbVram {
    fn cpu_read(&self, address: VramAddress) -> u8 {
        self.banks[self.selected as usize].read(address)
    }

    fn cpu_write(&mut self, address: VramAddress, value: u8) {
        self.banks[self.selected as usize].write(address, value);
    }

    fn bank(&self, bank: u8) -> &VramBank {
        &self.banks[bank as usize]
    }

    fn read_bank_select(&self) -> u8 {
        0xFE | self.selected
    }

    fn write_bank_select(&mut self, value: u8) {
        self.selected = value & 0x01;
    }

    fn init_post_boot(&mut self, logo: &[u8; 0x30]) {
        self.banks[0].seed_post_boot(logo);
    }
}

/// The CGB colour PPU. Holds the BG/OBJ colour-palette RAM; the BG layer now
/// resolves through the BG attribute + BG palette RAM to RGB555. Objects still
/// resolve through the shared DMG OBP shade until the OBJ colour pipeline lands.
#[derive(Default)]
pub struct CgbPpu {
    bg_cram: ColorRam,
    obj_cram: ColorRam,
}

impl PpuModel for CgbPpu {
    type Vram = CgbVram;
    type BgCell = BgAttribute;
    type Pixel = Color555;

    fn bg_attribute(vram: &CgbVram, map_offset: u16) -> BgAttribute {
        BgAttribute(vram.bank(1).read_byte(map_offset))
    }

    fn bg_tile_source(cell: BgAttribute, fine_y: u8) -> (u8, u8) {
        let row = if cell.flip_y() { 7 - fine_y } else { fine_y };
        (cell.tile_bank(), row)
    }

    fn flip_bg_planes(cell: BgAttribute, low: u8, high: u8) -> (u8, u8) {
        if cell.flip_x() {
            (low.reverse_bits(), high.reverse_bits())
        } else {
            (low, high)
        }
    }

    fn resolve(&self, mux: &PixelMux<BgAttribute>, regs: &PipelineRegisters) -> Color555 {
        let bg_index = (mux.bg_hi << 1) | mux.bg_lo;

        // OBJ resolves through the DMG OBP shade until the OBJ colour pipeline
        // (OBJ palette RAM, 3-bit palette, full BG-vs-OBJ priority) lands. The
        // BG-blocks-OBJ test mirrors the shared XULA/WOXA → NULY priority.
        if regs.sprites_enabled_for_resolve() {
            let spr_index = (mux.spr_hi << 1) | mux.spr_lo;
            let bg_blocks_obj = regs.bg_window_enabled_for_resolve() && bg_index != 0;
            if spr_index != 0 && (mux.spr_pri == 0 || !bg_blocks_obj) {
                let palette = if mux.spr_pal == 0 {
                    regs.palettes.sprite0.output()
                } else {
                    regs.palettes.sprite1.output()
                };
                let shade = PaletteMap(palette).map(PaletteIndex(spr_index)).0;
                return GREYSCALE[shade as usize];
            }
        }

        // BG/Window in colour: the CGB always draws the BG from its palette RAM
        // (LCDC.0 is BG/OBJ master priority, not a BG blank).
        self.bg_cram.color(mux.bg_cell.palette(), bg_index)
    }

    fn trace_shade(pixel: Color555) -> u8 {
        GREYSCALE
            .iter()
            .position(|&grey| grey == pixel)
            .unwrap_or(0) as u8
    }

    fn read_color_register(&self, register: ColorRegister, rendering: bool) -> u8 {
        match register {
            ColorRegister::BackgroundIndex => self.bg_cram.read_index(),
            ColorRegister::ObjectIndex => self.obj_cram.read_index(),
            ColorRegister::BackgroundData if rendering => 0xFF,
            ColorRegister::ObjectData if rendering => 0xFF,
            ColorRegister::BackgroundData => self.bg_cram.read_data(),
            ColorRegister::ObjectData => self.obj_cram.read_data(),
        }
    }

    fn write_color_register(&mut self, register: ColorRegister, value: u8, rendering: bool) {
        match register {
            ColorRegister::BackgroundIndex => self.bg_cram.write_index(value),
            ColorRegister::ObjectIndex => self.obj_cram.write_index(value),
            ColorRegister::BackgroundData if rendering => self.bg_cram.skip_data(),
            ColorRegister::ObjectData if rendering => self.obj_cram.skip_data(),
            ColorRegister::BackgroundData => self.bg_cram.write_data(value),
            ColorRegister::ObjectData => self.obj_cram.write_data(value),
        }
    }
}

/// The Game Boy Color [`Model`]. Remaining CGB features (VBK, CRAM, HDMA) and
/// the color pixel pipeline attach here as they land.
pub struct Cgb {
    /// 8 × 4 KiB work-RAM banks. C000-CFFF is fixed bank 0; D000-DFFF is the
    /// SVBK-selected bank.
    wram: Box<[u8; 0x8000]>,
    /// SVBK ($FF70) bits 0-2 as written; the effective D000 bank is `max(svbk, 1)`.
    svbk: u8,
    /// KEY1 ($FF4D) bit 0 — speed-switch arm.
    key1_armed: bool,
    /// KEY1 ($FF4D) bit 7 — current speed (false = normal, true = double).
    /// The switch toggles it; the 2× clock cadence itself lands later.
    double_speed: bool,
    /// OPRI ($FF6C) bit 0 — object priority mode (0 = by OAM index). The
    /// priority effect lands with the color PPU.
    opri: bool,
}

impl Default for Cgb {
    fn default() -> Self {
        Self {
            wram: Box::new([0; 0x8000]),
            svbk: 1,
            key1_armed: false,
            double_speed: false,
            opri: false,
        }
    }
}

impl Cgb {
    /// Index into `wram` for a work-RAM or echo-RAM address, else `None`.
    fn wram_index(&self, address: u16) -> Option<usize> {
        let bank = if self.svbk == 0 { 1 } else { self.svbk } as usize;
        let banked = |within: u16| bank * 0x1000 + within as usize;
        match address {
            0xC000..=0xCFFF => Some((address - 0xC000) as usize),
            0xD000..=0xDFFF => Some(banked(address - 0xD000)),
            0xE000..=0xEFFF => Some((address - 0xE000) as usize),
            0xF000..=0xFDFF => Some(banked(address - 0xF000)),
            _ => None,
        }
    }
}

impl Model for Cgb {
    type Ppu = CgbPpu;
    type Screen = Screen;

    fn cpu_post_boot(_checksum: u8) -> Cpu {
        Cpu::post_boot_cgb()
    }

    fn resolve_stop(&mut self) -> StopAction {
        if self.key1_armed {
            self.double_speed = !self.double_speed;
            self.key1_armed = false;
            StopAction::SpeedSwitch
        } else {
            StopAction::Remain
        }
    }

    fn cpu_steps_per_dot(&self) -> u8 {
        if self.double_speed { 2 } else { 1 }
    }

    fn on_reset(&mut self, _cartridge: &Cartridge) {
        *self = Self::default();
    }

    fn map_read(&self, address: u16) -> Option<u8> {
        if let Some(i) = self.wram_index(address) {
            return Some(self.wram[i]);
        }
        match address {
            0xFF4C => Some(0xFF), // KEY0: boot-locked
            0xFF4D => Some(0x7E | ((self.double_speed as u8) << 7) | self.key1_armed as u8), // KEY1
            0xFF6C => Some(0xFE | self.opri as u8), // OPRI: bit0
            0xFF70 => Some(self.svbk | 0xF8), // SVBK: bits 0-2
            _ => None,
        }
    }

    fn map_write(&mut self, address: u16, value: u8) -> bool {
        if let Some(i) = self.wram_index(address) {
            self.wram[i] = value;
            return true;
        }
        match address {
            0xFF4C => true, // KEY0: boot-locked, ignore
            0xFF4D => {
                self.key1_armed = value & 0x01 != 0;
                true
            }
            0xFF6C => {
                self.opri = value & 0x01 != 0;
                true
            }
            0xFF70 => {
                self.svbk = value & 0x07;
                true
            }
            _ => false,
        }
    }
}

/// The Game Boy Color.
pub type GameBoyColor = Console<Cgb>;
