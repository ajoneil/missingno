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

mod dmg_palette_data;

use missingno_gb::ppu::memory::{Vram, VramAddress, VramBank};
use missingno_gb::ppu::rendering::Mode;
use missingno_gb::ppu::types::sprites::{Attributes, ObjAttr};
use missingno_gb::ppu::{
    CartridgeBootHeader, ColorRegister, DmgPixel, DomainSamples, PipelineRegisters, PixelMux, Ppu,
    PpuModel, resolve_dmg_pixel,
};
use missingno_gb::{
    Console, Model, StopAction, VramDmaClaim, WaveRamCoupling, cartridge::Cartridge, cpu::Cpu,
    dma::Dma, joypad::Joypad, shared_oam_dma_write_conflict_byte, timers::Timers,
};

use crate::screen::{Color555, GREYSCALE, Screen};

/// CPU T-cycles the CPU stays `Stopped` during a double-speed switch (the
/// ~0x20000-T-cycle blackout). The divider and PPU run throughout; the CPU
/// re-engages at the new speed when this drains. Tuned against the age `spsw-*`
/// expected values.
const SPEED_SWITCH_BLACKOUT_TCYCLES: u32 = 0x2_0000;

/// Master edges of clock-mux relock tail after the 1×→2× hold: the dot clock
/// keeps stepping the PPU while the CPU clock is still settling, so the divider
/// stays quiet here (DIV is set by the hold alone) but the PPU advances — that
/// is the post-switch CPU↔dot re-phase.
const SWITCH_TO_DOUBLE_RELOCK_EDGES: u32 = 5;

/// Relock tail for the 2×→1× swap. The downward mux also settles to a phase;
/// it sets the CPU↔dot alignment the NEXT 1×→2× switch enters from, so over
/// repeated switches it determines whether the post-switch reads converge to
/// the single-switch alignment.
const SWITCH_TO_SINGLE_RELOCK_EDGES: u32 = 2;

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

    /// Write a 4-colour RGB555 palette into one of the 8 slots (the boot ROM
    /// installs the DMG-compatibility palette this way).
    fn install(&mut self, palette: usize, colours: [u16; 4]) {
        for (index, &colour) in colours.iter().enumerate() {
            let base = (palette * 4 + index) * 2;
            self.data[base] = colour as u8;
            self.data[base + 1] = (colour >> 8) as u8;
        }
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
    pub fn palette(self) -> u8 {
        self.0 & 0x07
    }

    pub fn tile_bank(self) -> u8 {
        (self.0 >> 3) & 0x01
    }

    pub fn flip_x(self) -> bool {
        self.0 & 0x20 != 0
    }

    pub fn flip_y(self) -> bool {
        self.0 & 0x40 != 0
    }

    /// BG-to-OBJ priority (bit 7): BG colour indices 1-3 of this tile draw over OBJ.
    pub fn priority(self) -> bool {
        self.0 & 0x80 != 0
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

/// The CGB boot ROM's default DMG-compatibility palette for a cartridge whose
/// title does not match the boot ROM table (palette combination 0): BG palette
/// 29, and OBJ palettes 0 and 1 both = palette 4. Little-endian RGB555.
pub const DMG_COMPAT_BG: [u16; 4] = [0x7FFF, 0x1BEF, 0x6180, 0x0000];
pub const DMG_COMPAT_OBJ: [u16; 4] = [0x7FFF, 0x421F, 0x1CF2, 0x0000];

/// Reverse-map a DMG-compatibility framebuffer colour to its DMG shade index
/// (0-3), for shade-pattern screenshot comparison. The compat palette is a
/// bijection over the four shades (white→0, BG green / OBJ pink →1, BG blue /
/// OBJ red →2, black→3), so the shade pattern is recoverable independent of the
/// tint. `None` for any off-palette colour.
pub fn dmg_compat_shade(color: Color555) -> Option<u8> {
    DMG_COMPAT_BG
        .iter()
        .chain(DMG_COMPAT_OBJ.iter())
        .position(|&c| Color555(c & 0x7FFF) == color)
        .map(|i| (i % 4) as u8)
}

/// The CGB boot ROM's DMG-compatibility palette selection: a Nintendo-licensee
/// gate, then the title checksum (with a 4th-letter tiebreak for collisions)
/// picks a palette combination. Returns the `(BG, OBJ0, OBJ1)` RGB555 palettes to
/// install in CRAM. A non-Nintendo or unmatched title falls to combination 0 —
/// the well-known green/blue-BG, pink/red-OBJ compatibility palette.
fn dmg_compat_palettes(
    title: &[u8; 16],
    old_licensee: u8,
    new_licensee: [u8; 2],
) -> ([u16; 4], [u16; 4], [u16; 4]) {
    use dmg_palette_data as data;

    let is_nintendo =
        old_licensee == 0x01 || (old_licensee == 0x33 && new_licensee == [b'0', b'1']);

    let mut combo = 0u8;
    if is_nintendo {
        let checksum = title.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        for i in 0..data::TITLE_CHECKSUMS.len() {
            // A collision-region match also has to agree on the 4th title letter,
            // otherwise the search continues.
            if data::TITLE_CHECKSUMS[i] == checksum
                && (i < data::FIRST_DUP_INDEX
                    || data::DUPS_4TH_LETTER[i - data::FIRST_DUP_INDEX] == title[3])
            {
                combo = data::PALETTE_PER_CHECKSUM[i] & 0x7F;
                break;
            }
        }
    }

    let [obj0, obj1, bg] = data::PALETTE_COMBINATIONS[combo as usize];
    (
        data::PALETTES[bg as usize],
        data::PALETTES[obj0 as usize],
        data::PALETTES[obj1 as usize],
    )
}

/// The CGB object FIFO: colour planes, a 3-bit palette (OBP0-7), priority, and a
/// per-pixel source slot (the OAM-scan store index = OAM-priority rank). When OPRI
/// selects CGB priority, a lower-slot object's pixel overwrites a higher one;
/// otherwise stages fill only when transparent (DMG fetch-order).
#[derive(Default)]
pub struct CgbObjShifter {
    low: u8,
    high: u8,
    palette: [u8; 3],
    priority: u8,
    slot: [u8; 8],
}

impl CgbObjShifter {
    fn shift(&mut self) {
        self.low <<= 1;
        self.high <<= 1;
        for plane in &mut self.palette {
            *plane <<= 1;
        }
        self.priority <<= 1;
        self.slot.copy_within(0..7, 1);
        self.slot[0] = 0;
    }

    fn pixel(&self) -> (u8, u8, u8, u8) {
        let lo = (self.low >> 7) & 1;
        let hi = (self.high >> 7) & 1;
        let pal = (0..3).fold(0, |acc, p| acc | (((self.palette[p] >> 7) & 1) << p));
        let pri = (self.priority >> 7) & 1;
        (lo, hi, pal, pri)
    }

    fn registers(&self) -> (u8, u8, u8, u8) {
        (self.low, self.high, self.palette[0], self.priority)
    }

    fn merge(
        &mut self,
        low: u8,
        high: u8,
        palette: u8,
        priority_bit: u8,
        slot: u8,
        by_index: bool,
    ) {
        for bit_pos in 0..8u8 {
            let lo = (low >> bit_pos) & 1;
            let hi = (high >> bit_pos) & 1;
            let color = (hi << 1) | lo;
            if color == 0 {
                continue;
            }

            let existing_lo = (self.low >> bit_pos) & 1;
            let existing_hi = (self.high >> bit_pos) & 1;
            let existing_color = (existing_hi << 1) | existing_lo;
            let pos = bit_pos as usize;
            if existing_color != 0 && !(by_index && slot < self.slot[pos]) {
                continue;
            }

            let mask = 1 << bit_pos;
            self.low = (self.low & !mask) | (lo << bit_pos);
            self.high = (self.high & !mask) | (hi << bit_pos);
            for (p, plane) in self.palette.iter_mut().enumerate() {
                *plane = (*plane & !mask) | (((palette >> p) & 1) << bit_pos);
            }
            self.priority = (self.priority & !mask) | (priority_bit << bit_pos);
            self.slot[pos] = slot;
        }
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
}

impl PpuModel for CgbPpu {
    // The CGB suppresses the DMG armed-but-disabled window-X → BG drain-detector
    // slip (its NUKO→PANY coupling requires the window enabled).
    const WINDOW_DRAIN_SLIP_WHILE_DISABLED: bool = false;

    // The CGB fixed the DMG STAT-write glitch — a STAT write re-evaluates with the
    // written enables only, never all-enables-high.
    const STAT_WRITE_ALL_ENABLES_GLITCH: bool = false;
    const HAS_CLOCK_DOMAIN_SYNC: bool = true;
    const ENABLE_QUALIFIED_WINDOW_HIT: bool = true;
    const WINDOW_RESTART_MASKS_MODE3_END: bool = true;
    const REVISED_OAM_LOCK: bool = true;
    const TILE_SEL_RESET_GLITCH: bool = true;
    const BGP_WRITE_RACE: bool = false;
    const OBP_WRITE_RACE: bool = false;
    const BG_FETCH_SCY_STAGE_EARLY: bool = true;

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
        if !header.is_cgb {
            self.dmg_compat = true;
            // The boot ROM selects DMG object priority (OPRI=1) for a DMG cart.
            self.opri = true;
            let (bg, obj0, obj1) =
                dmg_compat_palettes(&header.title, header.old_licensee, header.new_licensee);
            self.bg_cram.install(0, bg);
            self.obj_cram.install(0, obj0);
            self.obj_cram.install(1, obj1);
        }
        // The boot ROM's palette writes leave the CRAM index registers at
        // $C8/$D0 (auto-increment on).
        self.bg_cram.write_index(0xC8);
        self.obj_cram.write_index(0xD0);
    }

    fn obj_data_bank(&self, attrs: Attributes) -> u8 {
        if self.dmg_compat { 0 } else { attrs.cgb_bank() }
    }

    fn obj_attr(&self, attrs: Attributes) -> ObjAttr {
        ObjAttr {
            // DMG-compat objects select OBP0/OBP1 (bit 4); full-CGB select OBP0-7.
            palette: if self.dmg_compat {
                attrs.dmg_palette()
            } else {
                attrs.cgb_palette()
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

    fn tick_clock_domain(&mut self, samples: DomainSamples) {
        self.drawing_synced = samples.drawing;
        if samples.palette_capture {
            self.palette_drawing_synced = samples.drawing;
        }
    }

    fn vram_cpu_lock(&self, live: bool) -> bool {
        // Slow set, fast clear: the lock asserts once the synced sample
        // confirms XYMU, and drops combinationally with it.
        live && self.drawing_synced
    }

    fn read_color_register(&self, register: ColorRegister) -> u8 {
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

    fn write_color_register(&mut self, register: ColorRegister, value: u8) {
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

    fn trace_shade(pixel: Color555) -> u8 {
        // Greyscale fallback, then the DMG-compat boot palette (matching
        // `Screen::to_greyscale_bytes`); full-CGB colours have no 2-bit shade.
        GREYSCALE
            .iter()
            .position(|&grey| grey == pixel)
            .map(|i| i as u8)
            .or_else(|| dmg_compat_shade(pixel))
            .unwrap_or(0)
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

/// How the active VRAM DMA is paced. GDMA holds the CPU and flows continuously;
/// HDMA copies one 16-byte block per HBlank, releasing the CPU between blocks.
#[derive(Default, PartialEq)]
enum TransferMode {
    #[default]
    Idle,
    General,
    HBlank,
}

/// CGB VRAM DMA ($FF51-55) controller. The source and destination pointers run
/// as bytes are copied and persist after a transfer, so a follow-on transfer
/// continues where the last left off. The step loop ticks it each M-cycle: a
/// transfer flows `quota` bytes per M-cycle while it holds the CPU.
#[derive(Default)]
struct VramDma {
    /// Running source pointer, 16-byte aligned (HDMA1/HDMA2).
    source: u16,
    /// Running destination, a VRAM address $8000..=$9FF0 (HDMA3/HDMA4).
    dest: u16,
    mode: TransferMode,
    /// Bytes left in the whole transfer.
    remaining: u16,
    /// Bytes left in the current HBlank block (HBlank mode). The CPU is held
    /// while this is >0.
    block_remaining: u8,
    /// Bytes still movable this M-cycle (refilled per tick: 2 single, 1 double).
    quota: u8,
    /// This HBlank's block has been latched — one block per mode-0 period.
    hblank_block_taken: bool,
    /// Trigger pend stage: the previous fall's view showed armed ∧ mode 0;
    /// commits to a cancel-immune block one fall later.
    pend: bool,
    /// The pend formed on the fall of the FF55 arm commit itself — the arm
    /// strobe pre-loads the engine's working pointers, so no setup cell.
    pend_from_arm: bool,
    /// FF55 armed on this fall (set by the write path, consumed by the tick).
    armed_this_fall: bool,
    /// Leading no-data cells of the block: the engine loads its working
    /// pointers from the HDMA1-4 holding registers on a PPU-triggered block.
    setup_cells: u8,
    /// Dots until a committed block claims the bus (the transfer readies two
    /// dots after the commit).
    ready_in: u8,
    /// Falls since the halt gate rose: the taken-clear path runs one
    /// boundary-clocked synchronizer stage behind the gate, so a clear in
    /// flight at the halt latch (within its M-cycle, 4 falls) still lands;
    /// later clears wait for the resume.
    halted_falls: u8,
    /// The previous fall's mode view showed mode 0 — entry-edge detection for
    /// the IF-rise-to-resume window (only an entry pends there).
    prev_view_hblank: bool,
    /// Falls since the registered request entered the trigger's two-stage
    /// pipe: a token still inside (≤2 falls) at the IF-rise thaw commits
    /// there; an older token relaunches through the pipe — the one-fall
    /// penalty that decides the grant-vs-dispatch tie.
    pend_age: u8,
    /// A speed-switch cancel caught the engine mid-byte: that byte completes
    /// (pointers advance) without counting against the latched length.
    escape_byte: bool,
}

impl VramDma {
    /// Whether a byte may move this M-cycle: a GDMA runs while bytes remain; a
    /// latched HBlank block runs to completion regardless of the live `mode`
    /// (the block sequencer, once started, does not consult the arming flag).
    fn moving(&self) -> bool {
        (self.block_remaining > 0 && self.remaining > 0 && self.ready_in == 0)
            || (self.mode == TransferMode::General && self.remaining > 0)
    }
}

/// The Game Boy Color [`Model`]. Remaining CGB features (the color pixel
/// pipeline) attach here as they land.
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
    /// A DMG cartridge is running in compatibility mode (KEY0 bit 2). Read back
    /// from KEY0 ($FF4C) as $04.
    dmg_compat: bool,
    /// VRAM DMA ($FF51-55).
    vram_dma: VramDma,
    /// Remaining master edges of the double-speed switch blackout. The CPU
    /// clock is held (the dot clock / divider keep running off the master)
    /// until this drains, then the SM83 re-engages at the new speed and the
    /// dot-clock phase the count expired on. 0 = not switching.
    speed_switch_blackout: u32,
    /// Pre-ALET-rise XYMU (mode-3) state, sampled before this dot's ALET edge
    /// (where VOGA captures) — the pre-transition view a double-speed FF41 read's
    /// `data_phase_n↑` latch saw; `resolve_read_latch` resolves the read's STAT
    /// mode to it.
    pre_alet_rendering: bool,
    /// A pending OAM read's lock at the drive enable (tobe↑) — the
    /// single-speed decisive grant sample, taken before that fall's lock
    /// onset (`resolve_read_latch` consumes it).
    read_drive_oam_lock: Option<bool>,
    /// Undocumented CGB scratch registers: $FF72/$FF73 full bytes, $FF74
    /// (CGB mode only; open bus in compat), $FF75 bits 6-4 (the rest read 1).
    ff72: u8,
    ff73: u8,
    ff74: u8,
    ff75: u8,
    /// CGB ≤C extra OAM rows: 24 RAM bytes behind a decoder that ignores
    /// address bits 3-4 (three 8-byte rows at $FEA0/$FEC0/$FEE0, each
    /// aliased 4x in its block).
    extra_oam: [u8; 24],
}

impl Default for Cgb {
    fn default() -> Self {
        Self {
            wram: Box::new([0; 0x8000]),
            svbk: 1,
            key1_armed: false,
            double_speed: false,
            dmg_compat: false,
            vram_dma: VramDma::default(),
            speed_switch_blackout: 0,
            pre_alet_rendering: false,
            read_drive_oam_lock: None,
            ff72: 0,
            ff73: 0,
            ff74: 0,
            ff75: 0,
            extra_oam: [0; 24],
        }
    }
}

impl Cgb {
    /// Master edges of the clock-mux relock tail at the end of the blackout.
    /// `double_speed` holds the NEW speed: the 1×→2× swap settles one way, the
    /// 2×→1× swap another (the latter sets the entry phase of the next swap).
    fn relock_edges(&self) -> u32 {
        if self.double_speed {
            SWITCH_TO_DOUBLE_RELOCK_EDGES
        } else {
            SWITCH_TO_SINGLE_RELOCK_EDGES
        }
    }

    /// Index into `extra_oam` for a $FEA0-$FEFF address: row from address
    /// bits 6-5, offset from bits 2-0 (bits 3-4 ignored by the decoder).
    fn extra_oam_index(address: u16) -> usize {
        let row = ((address >> 5) & 0x7) as usize - 5;
        row * 8 + (address & 0x7) as usize
    }

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

/// CGB splits the cartridge and WRAM onto separate buses (DMG shares one
/// external bus), so the CPU can touch one while OAM DMA drives the other.
#[derive(PartialEq)]
enum CgbBus {
    Cartridge,
    WorkRam,
    Video,
}

fn cgb_bus(address: u16) -> Option<CgbBus> {
    match address {
        0x8000..=0x9FFF => Some(CgbBus::Video),
        0xC000..=0xFDFF => Some(CgbBus::WorkRam),
        0x0000..=0x7FFF | 0xA000..=0xBFFF => Some(CgbBus::Cartridge),
        _ => None,
    }
}

/// The bus an OAM-DMA *source* page drives, per the DMA decoder's external-RAM
/// `/CS` for `$A0–$FF`. Differs from `cgb_bus` in the echo region: `$E000–$FDFF`
/// is WRAM to the CPU but, to the DMA, is past the cart-RAM window — the
/// cartridge bus (which floats to `$FF`, see `dma_source_open_bus`). `$C0–$DF`
/// still reaches real WRAM on the WRAM bus.
fn cgb_dma_source_bus(address: u16) -> CgbBus {
    match address {
        0x8000..=0x9FFF => CgbBus::Video,
        0xC000..=0xDFFF => CgbBus::WorkRam,
        _ => CgbBus::Cartridge,
    }
}

impl Model for Cgb {
    type Ppu = CgbPpu;
    type Screen = Screen;
    const TRACE_MODEL_NAME: &'static str = "CGB-C";
    const WAVE_RAM_COUPLING: WaveRamCoupling = WaveRamCoupling::ChannelPosition;
    const HAS_PCM_REGISTERS: bool = true;

    fn oam_dma_bus_conflict(&self, cpu_addr: u16, dma_source: u16) -> bool {
        cgb_bus(cpu_addr) == Some(cgb_dma_source_bus(dma_source))
    }

    /// A WRAM-bus access taken while the DMA sources from the cart bus has its
    /// `$C000`/`$D000` half-selector (A12) driven by the DMA source page; the low
    /// 12 bits stay the CPU's. A VRAM or WRAM source leaves the access untouched.
    fn oam_dma_wram_remap(&self, cpu_addr: u16, dma_source: u16) -> Option<u16> {
        (cgb_bus(cpu_addr) == Some(CgbBus::WorkRam)
            && cgb_dma_source_bus(dma_source) == CgbBus::Cartridge)
            .then(|| (dma_source & 0x1000) | (cpu_addr & 0x0FFF) | 0xC000)
    }

    /// On the WRAM bus the colliding CPU write sits on a different bus from the
    /// DMA source, so it never reaches the OAM write phase — the DMA deposits the
    /// raw byte it fetched. Other source buses follow the shared model.
    fn oam_dma_write_conflict_byte(&self, src_byte: u8, cpu_value: u8, dma_source: u16) -> u8 {
        if cgb_dma_source_bus(dma_source) == CgbBus::WorkRam {
            src_byte
        } else {
            shared_oam_dma_write_conflict_byte(src_byte, cpu_value, dma_source)
        }
    }

    fn oam_dma_conflict_zeroes_oam(&self, cpu_addr: u16, dma_source: u16) -> bool {
        cgb_dma_source_bus(dma_source) == CgbBus::Video && cgb_bus(cpu_addr) == Some(CgbBus::Video)
    }

    fn dma_source_open_bus(&self, address: u16) -> Option<u8> {
        (address >= 0xE000).then_some(0xFF)
    }

    fn cpu_post_boot(_checksum: u8) -> Cpu {
        Cpu::post_boot_cgb()
    }

    fn has_serial_fast_clock(&self) -> bool {
        !self.dmg_compat
    }

    fn halt_wake_samples_early(&self) -> bool {
        // The T2-rise presample holds at both speeds — double speed shifts the
        // dot↔T-cycle ratio, not where in the M-cycle the comparator samples.
        true
    }

    /// CGB boot-ROM handoff divider phase. The boot ROM runs longer for a
    /// DMG cartridge (compat-palette setup): FF04 reads $1E / $26.
    fn timers_post_boot(cgb_cart: bool) -> Timers {
        Timers::post_boot_with_counter(if cgb_cart { 0x47A8 } else { 0x099F })
    }

    /// CGB boot-ROM handoff is mid-VBlank; the line depends on the boot
    /// duration (CGB cart: line 144, dot ~164; DMG cart: line 148, dot ~356).
    /// The boot ROM also zeroes OBP0/OBP1 (DMG leaves them at $FF).
    fn ppu_post_boot(cgb_cart: bool) -> Ppu<CgbPpu> {
        let mut ppu = if cgb_cart {
            Ppu::post_boot_vblank_handoff(144, 41)
        } else {
            Ppu::post_boot_vblank_handoff(148, 88)
        };
        ppu.set_post_boot_object_palettes(0x00);
        ppu
    }

    /// The CGB boot ROM hands off with both key-matrix lines deselected
    /// (P1 reads $FF).
    fn joypad_post_boot() -> Joypad {
        Joypad {
            read_buttons: false,
            read_dpad: false,
            pressed_buttons: Vec::new(),
        }
    }

    /// The CGB boot ROM leaves FF46 reading $00.
    fn dma_post_boot() -> Dma {
        Dma::with_source_register(0x00)
    }

    fn resolve_stop(&mut self) -> StopAction {
        if self.key1_armed {
            // The clock-mux settle is bus-coupled, and only the upward swap
            // disturbs the mux and resets the trigger's request/commit
            // chain (the CPU-written arming/length registers persist).
            if self.double_speed {
                // Downward: the chain survives, so a granted burst keeps
                // the bus and the settle waits for its release.
                if self.vram_dma.block_remaining > 0 && self.vram_dma.ready_in == 0 {
                    return StopAction::Remain;
                }
            } else {
                // Upward: the reset grades the committed block, which is
                // cancel-immune and ignores the arming flag. Not yet
                // bus-eligible: discarded whole. Bus-eligible: the dropped
                // grant latches the stop condition — the in-flight byte
                // completes outside the latched length.
                if self.vram_dma.block_remaining > 0 {
                    if self.vram_dma.ready_in == 0 {
                        self.vram_dma.mode = TransferMode::Idle;
                        self.vram_dma.block_remaining = 1;
                        self.vram_dma.escape_byte = true;
                    } else {
                        self.vram_dma.block_remaining = 0;
                        self.vram_dma.ready_in = 0;
                        self.vram_dma.setup_cells = 0;
                    }
                }
                self.vram_dma.pend = false;
            }
            self.double_speed = !self.double_speed;
            self.key1_armed = false;
            self.speed_switch_blackout = self.speed_switch_blackout_master_edges();
            StopAction::SpeedSwitch
        } else {
            StopAction::Remain
        }
    }

    fn speed_switch_in_progress(&self) -> bool {
        self.speed_switch_blackout > 0
    }

    fn drain_speed_switch_blackout(&mut self, elapsed: u32) -> bool {
        self.speed_switch_blackout = self.speed_switch_blackout.saturating_sub(elapsed);
        self.speed_switch_blackout == 0
    }

    fn speed_switch_divider_active(&self) -> bool {
        // The divider runs through the hold but freezes during the relock tail:
        // the CPU clock is still settling there, so it gains no edges (this keeps
        // the re-phase from disturbing DIV). The tail is the final `relock`
        // master edges of the count. Placing the quiet edges at the tail vs the
        // head is observationally identical (no test in the corpus latches a
        // divider-driven event in that window), so this picks the resume-side
        // offset that SameBoy/gambatte also model.
        self.speed_switch_blackout > self.relock_edges()
    }

    fn cpu_steps_per_dot(&self) -> u8 {
        if self.double_speed { 2 } else { 1 }
    }

    fn speed_switch_blackout_master_edges(&self) -> u32 {
        // The blackout is a fixed real-time hold; in master edges (dot-clock
        // half-cycles) it is the same duration at either speed — the dot clock
        // runs at a constant rate. `double_speed` already holds the new speed,
        // so convert the T-cycle figure by the post-switch ratio (2 master
        // edges per CPU T-cycle at single speed, 1 at double). The relock tail
        // rides on the end (PPU only, divider quiet).
        let hold = SPEED_SWITCH_BLACKOUT_TCYCLES * 2 / self.cpu_steps_per_dot() as u32;
        hold + self.relock_edges()
    }

    fn note_pre_alet_rendering(&mut self, rendering: bool) {
        if self.double_speed {
            self.pre_alet_rendering = rendering;
        }
    }

    fn note_read_drive_phase(&mut self, oam_lock: Option<bool>) {
        self.read_drive_oam_lock = oam_lock;
    }

    fn resolve_read_latch(&self, address: u16, value: u8, latch_lock: Option<bool>) -> u8 {
        match address {
            // Double-speed STAT mode bits: the read's data_phase_n↑ latches
            // before this dot's ALET edge, where VOGA clears XYMU (mode 3→0).
            // So a read taken while the PPU was rendering just before that edge
            // reads mode 3 even though the post-edge live mode has already
            // fallen to 0. This is the CGB CPU↔ALET half-dot phase — distinct
            // from the DMG, whose lockstep timing lands the latch after the edge.
            0xFF41 if self.double_speed => {
                if self.pre_alet_rendering {
                    value | 0b11
                } else {
                    value
                }
            }
            // Single speed: OR-of-accessibility over the drive-enable grant
            // sample and the latch-edge lock — the bus keeps the byte OAM
            // drove while addressed and unlocked. (The earlier address-phase
            // grant is double-speed-only; a single-speed onset between the
            // address phase and tobe↑ still floats the read.)
            0xFE00..=0xFEFF if !self.double_speed => match (self.read_drive_oam_lock, latch_lock) {
                (Some(false), _) => value,
                (_, Some(true)) => 0xFF,
                _ => value,
            },
            _ if latch_lock == Some(true) => 0xFF,
            _ => value,
        }
    }

    fn on_reset(&mut self, cartridge: &Cartridge, has_boot_rom: bool) {
        *self = Self::default();
        // A DMG cartridge boots the CGB into compatibility mode (KEY0 bit 2).
        // With a real boot ROM that decision is the boot ROM's (via KEY0);
        // only HLE it on the skip-boot path.
        if !has_boot_rom {
            self.dmg_compat = !cartridge.is_cgb();
        }
    }

    fn map_read(&self, address: u16, ppu: &Ppu<CgbPpu>, vram: &CgbVram) -> Option<u8> {
        if let Some(i) = self.wram_index(address) {
            return Some(self.wram[i]);
        }
        match address {
            0xFEA0..=0xFEFF => Some(self.extra_oam[Self::extra_oam_index(address)]),
            // DMG-compat locks out the speed/banking/priority registers and
            // the $FF74 scratch byte — open bus for the rest of the session.
            0xFF4C | 0xFF4D | 0xFF6C | 0xFF70 | 0xFF74 if self.dmg_compat => Some(0xFF),
            // KEY0: boot-locked; reads the latched mode ($00 = CGB).
            0xFF4C => Some(0x00),
            0xFF4D => Some(0x7E | ((self.double_speed as u8) << 7) | self.key1_armed as u8), // KEY1
            0xFF4F => Some(vram.read_bank_select()),                                         // VBK
            // HDMA1-4 are write-only.
            0xFF51..=0xFF54 => Some(0xFF),
            // HDMA5 status: bit 7 = 0 while an HDMA is active, blocks-left-minus-1
            // in bits 6-0. Idle/done/stopped reads bit 7 = 1 (done = $FF). A GDMA
            // is never observable here — it holds the CPU for its whole duration.
            0xFF55 => {
                let active = self.vram_dma.mode == TransferMode::HBlank;
                let blocks = self.vram_dma.remaining / 16;
                Some(((!active as u8) << 7) | (blocks.wrapping_sub(1) & 0x7F) as u8)
            }
            0xFF68 => Some(ppu.read_color_register(ColorRegister::BackgroundIndex)), // BCPS
            0xFF69 => Some(ppu.read_color_register(ColorRegister::BackgroundData)),  // BCPD
            0xFF6A => Some(ppu.read_color_register(ColorRegister::ObjectIndex)),     // OCPS
            0xFF6B => Some(ppu.read_color_register(ColorRegister::ObjectData)),      // OCPD
            0xFF6C => Some(ppu.read_object_priority()),                              // OPRI
            0xFF70 => Some(self.svbk | 0xF8), // SVBK: bits 0-2
            0xFF72 => Some(self.ff72),
            0xFF73 => Some(self.ff73),
            0xFF74 => Some(self.ff74),
            0xFF75 => Some(0x8F | self.ff75),
            _ => None,
        }
    }

    fn map_write(
        &mut self,
        address: u16,
        value: u8,
        ppu: &mut Ppu<CgbPpu>,
        vram: &mut CgbVram,
    ) -> bool {
        if let Some(i) = self.wram_index(address) {
            self.wram[i] = value;
            return true;
        }
        match address {
            0xFEA0..=0xFEFF => {
                self.extra_oam[Self::extra_oam_index(address)] = value;
                true
            }
            // DMG-compat locks out the speed/banking/priority/VRAM-DMA
            // registers and the $FF74 scratch byte.
            0xFF4D | 0xFF51..=0xFF55 | 0xFF6C | 0xFF70 | 0xFF74 if self.dmg_compat => true,
            0xFF4C => true, // KEY0: boot-locked, ignore
            0xFF4D => {
                self.key1_armed = value & 0x01 != 0;
                true
            }
            0xFF4F => {
                vram.write_bank_select(value); // VBK
                true
            }
            0xFF51 => {
                self.vram_dma.source = (self.vram_dma.source & 0x00FF) | ((value as u16) << 8);
                true
            }
            0xFF52 => {
                self.vram_dma.source = (self.vram_dma.source & 0xFF00) | (value & 0xF0) as u16;
                true
            }
            0xFF53 => {
                let low = self.vram_dma.dest & 0x00FF;
                self.vram_dma.dest = 0x8000 | ((((value as u16) << 8) | low) & 0x1FF0);
                true
            }
            0xFF54 => {
                self.vram_dma.dest =
                    0x8000 | ((self.vram_dma.dest & 0x1F00) | (value & 0xF0) as u16);
                true
            }
            0xFF55 => {
                let length = ((value & 0x7F) as u16 + 1) * 16;
                if value & 0x80 != 0 {
                    // Arm HDMA: one 16-byte block per HBlank. A block already
                    // latched by the trigger is immune and keeps flowing; an
                    // arm landing during mode 0 pends at this fall's trigger
                    // evaluation. With the LCD off no HBlank will come — the
                    // arm strobe services one block immediately.
                    self.vram_dma.mode = TransferMode::HBlank;
                    self.vram_dma.remaining = length;
                    self.vram_dma.armed_this_fall = true;
                    if !ppu.control().video_enabled() {
                        self.vram_dma.block_remaining = 16;
                        self.vram_dma.pend_from_arm = true;
                        self.vram_dma.setup_cells = 0;
                        self.vram_dma.ready_in = 2;
                    }
                } else if self.vram_dma.mode == TransferMode::HBlank {
                    // bit 7 = 0 while an HDMA runs clears the arming only (no
                    // GDMA starts); a latched block completes. Bits 6-0 are
                    // the length register and store on every write — the
                    // status read reflects them.
                    self.vram_dma.mode = TransferMode::Idle;
                    self.vram_dma.remaining = length;
                } else {
                    // GDMA: copy the whole length while holding the CPU.
                    self.vram_dma.mode = TransferMode::General;
                    self.vram_dma.remaining = length;
                }
                true
            }
            0xFF68 => {
                ppu.write_color_register(ColorRegister::BackgroundIndex, value); // BCPS
                true
            }
            0xFF69 => {
                ppu.write_color_register(ColorRegister::BackgroundData, value); // BCPD
                true
            }
            0xFF6A => {
                ppu.write_color_register(ColorRegister::ObjectIndex, value); // OCPS
                true
            }
            0xFF6B => {
                ppu.write_color_register(ColorRegister::ObjectData, value); // OCPD
                true
            }
            0xFF6C => {
                ppu.write_object_priority(value); // OPRI
                true
            }
            0xFF70 => {
                self.svbk = value & 0x07;
                true
            }
            0xFF72 => {
                self.ff72 = value;
                true
            }
            0xFF73 => {
                self.ff73 = value;
                true
            }
            0xFF74 => {
                self.ff74 = value;
                true
            }
            0xFF75 => {
                self.ff75 = value & 0x70;
                true
            }
            _ => false,
        }
    }

    fn vram_dma_tick(&mut self, mode: Mode, engine_gated: bool, cpu_halted: bool) -> VramDmaClaim {
        let in_hblank = mode == Mode::HorizontalBlank;
        let entry_edge = in_hblank && !self.vram_dma.prev_view_hblank;
        self.vram_dma.prev_view_hblank = in_hblank;
        if cpu_halted {
            self.vram_dma.halted_falls = self.vram_dma.halted_falls.saturating_add(1);
        } else {
            self.vram_dma.halted_falls = 0;
        }
        // The taken-clear stays live through the halt-latch M-cycle, then
        // freezes until the CPU's own resume (halt only; STOP freezes it
        // outright via the engine gate).
        if !in_hblank && (!cpu_halted || self.vram_dma.halted_falls <= 4) {
            self.vram_dma.hblank_block_taken = false;
        }
        // The engine gate freezes commit/grant; the mode-0 entry detector
        // keeps running — an entry registers a pend-request (consulting the
        // taken flag) that persists through the freeze and commits at the
        // thaw. A latched block keeps draining.
        if engine_gated {
            if self.vram_dma.pend {
                self.vram_dma.pend_age = self.vram_dma.pend_age.saturating_add(1);
            }
            if cpu_halted
                && entry_edge
                && self.vram_dma.mode == TransferMode::HBlank
                && !self.vram_dma.hblank_block_taken
                && self.vram_dma.remaining > 0
            {
                self.vram_dma.pend = true;
                self.vram_dma.pend_from_arm = false;
                self.vram_dma.pend_age = 0;
            }
            self.vram_dma.quota = if self.vram_dma.moving() {
                if self.double_speed { 1 } else { 2 }
            } else {
                0
            };
            return VramDmaClaim::default();
        }

        // Two-stage trigger, evaluated each fall on the post-rise mode view
        // with this fall's FF55 commit visible: a pend commits to a
        // cancel-immune block one fall later; an FF55 write at either fall
        // kills the pend (armed is consulted at both stages).
        let armed = self.vram_dma.mode == TransferMode::HBlank;
        let committing = self.vram_dma.pend
            && armed
            && in_hblank
            && (!cpu_halted || self.vram_dma.pend_age <= 2);
        if committing {
            self.vram_dma.block_remaining = 16;
            self.vram_dma.hblank_block_taken = true;
            self.vram_dma.ready_in = 2;
            self.vram_dma.setup_cells = if self.vram_dma.pend_from_arm { 0 } else { 1 };
        }
        self.vram_dma.pend = !committing
            && armed
            && in_hblank
            && !self.vram_dma.hblank_block_taken
            && self.vram_dma.remaining > 0;
        if self.vram_dma.pend {
            self.vram_dma.pend_from_arm = self.vram_dma.armed_this_fall;
            self.vram_dma.pend_age = 0;
        }
        self.vram_dma.armed_this_fall = false;
        if self.vram_dma.ready_in > 0 {
            self.vram_dma.ready_in -= 1;
        }

        // Refill this M-cycle's byte budget while the transfer is moving bytes:
        // 2/M-cycle single speed, 1 in double speed.
        self.vram_dma.quota = if self.vram_dma.moving() {
            if self.double_speed { 1 } else { 2 }
        } else {
            0
        };
        VramDmaClaim {
            committed: committing,
            // A claim is standing once it has aged through one full M-cycle
            // of the freeze — the synchronizer stage that carries it into
            // the CPU's M-cycle clock domain; a younger claim hasn't crossed when
            // the halt-release fetch starts.
            standing: committing && self.vram_dma.pend_age >= 4,
        }
    }

    fn vram_dma_next_byte(&mut self) -> Option<(u16, u16)> {
        if self.vram_dma.quota == 0 || !self.vram_dma.moving() {
            return None;
        }
        let pair = (self.vram_dma.source, self.vram_dma.dest);
        // Pointers advance per byte and persist for any follow-on transfer; the
        // destination wraps within VRAM. A switch-cancel escape byte does not
        // count against the latched length.
        self.vram_dma.source = self.vram_dma.source.wrapping_add(1);
        self.vram_dma.dest = 0x8000 | (self.vram_dma.dest.wrapping_add(1) & 0x1FFF);
        if self.vram_dma.escape_byte {
            self.vram_dma.escape_byte = false;
        } else {
            self.vram_dma.remaining -= 1;
        }
        self.vram_dma.quota -= 1;
        if self.vram_dma.block_remaining > 0 {
            self.vram_dma.block_remaining -= 1;
        }
        if self.vram_dma.remaining == 0 {
            self.vram_dma.mode = TransferMode::Idle;
        }
        Some(pair)
    }

    fn vram_dma_holds_cpu(&self) -> bool {
        self.vram_dma.mode == TransferMode::General && self.vram_dma.remaining > 0
    }

    fn vram_dma_seizes_bus(&self) -> bool {
        self.vram_dma.ready_in == 0
            && (self.vram_dma.setup_cells > 0
                || (self.vram_dma.block_remaining > 0 && self.vram_dma.remaining > 0))
    }

    fn vram_dma_take_setup_cell(&mut self) -> bool {
        if self.vram_dma.setup_cells > 0 {
            self.vram_dma.setup_cells -= 1;
            true
        } else {
            false
        }
    }
}

/// The Game Boy Color.
pub type GameBoyColor = Console<Cgb>;

#[cfg(test)]
mod dmg_palette_tests {
    use super::*;

    fn title(s: &str) -> [u8; 16] {
        let mut t = [0u8; 16];
        for (i, b) in s.bytes().take(16).enumerate() {
            t[i] = b;
        }
        t
    }

    #[test]
    fn non_nintendo_falls_to_compat_default() {
        // Any non-Nintendo licensee gates to combination 0, regardless of title.
        let (bg, obj0, obj1) = dmg_compat_palettes(&title("TETRIS"), 0x00, [0, 0]);
        assert_eq!(bg, DMG_COMPAT_BG);
        assert_eq!(obj0, DMG_COMPAT_OBJ);
        assert_eq!(obj1, DMG_COMPAT_OBJ);
    }

    #[test]
    fn nintendo_title_selects_its_palette() {
        // TETRIS (old licensee $01, checksum $DB) selects combination 3 = palette 24.
        let (bg, _, _) = dmg_compat_palettes(&title("TETRIS"), 0x01, [0, 0]);
        assert_eq!(bg, dmg_palette_data::PALETTES[24]);
        assert_ne!(bg, DMG_COMPAT_BG);
    }

    #[test]
    fn fourth_letter_disambiguates_checksum_collision() {
        // Two titles with the same checksum ($46) but different 4th letters resolve
        // to different table entries (66 = 'E', 80 = 'R') via the tiebreak search.
        let mut e = [0u8; 16];
        e[0] = 0x01;
        e[3] = b'E';
        let mut r = [0u8; 16];
        r[0] = 0xF4;
        r[3] = b'R';
        assert_eq!(e.iter().fold(0u8, |s, &x| s.wrapping_add(x)), 0x46);
        assert_eq!(r.iter().fold(0u8, |s, &x| s.wrapping_add(x)), 0x46);

        let bg_of = |combo: u8| {
            dmg_palette_data::PALETTES
                [dmg_palette_data::PALETTE_COMBINATIONS[combo as usize][2] as usize]
        };
        let combo_e = dmg_palette_data::PALETTE_PER_CHECKSUM[66] & 0x7F;
        let combo_r = dmg_palette_data::PALETTE_PER_CHECKSUM[80] & 0x7F;
        assert_eq!(dmg_compat_palettes(&e, 0x01, [0, 0]).0, bg_of(combo_e));
        assert_eq!(dmg_compat_palettes(&r, 0x01, [0, 0]).0, bg_of(combo_r));
    }
}
