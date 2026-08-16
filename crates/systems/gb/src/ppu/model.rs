//! The PPU's per-console hardware divergence — the catalogue of how the CGB
//! colour PPU differs from the DMG monochrome PPU. Everything not named here is
//! the same silicon, living in the shared `Ppu`/`Rendering` pipeline.
//!
//! `Ppu<P: PpuModel>` is parameterised over this seam the same way
//! `Console<M: Model>` is — the DMG and CGB pipelines monomorphise to distinct,
//! conditional-free code, and the CGB colour hardware (CRAM, attributes, the
//! colour resolve) lives in `missingno-gbc`'s impl rather than behind a flag.

use super::TracePixel;
use super::crossing::CaptureSpec;
use super::draw::shifters::ObjShifter;
use super::memory::{Vram, VramBank};
use super::registers::{PipelineRegisters, TileSelGlitch};
use super::stat_interrupt::StatShadow;
use super::types::palette::{PaletteIndex, PaletteMap};
use super::types::sprites::{self, ObjAttr};

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

/// One decoded background-shifter stage for inspection: the 2-bit colour number
/// and, on the CGB, the tile's BG palette index (0-7); DMG leaves the palette 0.
#[derive(Clone, Copy, Debug)]
pub struct BgFifoCell {
    pub color: u8,
    pub palette: u8,
}

/// One decoded object-FIFO stage for inspection: the 2-bit colour (0 =
/// transparent), the palette selector (DMG OBP0/OBP1 = 0/1; CGB OBP0-7), and the
/// BG-over-OBJ priority bit.
#[derive(Clone, Copy, Debug)]
pub struct ObjFifoCell {
    pub color: u8,
    pub palette: u8,
    pub priority: u8,
}

/// Decode a packed object-FIFO into its 8 stages, MSB-first (cell 0 = the next
/// pixel to pop). The palette planes differ per console, so each model supplies
/// its own; the colour and priority packing is shared.
pub fn obj_fifo_cells_from(low: u8, high: u8, palette: [u8; 3], priority: u8) -> [ObjFifoCell; 8] {
    std::array::from_fn(|i| {
        let bit = 7 - i as u8;
        let color = (((high >> bit) & 1) << 1) | ((low >> bit) & 1);
        let pal = (0..3).fold(0, |acc, p| acc | (((palette[p] >> bit) & 1) << p));
        ObjFifoCell {
            color,
            palette: pal,
            priority: (priority >> bit) & 1,
        }
    })
}

/// Cartridge header bytes the boot-ROM handoff HLE consults: the CGB flag, and
/// the title + licensee a CGB hashes to pick a DMG-compatibility palette.
pub struct CartridgeBootHeader {
    pub is_cgb: bool,
    /// $0134-$0143.
    pub title: [u8; 16],
    /// $014B.
    pub old_licensee: u8,
    /// $0144-$0145.
    pub new_licensee: [u8; 2],
}

/// The hardware that differs between the DMG and CGB PPUs. The shared pipeline
/// resolves a pixel by calling [`PpuModel::resolve`]; the result is the final
/// framebuffer pixel for that console.
pub trait PpuModel: Default {
    /// The DMG window-X comparator (NUKO) drives the PANY BG drain-detector slip
    /// whenever the window is armed (REJO), even with WIN_EN off — an
    /// armed-but-disabled 1-dot BG slip. The CGB suppresses that coupling: its
    /// NUKO→PANY path requires the window enabled; the enabled-window slip is
    /// unaffected on both.
    const WINDOW_DRAIN_SLIP_WHILE_DISABLED: bool = true;

    /// The DMG "STAT write" glitch: a write to STAT ($FF41) momentarily drives every
    /// mode/LYC source-enable high, so the write can raise the STAT line even when no
    /// enabled condition is actually met. The CGB fixed this — its STAT write
    /// re-evaluates the line with the written enables only (a matching-mode write can
    /// still raise it).
    const STAT_WRITE_ALL_ENABLES_GLITCH: bool = true;

    /// DMG gate AROR ANDs OBJ-enable (XYLO) into the FEPO sprite-fetch trigger, so LCDC.1=0
    /// suppresses the fetch and its mode-3 penalty entirely. The CGB drops OBJ-enable from the
    /// trigger — the fetch (and its penalty dots) run regardless; the enable bit is consumed only
    /// at the pixel pop (BG-vs-OBJ resolve), so OBJ-off lines still pay the sprite penalty without
    /// drawing the sprite.
    const FETCH_TRIGGER_GATED_BY_OBJ_ENABLE: bool = true;

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

    /// The FF41/FF45 → STAT-IRQ-block synchroniser: CGB holds DFF copies of the
    /// enables and LYC cells (the CGB's `SyncedStatCells`), DMG feeds the block
    /// combinationally and carries a ZST `()`.
    type StatShadow: StatShadow + Default;

    fn stat_shadow(&self) -> &Self::StatShadow;
    fn stat_shadow_mut(&mut self) -> &mut Self::StatShadow;

    /// The mid-Mode-3 LCDC.4-clear TILE_SEL reset glitch cell: CGB substitutes
    /// the tile index byte into the bitplane read on the crossing-capture dot,
    /// DMG carries a ZST `()`.
    type TileSelGlitch: TileSelGlitch + Default;

    fn tile_sel_glitch(&self) -> &Self::TileSelGlitch;
    fn tile_sel_glitch_mut(&mut self) -> &mut Self::TileSelGlitch;

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

    /// morepork shift-register state: (lo, hi, palette, priority).
    fn obj_trace(fifo: &Self::ObjFifo) -> (u8, u8, u8, u8);

    /// The object FIFO decoded into its 8 stages for the debugger, MSB-first
    /// (cell 0 = the next pixel to pop). DMG carries a 1-bit OBP select; the CGB
    /// a 3-bit OBP index.
    fn obj_fifo_cells(fifo: &Self::ObjFifo) -> [ObjFifoCell; 8];

    /// The BG tile's palette index riding the shifter (CGB attribute, 0-7); the
    /// DMG BG has no palette selector.
    fn bg_cell_palette(_cell: Self::BgCell) -> u8 {
        0
    }

    /// OPRI ($FF6C): object-priority mode. DMG has no such register.
    fn object_priority_register(&self) -> u8 {
        0xFF
    }
    fn set_object_priority_register(&mut self, _value: u8) {}

    /// Post-boot cartridge configuration (HLE of the boot ROM's handoff state).
    /// The CGB enters DMG-compatibility mode — installing a boot compat palette
    /// into CRAM (selected from the title checksum) and routing the DMG palette
    /// registers through it — when a DMG cartridge is inserted. DMG hardware:
    /// nothing to configure.
    fn init_post_boot(&mut self, _header: &CartridgeBootHeader) {}

    /// Resolve the BG/OBJ mux to a final framebuffer pixel. Palette state and
    /// LCDC are read live from `regs`.
    fn resolve(&self, mux: &PixelMux<Self::BgCell>, regs: &PipelineRegisters) -> Self::Pixel;

    /// This pixel as the morepork pixel stream records it — a shade on DMG, an
    /// RGB555 colour on CGB (matching the trace's declared `pix_format`).
    fn trace_pixel(pixel: Self::Pixel) -> TracePixel;

    /// The model has synchronisers capturing on the CPU-clock M-cycle
    /// boundary (CGB): the palette block's mode-3 sample on the boundary
    /// rise, and the FF41/FF45 register file crossing into the STAT-IRQ
    /// block on the boundary fall. DMG couples its registers combinationally.
    const HAS_CLOCK_DOMAIN_SYNC: bool = false;

    /// The CGB window-hit latch is enable-qualified: the (synced) enable
    /// dropping releases RYDY immediately, truncating the mode-3 extension
    /// at that dot. The DMG's RYDY holds to PORY.
    const ENABLE_QUALIFIED_WINDOW_HIT: bool = false;

    /// The CGB right-edge window restart is observable: RYDY masks WEGO's
    /// XYMU clear until PORY completes the restarted fetch — mode 3 and the
    /// OAM/VRAM locks run long — while the mode-0 STAT leg follows XUGU from
    /// the restart to line end, unmasked by terminal sprite fetches. The DMG
    /// clears XYMU unconditionally — its right-edge cascade is
    /// observationally inert.
    const WINDOW_RESTART_MASKS_MODE3_END: bool = false;

    /// The CGB's revised OAM lock logic (the family that also removed the
    /// OAM corruption bug): the write lock equals the read lock — it carries
    /// the RUTU-pending term and has no AJUJ write-permit pulse. The DMG
    /// keeps both artifacts.
    const REVISED_OAM_LOCK: bool = false;

    /// The CGB TILE_SEL reset glitch: an LCDC.4 clear landing on a bitplane
    /// read's dot substitutes the fetched tile index byte as that bitplane's
    /// data (indices < 0x80 only — higher indices address identically in both
    /// modes). Absent on DMG and CGB revision D.
    const TILE_SEL_RESET_GLITCH: bool = false;

    /// The CGB TILE_SEL set glitch: an LCDC.4 set landing on the counter-2 low
    /// bitplane read substitutes the last bitplane-1 (high) byte driven onto the
    /// tile-data bus (the most recent sprite fetch, else the last BG tile) as the
    /// low plane's data. Absent on DMG.
    const TILE_SEL_SET_GLITCH: bool = false;

    /// The DMG BGP cell is a dlatch (NURA combiner): a capture-coincident
    /// cp_pad sample sees the post-write value, and a second same-scanline
    /// write presents OR(prior, new) for one emit. CGB rebuilt the block as a
    /// clean DFF — the coincident sample sees the pre-capture value and no OR
    /// transient exists.
    const BGP_WRITE_RACE: bool = true;

    /// The OBP0/OBP1 cells (WUFU/MOKA) share BGP's `dlatch_ee` silicon; on CGB they
    /// are clean DFFs, so a write-coincident object emit reads the pre-capture value.
    /// The DMG dlatch OR transient is unmodelled (no test exercises it).
    const OBP_WRITE_RACE: bool = true;

    /// The mid-Mode-3 SCY ($FF42) write → BG-fetch crossing. The DMG couples it
    /// combinationally (the write is immediate); the CGB latches it onto its own
    /// clock and the BG fetch samples it the descriptor's falls late (both the
    /// map-row and the two tile-data fine-Y reads see the delayed value). The
    /// non-zero CGB value is authored behind the `missingno-gbc` wall — the DMG
    /// core names only the combinational collapse.
    const SCY_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The FF45 (LYC) → STAT-IRQ-block crossing. The DMG feeds the comparator
    /// combinationally (the cell drives PALY directly); the CGB crosses the cell
    /// into the IRQ block on the resolved capture edge — pure (ii) clock phase,
    /// no (iv) register-path lag (`delayed_falls: 0`). DMG names only the
    /// combinational collapse.
    const LYC_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The FF43 (SCX) → fine-scroll-match (POHU) crossing. The DMG reads the
    /// cell live; the CGB crosses it into the pixel pipeline on the resolved
    /// capture edge — pure (ii) clock phase, no (iv) register-path lag
    /// (`delayed_falls: 0`). DMG names only the combinational collapse.
    const SCX_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The window register file (WY/WX/LCDC.5/LCDC.2) → window-decode + scan
    /// Y-comparator crossing. The DMG reads the cells live; the CGB crosses them
    /// into the pixel pipeline on the resolved capture edge — pure (ii) clock
    /// phase, no (iv) register-path lag (`delayed_falls: 0`). DMG names only
    /// the combinational collapse.
    const WINDOW_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The FF41 (STAT enables) → STAT-IRQ-block crossing. The DMG feeds the SUKO
    /// legs combinationally (no register edge can occur inside a TALU
    /// evaluation); the CGB crosses the enables cell into the IRQ block on the
    /// resolved capture edge — the M-boundary fall — where the resulting
    /// register-path edges race that fall's condition edges in the SUKO
    /// waveform. Pure (ii) clock phase, no (iv) register-path lag
    /// (`delayed_falls: 0`); the intra-evaluation arrival is the separate
    /// `REGISTER_PATH_ARRIVAL_PS` waveform constant. DMG names only the
    /// combinational collapse.
    const STAT_ENABLES_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The mid-Mode-3 LCDC tile-map-select (LCDC.3/.6) write → BG-fetch crossing.
    /// The DMG couples it combinationally (the fetch reads LCDC live); the CGB
    /// latches the write onto its own clock and the fetch samples the select bit
    /// the descriptor's falls late — the documented CGB resync lag translating the
    /// OLD/NEW-map boundary. The non-zero CGB value is authored behind the
    /// `missingno-gbc` wall — the DMG core names only the combinational collapse.
    const TILE_MAP_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The mid-Mode-3 LCDC.4 (TILE_SEL/WEXU) write → BG tile-data fetch crossing.
    /// The DMG couples it combinationally (the fetch reads LCDC live); the CGB
    /// latches the write onto its own clock and the tile-data fetch samples the
    /// select bit the descriptor's falls late — the same resync as the LCDC.3/.6
    /// tile-map siblings. DMG names only the combinational collapse.
    const TILE_DATA_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The mid-Mode-3 LCDC.0 (VYXE) write → BG-plane-blank (RAJY) crossing. The
    /// DMG couples it combinationally; the CGB synchronises the write onto its
    /// own clock, so the OLD-overlay holds the pre-write value the crossing's
    /// extra falls longer (RAJY lands that many dots later). DMG names only the
    /// combinational collapse.
    const BG_ENABLE_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The mid-Mode-3 LCDC.1 (XYLO) write → OBJ-mux crossing. Combinational by
    /// default (DMG); the CGB overrides it with a register-path lag so the
    /// OBJ-disable reaches the mux a crossing late.
    const OBJ_ENABLE_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The mid-Mode-3 LCDC.2 (OBJ size) write → sprite-fetch crossing.
    /// Combinational by default (DMG reads obj-size live); the CGB overrides it
    /// so the size reaches the fetch's two tile-data reads the crossing's falls
    /// late, splitting an 8x8↔8x16 change across the low/high bitplanes.
    const OBJ_SIZE_CROSSING: CaptureSpec = CaptureSpec::COMBINATIONAL;

    /// The CPU's view of the VRAM lock. The DMG CPU sees XYMU
    /// combinationally; the CGB arbiter samples it in the M-cycle clock
    /// domain — the same captured sample as the CRAM lock.
    fn vram_cpu_lock(&self, live: bool) -> bool {
        live
    }

    /// M-cycle-boundary capture: the model's clock-domain synchronisers sample
    /// their inputs. `drawing` is the live mode-3 latch (XYMU view) the CGB VRAM
    /// arbiter samples; `palette_drawing` is XYMU through the dot-fall stage,
    /// what the CGB palette block locks CRAM on (a boundary-coincident stage
    /// capture is not yet visible there). The one CGB synchroniser on a
    /// different edge is the halt-wake comparator presample (T2 rise,
    /// `Model::halt_wake_samples_early`).
    fn tick_clock_domain(&mut self, _drawing: bool, _palette_drawing: bool) {}
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
                regs.palettes.sprite0_for_resolve()
            } else {
                regs.palettes.sprite1_for_resolve()
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
pub struct DmgPpu {
    /// The STAT-IRQ block reads the cells combinationally — the synchroniser is
    /// a ZST.
    stat_shadow: (),
    /// No TILE_SEL reset glitch on DMG silicon — a ZST.
    tile_sel_glitch: (),
}

impl PpuModel for DmgPpu {
    type Vram = VramBank;
    type BgCell = ();
    type Pixel = PaletteIndex;

    type StatShadow = ();

    fn stat_shadow(&self) -> &() {
        &self.stat_shadow
    }
    fn stat_shadow_mut(&mut self) -> &mut () {
        &mut self.stat_shadow
    }

    type TileSelGlitch = ();

    fn tile_sel_glitch(&self) -> &() {
        &self.tile_sel_glitch
    }
    fn tile_sel_glitch_mut(&mut self) -> &mut () {
        &mut self.tile_sel_glitch
    }

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

    fn obj_fifo_cells(fifo: &ObjShifter) -> [ObjFifoCell; 8] {
        let (low, high, palette, priority) = fifo.registers();
        obj_fifo_cells_from(low, high, [palette, 0, 0], priority)
    }

    fn resolve(&self, mux: &PixelMux<()>, regs: &PipelineRegisters) -> PaletteIndex {
        PaletteIndex(resolve_shade(mux, regs))
    }

    fn trace_pixel(pixel: PaletteIndex) -> TracePixel {
        TracePixel::Shade(pixel.0)
    }
}

#[cfg(test)]
mod tests {
    use super::obj_fifo_cells_from;

    #[test]
    fn obj_fifo_decode_is_msb_first() {
        // low = 0b1000_0000, high = 0 → the MSB stage (cell 0) has colour 1.
        let cells = obj_fifo_cells_from(0b1000_0000, 0, [0, 0, 0], 0);
        assert_eq!(cells[0].color, 1);
        assert!(cells[1..].iter().all(|c| c.color == 0));
    }

    #[test]
    fn obj_fifo_decode_packs_colour_palette_priority() {
        // Stage 0 (bit 7): colour 3 (both planes), palette 0b101 = 5, priority 1.
        let cells = obj_fifo_cells_from(
            0b1000_0000,
            0b1000_0000,
            [0b1000_0000, 0, 0b1000_0000],
            0b1000_0000,
        );
        assert_eq!(cells[0].color, 3);
        assert_eq!(cells[0].palette, 5);
        assert_eq!(cells[0].priority, 1);
    }
}
