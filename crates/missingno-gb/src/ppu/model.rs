//! The PPU's per-console hardware divergence — the catalogue of how the CGB
//! colour PPU differs from the DMG monochrome PPU. Everything not named here is
//! the same silicon, living in the shared `Ppu`/`Rendering` pipeline.
//!
//! `Ppu<P: PpuModel>` is parameterised over this seam the same way
//! `Console<M: Model>` is — the DMG and CGB pipelines monomorphise to distinct,
//! conditional-free code, and the CGB colour hardware (CRAM, attributes, the
//! colour resolve) lives in `missingno-gbc`'s impl rather than behind a flag.

use super::draw::shifters::ObjShifter;
use super::memory::{Vram, VramBank};
use super::registers::PipelineRegisters;
use super::types::palette::{PaletteIndex, PaletteMap};
use super::types::sprites::{self, ObjAttr};

/// A CGB colour-palette RAM port. BCPS/BCPD ($FF68/9) address BG palettes;
/// OCPS/OCPD ($FF6A/B) address OBJ palettes. Index ports are always accessible;
/// data ports are blocked while the PPU renders (mode 3).
#[derive(Clone, Copy, Debug)]
pub enum ColorRegister {
    BackgroundIndex,
    BackgroundData,
    ObjectIndex,
    ObjectData,
}

/// The BG/OBJ shifter outputs feeding the pixel mux on a given dot. `bg_cell`
/// is the per-tile BG data riding the shifter beyond the two bitplanes — `()`
/// on the DMG, the BG map attribute on the CGB (palette / priority / bank).
pub struct PixelMux<C> {
    pub bg_lo: u8,
    pub bg_hi: u8,
    pub bg_cell: C,
    pub spr_lo: u8,
    pub spr_hi: u8,
    pub spr_pal: u8,
    pub spr_pri: u8,
}

/// The hardware that differs between the DMG and CGB PPUs. The shared pipeline
/// resolves a pixel by calling [`PpuModel::resolve`]; the result is the final
/// framebuffer pixel for that console.
pub trait PpuModel: Default {
    /// The DMG window-X comparator (NUKO) drives the §6.1 PANY BG drain-detector
    /// slip whenever the window is armed (REJO), even with WIN_EN off — an
    /// armed-but-disabled 1-dot BG slip. The CGB suppresses that coupling: its
    /// NUKO→PANY path requires the window enabled. (Confirmed by dmg-sim; the
    /// enabled-window slip is unaffected on both.)
    const WINDOW_DRAIN_SLIP_WHILE_DISABLED: bool = true;

    /// This console's video RAM: DMG one bank, CGB two (VBK-banked).
    type Vram: Vram;

    /// Per-tile BG data riding the shifter beyond the two bitplanes: `()` on the
    /// DMG (the BG map has no attribute), the BG map attribute byte on the CGB.
    type BgCell: Copy + Default;

    /// The framebuffer pixel this PPU emits — DMG a 2-bit shade index, CGB RGB555.
    type Pixel: Copy;

    /// Read the BG map attribute for a tile-map cell. The CGB attribute lives in
    /// VRAM bank 1 at the same offset as the bank-0 tile index. DMG: `()`.
    fn bg_attribute(vram: &Self::Vram, map_offset: u16) -> Self::BgCell;

    /// VRAM bank + fine-Y row for a BG tile-data read. The CGB applies the
    /// attribute's bank-select (bit 3) and Y-flip (bit 6); DMG: bank 0, row as-is.
    fn bg_tile_source(cell: Self::BgCell, fine_y: u8) -> (u8, u8);

    /// X-flip the loaded BG bitplanes (CGB attribute bit 5); DMG: unchanged.
    fn flip_bg_planes(cell: Self::BgCell, low: u8, high: u8) -> (u8, u8);

    /// VRAM bank for an object's tile-data read (CGB OAM attr bit 3). Instance
    /// method: DMG-compatibility mode reinterprets the OAM attribute byte
    /// DMG-style, so bit 3 is not a bank-select there — objects stay in bank 0.
    /// DMG: 0.
    fn obj_data_bank(&self, _attrs: sprites::Attributes) -> u8 {
        0
    }

    /// The per-pixel OBJ attribute carried on the sprite shifter. Instance
    /// method: the CGB reads its 3-bit palette (OAM bits 0-2) in full-CGB mode
    /// but the DMG OBP-select (bit 4) in DMG-compatibility mode.
    fn obj_attr(&self, attrs: sprites::Attributes) -> ObjAttr;

    /// The console's object FIFO. The DMG resolves overlaps by fetch order with a
    /// 1-bit OBP-select; the CGB resolves by OAM index with a 3-bit palette. The
    /// whole FIFO is opaque to the shared pipeline — only the neutral operations
    /// below cross the seam.
    type ObjFifo: Default;

    /// SACU shift toward the LCD.
    fn obj_shift(fifo: &mut Self::ObjFifo);

    /// WUTY load of a fetched sprite's 8 pixels (transparency-gated). `slot` is the
    /// sprite's OAM-scan store index — its identity, which the CGB ranks priority by.
    fn obj_merge(&self, fifo: &mut Self::ObjFifo, low: u8, high: u8, attr: ObjAttr, slot: u8);

    /// Stage-7 Q output for the pixel mux: (lo, hi, palette, priority).
    fn obj_pixel(fifo: &Self::ObjFifo) -> (u8, u8, u8, u8);

    /// gbtrace shift-register state: (lo, hi, palette, priority).
    fn obj_trace(fifo: &Self::ObjFifo) -> (u8, u8, u8, u8);

    /// OPRI ($FF6C): object-priority mode. DMG has no such register.
    fn object_priority_register(&self) -> u8 {
        0xFF
    }
    fn set_object_priority_register(&mut self, _value: u8) {}

    /// Post-boot cartridge configuration (HLE of the boot ROM's handoff state).
    /// The CGB enters DMG-compatibility mode — installing the boot compat
    /// palette into CRAM and routing the DMG palette registers through it — when
    /// a DMG cartridge is inserted. DMG hardware: nothing to configure.
    fn init_post_boot(&mut self, _cartridge_is_cgb: bool) {}

    /// Resolve the BG/OBJ mux to a final framebuffer pixel. Palette state and
    /// LCDC are read live from `regs`.
    fn resolve(&self, mux: &PixelMux<Self::BgCell>, regs: &PipelineRegisters) -> Self::Pixel;

    /// The 2-bit shade a gbtrace pixel stream records for this pixel.
    fn trace_shade(pixel: Self::Pixel) -> u8;

    /// Read a CGB colour-palette register. `rendering` is true in mode 3, when
    /// the data ports are locked. DMG has no colour RAM — reads 0xFF.
    fn read_color_register(&self, _reg: ColorRegister, _rendering: bool) -> u8 {
        0xFF
    }

    /// Write a CGB colour-palette register. DMG has no colour RAM — ignored.
    fn write_color_register(&mut self, _reg: ColorRegister, _value: u8, _rendering: bool) {}
}

/// Which layer wins the shared DMG BG-vs-OBJ resolve, carrying its BGP/OBP-mapped
/// 2-bit shade. The DMG screen stores `shade` directly; the CGB DMG-compatibility
/// path indexes the winning layer's CRAM palette by it (OBJ uses `palette` as the
/// OBP0/OBP1 slot). (XULA/WOXA → NULY → POKA priority.)
pub enum DmgPixel {
    Background { shade: u8 },
    Object { palette: u8, shade: u8 },
}

/// Shared DMG pixel resolve: BG-vs-OBJ priority + the BGP/OBP shade map.
pub fn resolve_dmg_pixel<C>(mux: &PixelMux<C>, regs: &PipelineRegisters) -> DmgPixel {
    let bg_color = if regs.bg_window_enabled_for_resolve() {
        (mux.bg_hi << 1) | mux.bg_lo
    } else {
        0
    };

    if regs.sprites_enabled_for_resolve() {
        let spr_color = (mux.spr_hi << 1) | mux.spr_lo;
        if spr_color != 0 && (mux.spr_pri == 0 || bg_color == 0) {
            let obp = if mux.spr_pal == 0 {
                regs.palettes.sprite0.output()
            } else {
                regs.palettes.sprite1.output()
            };
            return DmgPixel::Object {
                palette: mux.spr_pal,
                shade: PaletteMap(obp).map(PaletteIndex(spr_color)).0,
            };
        }
    }

    DmgPixel::Background {
        shade: PaletteMap(regs.palettes.background_for_bg_resolve())
            .map(PaletteIndex(bg_color))
            .0,
    }
}

/// The DMG screen's 2-bit shade for this mux (the winning layer's mapped colour).
pub fn resolve_shade<C>(mux: &PixelMux<C>, regs: &PipelineRegisters) -> u8 {
    match resolve_dmg_pixel(mux, regs) {
        DmgPixel::Background { shade } | DmgPixel::Object { shade, .. } => shade,
    }
}

/// The original Game Boy PPU: a 2-bit shade per pixel, no colour memory.
#[derive(Default)]
pub struct DmgPpu;

impl PpuModel for DmgPpu {
    type Vram = VramBank;
    type BgCell = ();
    type Pixel = PaletteIndex;

    fn bg_attribute(_vram: &VramBank, _map_offset: u16) {}

    fn bg_tile_source(_cell: (), fine_y: u8) -> (u8, u8) {
        (0, fine_y)
    }

    fn flip_bg_planes(_cell: (), low: u8, high: u8) -> (u8, u8) {
        (low, high)
    }

    fn obj_attr(&self, attrs: sprites::Attributes) -> ObjAttr {
        ObjAttr {
            palette: attrs.dmg_palette(),
            priority: attrs.behind_background(),
        }
    }

    type ObjFifo = ObjShifter;

    fn obj_shift(fifo: &mut ObjShifter) {
        fifo.shift();
    }

    fn obj_merge(&self, fifo: &mut ObjShifter, low: u8, high: u8, attr: ObjAttr, _slot: u8) {
        fifo.merge(low, high, attr.palette, attr.priority as u8);
    }

    fn obj_pixel(fifo: &ObjShifter) -> (u8, u8, u8, u8) {
        fifo.pixel()
    }

    fn obj_trace(fifo: &ObjShifter) -> (u8, u8, u8, u8) {
        fifo.registers()
    }

    fn resolve(&self, mux: &PixelMux<()>, regs: &PipelineRegisters) -> PaletteIndex {
        PaletteIndex(resolve_shade(mux, regs))
    }

    fn trace_shade(pixel: PaletteIndex) -> u8 {
        pixel.0
    }
}
