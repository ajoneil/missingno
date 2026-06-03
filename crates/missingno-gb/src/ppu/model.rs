//! The PPU's per-console hardware divergence — the catalogue of how the CGB
//! colour PPU differs from the DMG monochrome PPU. Everything not named here is
//! the same silicon, living in the shared `Ppu`/`Rendering` pipeline.
//!
//! `Ppu<P: PpuModel>` is parameterised over this seam the same way
//! `Console<M: Model>` is — the DMG and CGB pipelines monomorphise to distinct,
//! conditional-free code, and the CGB colour hardware (CRAM, attributes, the
//! colour resolve) lives in `missingno-gbc`'s impl rather than behind a flag.

use super::memory::{Vram, VramBank};
use super::registers::PipelineRegisters;
use super::types::palette::{PaletteIndex, PaletteMap};

/// The BG/OBJ shifter outputs feeding the pixel mux on a given dot.
pub struct PixelMux {
    pub bg_lo: u8,
    pub bg_hi: u8,
    pub spr_lo: u8,
    pub spr_hi: u8,
    pub spr_pal: u8,
    pub spr_pri: u8,
}

/// The hardware that differs between the DMG and CGB PPUs. The shared pipeline
/// resolves a pixel by calling [`PpuModel::resolve`]; the result is the final
/// framebuffer pixel for that console.
pub trait PpuModel: Default {
    /// This console's video RAM: DMG one bank, CGB two (VBK-banked).
    type Vram: Vram;

    /// The framebuffer pixel this PPU emits — DMG a 2-bit shade index, CGB RGB555.
    type Pixel: Copy;

    /// Resolve the BG/OBJ mux to a final framebuffer pixel. Palette state and
    /// LCDC are read live from `regs`.
    fn resolve(&self, mux: &PixelMux, regs: &PipelineRegisters) -> Self::Pixel;

    /// The 2-bit shade a gbtrace pixel stream records for this pixel.
    fn trace_shade(pixel: Self::Pixel) -> u8;
}

/// Shared BG/OBJ → shade mux: the BGP/OBP-mapped 2-bit colour. This is the value
/// the DMG screen stores directly, and the index the CGB greyscale fallback maps
/// while its colour pipeline is unbuilt. (XULA/WOXA → NULY → POKA priority.)
pub fn resolve_shade(mux: &PixelMux, regs: &PipelineRegisters) -> u8 {
    let bgp = regs.palettes.background_for_bg_resolve();
    let obp0 = regs.palettes.sprite0.output();
    let obp1 = regs.palettes.sprite1.output();
    let bg_window_enabled = regs.bg_window_enabled_for_resolve();
    let sprites_enabled = regs.sprites_enabled_for_resolve();

    let bg_color = if bg_window_enabled {
        (mux.bg_hi << 1) | mux.bg_lo
    } else {
        0
    };

    if sprites_enabled {
        let spr_color = (mux.spr_hi << 1) | mux.spr_lo;
        if spr_color != 0 && (mux.spr_pri == 0 || bg_color == 0) {
            let palette = if mux.spr_pal == 0 { obp0 } else { obp1 };
            return PaletteMap(palette).map(PaletteIndex(spr_color)).0;
        }
    }

    PaletteMap(bgp).map(PaletteIndex(bg_color)).0
}

/// The original Game Boy PPU: a 2-bit shade per pixel, no colour memory.
#[derive(Default)]
pub struct DmgPpu;

impl PpuModel for DmgPpu {
    type Vram = VramBank;
    type Pixel = PaletteIndex;

    fn resolve(&self, mux: &PixelMux, regs: &PipelineRegisters) -> PaletteIndex {
        PaletteIndex(resolve_shade(mux, regs))
    }

    fn trace_shade(pixel: PaletteIndex) -> u8 {
        pixel.0
    }
}
