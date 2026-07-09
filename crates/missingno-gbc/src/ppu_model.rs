use missingno_gb::ppu::memory::Vram;
use missingno_gb::ppu::types::sprites::{Attributes, ObjAttr};
use missingno_gb::ppu::{
    CaptureSpec, CartridgeBootHeader, DmgPixel, InterruptFlags, PipelineRegisters, PixelMux,
    PpuModel, StatShadow, TileSelGlitch, TracePixel, resolve_dmg_pixel,
};

use crate::compat_palette::dmg_compat_palettes;
use crate::cram::{ColorRam, ColorRegister};
use crate::obj_fifo::CgbObjShifter;
use crate::screen::Color555;
use crate::vram::{BgAttribute, CgbVram};

/// The CGB FF41/FF45 synchroniser DFFs feeding the STAT-IRQ block.
#[derive(Default)]
pub struct SyncedStatCells {
    enables: InterruptFlags,
    lyc: u8,
}

impl StatShadow for SyncedStatCells {
    fn synced_enables(&self) -> InterruptFlags {
        self.enables
    }
    fn set_synced_enables(&mut self, value: InterruptFlags) {
        self.enables = value;
    }
    fn synced_lyc(&self, _cell: u8) -> u8 {
        self.lyc
    }
    fn set_synced_lyc(&mut self, value: u8) {
        self.lyc = value;
    }
}

/// The CGB colour PPU. Holds the BG/OBJ colour-palette RAM and the object FIFO;
/// the BG layer resolves through the BG attribute + BG palette RAM to RGB555 and
/// objects through OBJ palette RAM.
///
/// `dmg_compat` marks a DMG cartridge running on the CGB: the boot palette is
/// installed in CRAM and the DMG palette registers (BGP/OBP) index it. `opri`
/// is OPRI ($FF6C): false = CGB object priority (by OAM index), true = DMG (by X).
#[derive(Default)]
pub struct CgbPpu {
    bg_cram: ColorRam,
    obj_cram: ColorRam,
    dmg_compat: bool,
    opri: bool,
    /// XYMU sampled at the M-cycle (CPU-clock) boundary — the VRAM CPU
    /// arbiter's slow-domain view of drawing.
    drawing_synced: bool,
    /// XYMU sampled into the palette block's own 4-dot (VENA) clock — the
    /// CRAM data-port lock. Tracks `drawing_synced` at single speed; lags it
    /// by sampling half as often at double speed, where the palette clock is
    /// unchanged while the CPU M-cycle runs at 2×.
    palette_drawing_synced: bool,
    /// FF41/FF45 → STAT-IRQ-block synchroniser DFFs.
    stat_shadow: SyncedStatCells,
    /// The mid-Mode-3 LCDC.4-clear TILE_SEL reset glitch cell.
    tile_sel_glitch: TileSelResetGlitch,
}

/// The CGB TILE_SEL reset glitch cell: an LCDC.4-clearing write reaches the
/// tile-data addressing at the crossing-capture dot, so a bitplane read on that
/// dot returns the tile index byte instead of VRAM data. Live for one dot.
#[derive(Default)]
pub struct TileSelResetGlitch {
    pending: bool,
    active: bool,
}

impl TileSelGlitch for TileSelResetGlitch {
    fn arm(&mut self) {
        self.pending = true;
    }
    fn tick(&mut self) {
        self.active = self.pending;
        self.pending = false;
    }
    fn active(&self) -> bool {
        self.active
    }
    fn clear(&mut self) {
        self.pending = false;
        self.active = false;
    }
}

impl PpuModel for CgbPpu {
    // The CGB suppresses the DMG armed-but-disabled window-X → BG drain-detector
    // slip (its NUKO→PANY coupling requires the window enabled).
    const WINDOW_DRAIN_SLIP_WHILE_DISABLED: bool = false;

    // The CGB fixed the DMG STAT-write glitch — a STAT write re-evaluates with the
    // written enables only, never all-enables-high.
    const STAT_WRITE_ALL_ENABLES_GLITCH: bool = false;

    // The CGB drops OBJ-enable (XYLO/AROR) from the FEPO sprite-fetch trigger: on-line
    // sprites are fetched and pay their mode-3 penalty even with LCDC.1 off, with the enable
    // bit consumed only at the pixel pop (resolve).
    const FETCH_TRIGGER_GATED_BY_OBJ_ENABLE: bool = false;
    const HAS_CLOCK_DOMAIN_SYNC: bool = true;
    const ENABLE_QUALIFIED_WINDOW_HIT: bool = true;
    const WINDOW_RESTART_MASKS_MODE3_END: bool = true;
    const REVISED_OAM_LOCK: bool = true;
    const TILE_SEL_RESET_GLITCH: bool = true;
    const TILE_SEL_SET_GLITCH: bool = true;
    const BGP_WRITE_RACE: bool = false;
    const OBP_WRITE_RACE: bool = false;
    const SCY_CROSSING: CaptureSpec = crate::timing::SCY_CROSSING;
    const LYC_CROSSING: CaptureSpec = crate::timing::LYC_CROSSING;
    const SCX_CROSSING: CaptureSpec = crate::timing::SCX_CROSSING;
    const WINDOW_CROSSING: CaptureSpec = crate::timing::WINDOW_CROSSING;
    const STAT_ENABLES_CROSSING: CaptureSpec = crate::timing::STAT_ENABLES_CROSSING;
    const TILE_MAP_CROSSING: CaptureSpec = crate::timing::TILE_MAP_CROSSING;
    const TILE_DATA_CROSSING: CaptureSpec = crate::timing::TILE_DATA_CROSSING;
    const BG_ENABLE_CROSSING: CaptureSpec = crate::timing::BG_ENABLE_CROSSING;
    const OBJ_ENABLE_CROSSING: CaptureSpec = crate::timing::OBJ_ENABLE_CROSSING;
    const OBJ_SIZE_CROSSING: CaptureSpec = crate::timing::OBJ_SIZE_CROSSING;

    type Vram = CgbVram;
    type BgCell = BgAttribute;
    type Pixel = Color555;

    type StatShadow = SyncedStatCells;

    fn stat_shadow(&self) -> &SyncedStatCells {
        &self.stat_shadow
    }
    fn stat_shadow_mut(&mut self) -> &mut SyncedStatCells {
        &mut self.stat_shadow
    }

    type TileSelGlitch = TileSelResetGlitch;

    fn tile_sel_glitch(&self) -> &TileSelResetGlitch {
        &self.tile_sel_glitch
    }
    fn tile_sel_glitch_mut(&mut self) -> &mut TileSelResetGlitch {
        &mut self.tile_sel_glitch
    }

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

    type ObjFifo = CgbObjShifter;

    fn obj_shift(fifo: &mut CgbObjShifter) {
        fifo.shift();
    }

    fn obj_merge(&self, fifo: &mut CgbObjShifter, low: u8, high: u8, attr: ObjAttr, slot: u8) {
        // CGB object priority (OPRI=0) resolves overlaps by OAM index; DMG-style
        // (OPRI=1, and DMG-compat) resolves by fetch order.
        fifo.merge(
            low,
            high,
            attr.palette,
            attr.priority as u8,
            slot,
            !self.opri,
        );
    }

    fn obj_pixel(fifo: &CgbObjShifter) -> (u8, u8, u8, u8) {
        fifo.pixel()
    }

    fn obj_trace(fifo: &CgbObjShifter) -> (u8, u8, u8, u8) {
        fifo.registers()
    }

    fn object_priority_register(&self) -> u8 {
        0xFE | self.opri as u8
    }

    fn set_object_priority_register(&mut self, value: u8) {
        self.opri = value & 0x01 != 0;
    }

    fn init_post_boot(&mut self, header: &CartridgeBootHeader) {
        // The CGB boot ROM fades all BG palettes to white before handoff.
        self.bg_cram.fill(Color555::grey(31));
        // The index registers hold whatever the boot ROM's last palette
        // stream left: cart-type-dependent, auto-increment on in both cases.
        if !header.is_cgb {
            self.dmg_compat = true;
            // The boot ROM selects DMG object priority (OPRI=1) for a DMG cart.
            self.opri = true;
            let (bg, obj0, obj1) =
                dmg_compat_palettes(&header.title, header.old_licensee, header.new_licensee);
            self.bg_cram.install(0, bg);
            self.obj_cram.install(0, obj0);
            self.obj_cram.install(1, obj1);
            self.bg_cram.write_index(0x88);
            self.obj_cram.write_index(0x90);
        } else {
            // For a CGB cart the final fade commit streams all 64 BG bytes
            // (index wraps to 0) and a single OBJ byte (index rests at 1).
            self.bg_cram.write_index(0x80);
            self.obj_cram.write_index(0x81);
        }
    }

    fn obj_data_bank(&self, attrs: Attributes) -> u8 {
        if self.dmg_compat {
            0
        } else {
            attrs.vram_bank()
        }
    }

    fn obj_attr(&self, attrs: Attributes) -> ObjAttr {
        ObjAttr {
            // DMG-compat objects select OBP0/OBP1 (bit 4); full-CGB select OBP0-7.
            palette: if self.dmg_compat {
                attrs.dmg_palette()
            } else {
                attrs.color_palette()
            },
            priority: attrs.behind_background(),
        }
    }

    fn resolve(&self, mux: &PixelMux<BgAttribute>, regs: &PipelineRegisters) -> Color555 {
        if self.dmg_compat {
            return self.resolve_dmg_compat(mux, regs);
        }

        let bg_index = (mux.bg_hi << 1) | mux.bg_lo;

        if regs.sprites_enabled_for_resolve() {
            let obj_index = (mux.spr_hi << 1) | mux.spr_lo;
            if obj_index != 0 {
                // CGB BG-vs-OBJ priority: LCDC.0 is the BG/Window master-priority
                // override (not a BG blank); BG-attr b7 and OAM b7 each (when set,
                // with LCDC.0) let a non-zero BG colour draw over the object.
                let master_priority = regs.bg_window_enabled_for_resolve();
                let bg_over_obj = mux.bg_cell.priority();
                let oam_behind = mux.spr_pri != 0;
                let obj_wins = bg_index == 0 || !master_priority || (!bg_over_obj && !oam_behind);
                if obj_wins {
                    return self.obj_cram.color(mux.spr_pal, obj_index);
                }
            }
        }

        // BG/Window: the CGB always draws the BG from its palette RAM.
        self.bg_cram.color(mux.bg_cell.palette(), bg_index)
    }

    fn tick_clock_domain(&mut self, drawing: bool, palette_drawing: bool) {
        self.drawing_synced = drawing;
        self.palette_drawing_synced = palette_drawing;
    }

    fn vram_cpu_lock(&self, live: bool) -> bool {
        // Slow set, fast clear: the lock asserts once the synced sample
        // confirms XYMU, and drops combinationally with it.
        live && self.drawing_synced
    }

    fn trace_pixel(pixel: Color555) -> TracePixel {
        TracePixel::Rgb555(pixel.0 & 0x7FFF)
    }
}

impl CgbPpu {
    /// Debug view of BG palette RAM: the RGB555 colour at (palette 0-7, index 0-3).
    pub fn bg_color(&self, palette: u8, index: u8) -> Color555 {
        self.bg_cram.color(palette, index)
    }

    /// Debug view of OBJ palette RAM: the RGB555 colour at (palette 0-7, index 0-3).
    pub fn obj_color(&self, palette: u8, index: u8) -> Color555 {
        self.obj_cram.color(palette, index)
    }

    /// DMG-compatibility resolve: DMG-style BG-vs-OBJ priority picks the winning
    /// pixel, then its DMG shade (BGP/OBP-mapped) indexes the boot palette held
    /// in CRAM — BG palette 0, OBJ palette OBP0/OBP1 slot.
    fn resolve_dmg_compat(
        &self,
        mux: &PixelMux<BgAttribute>,
        regs: &PipelineRegisters,
    ) -> Color555 {
        // The DMG resolve picks the layer + shade; DMG-compat indexes that layer's
        // boot palette in CRAM (OBJ palette = OBP0/OBP1 slot).
        match resolve_dmg_pixel(mux, regs) {
            DmgPixel::Object { palette, shade } => self.obj_cram.color(palette, shade),
            DmgPixel::Background { shade } => self.bg_cram.color(0, shade),
        }
    }

    /// BCPS/OCPS ($FF68/$FF6A) index registers for the debugger — the raw
    /// auto-increment flag and index the CPU reads back. Read-only.
    pub fn palette_index_registers(&self) -> (u8, u8) {
        (
            self.read_color_register(ColorRegister::BackgroundIndex),
            self.read_color_register(ColorRegister::ObjectIndex),
        )
    }

    /// CPU read of a CGB colour-palette register; the palette block's own
    /// clock-domain sample supplies the data-port mode-3 lock.
    pub(crate) fn read_color_register(&self, register: ColorRegister) -> u8 {
        // DMG-compat locks only the CRAM data port; the index registers
        // stay live (boot leftovers read back).
        if self.dmg_compat
            && matches!(
                register,
                ColorRegister::BackgroundData | ColorRegister::ObjectData
            )
        {
            return 0xFF;
        }
        self.read_cram_register(register, self.palette_drawing_synced)
    }

    /// CPU write of a CGB colour-palette register.
    pub(crate) fn write_color_register(&mut self, register: ColorRegister, value: u8) {
        if self.dmg_compat
            && matches!(
                register,
                ColorRegister::BackgroundData | ColorRegister::ObjectData
            )
        {
            return;
        }
        self.write_cram_register(register, value, self.palette_drawing_synced);
    }

    fn read_cram_register(&self, register: ColorRegister, rendering: bool) -> u8 {
        match register {
            ColorRegister::BackgroundIndex => self.bg_cram.read_index(),
            ColorRegister::ObjectIndex => self.obj_cram.read_index(),
            ColorRegister::BackgroundData if rendering => 0xFF,
            ColorRegister::ObjectData if rendering => 0xFF,
            ColorRegister::BackgroundData => self.bg_cram.read_data(),
            ColorRegister::ObjectData => self.obj_cram.read_data(),
        }
    }

    fn write_cram_register(&mut self, register: ColorRegister, value: u8, rendering: bool) {
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
