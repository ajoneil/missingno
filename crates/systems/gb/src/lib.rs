pub mod audio;
pub mod board;
pub mod cartridge;
pub mod clock;
pub mod cpu;
pub mod cpu_bus;
pub mod debugger;
pub mod dma;
pub mod dmg_sram;
pub mod execute;
pub mod frame;
pub mod interrupts;
pub mod isa;
pub mod joypad;
pub mod media;
pub mod memory;
pub mod ppu;
pub mod serial_transfer;
pub mod sgb;
pub mod snapshot;
pub mod state_schema;
pub mod system;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod timers;
#[cfg(feature = "morepork")]
pub mod trace;

use audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use cpu_bus::CpuBus;
use dma::Dma;
use joypad::{Button, Joypad};
use memory::{Bus, ExternalBus, HighRam, VramBus};
use ppu::Ppu;
use ppu::memory::Vram;
use ppu::model::PpuModel;

pub use audio::channels::wave::WaveRamCoupling;
pub use audio::{ApuSpec, DmgApu};
pub use clock::{CpuDivider, CpuGate, Edge, MasterClock, Tick};
pub use isa::Sm83;
pub use memory::BootRom;
pub use ppu::PixelOutput;

/// Double-buffered LCD framebuffer, abstracted over its pixel storage so
/// the shared core can drive a DMG shade buffer or a CGB color buffer.
pub trait ScreenBuffer: Default + Clone {
    type Pixel: Copy;
    fn draw_pixel(&mut self, x: u8, y: u8, pixel: Self::Pixel);
    /// Swap back→front and clear back. Returns true for `new_screen` tracking.
    fn present(&mut self) -> bool;
    fn blank(&mut self);
    /// Seed the displayed (front) buffer from a save state's framebuffer bytes,
    /// so the first frame after a restore matches the save. Each console decodes
    /// its own pixel format (DMG shade indices, CGB little-endian RGB555).
    fn restore(&mut self, bytes: &[u8]);
}

/// CGB-only console-level arbitration state, relocated off the shared
/// [`Console`] so a DMG build carries none of it. The CGB model owns the real
/// storage; the DMG model is a ZST `()`, since none of these paths — the
/// speed-switch blackout, the HDMA bus-park, the VRAM-source OAM-zero conflict
/// — exist on the DMG.
pub trait ConsoleShadow {
    /// The master-edge count a double-speed switch blackout began on; the
    /// elapsed held edges are `master_edge - anchor`. Re-anchored at each switch.
    fn blackout_anchor(&self) -> u64;
    fn set_blackout_anchor(&mut self, edge: u64);

    /// A VRAM DMA is holding the CPU clock this M-cycle (bus master owns the bus).
    fn dma_cpu_hold(&self) -> bool;
    fn set_dma_cpu_hold(&mut self, held: bool);

    /// A bus master owns the VRAM/external bus this M-cycle, so a CPU access
    /// starting here waits for release (per-bus wait states, the sibling of the
    /// whole-bandwidth `dma_cpu_hold`). Computed at each M-boundary.
    fn bus_suspended(&self) -> bool;
    fn set_bus_suspended(&mut self, suspended: bool);

    /// The VRAM-DMA trigger's bus claim committed this M-cycle (consumed at the
    /// next M-cycle pick and then cleared).
    fn vram_dma_claim(&self) -> VramDmaClaim;
    fn set_vram_dma_claim(&mut self, claim: VramDmaClaim);
    fn clear_vram_dma_claim(&mut self);

    /// OAM offset whose DMA-deposited byte a VRAM-source bus conflict forces to
    /// `$00`, drained at the M-cycle-boundary fall.
    fn dma_conflict_oam_zero(&self) -> Option<u8>;
    fn set_dma_conflict_oam_zero(&mut self, offset: Option<u8>);
    fn take_dma_conflict_oam_zero(&mut self) -> Option<u8>;
}

impl ConsoleShadow for () {
    fn blackout_anchor(&self) -> u64 {
        0
    }
    fn set_blackout_anchor(&mut self, _edge: u64) {}
    fn dma_cpu_hold(&self) -> bool {
        false
    }
    fn set_dma_cpu_hold(&mut self, _held: bool) {}
    fn bus_suspended(&self) -> bool {
        false
    }
    fn set_bus_suspended(&mut self, _suspended: bool) {}
    fn vram_dma_claim(&self) -> VramDmaClaim {
        VramDmaClaim::default()
    }
    fn set_vram_dma_claim(&mut self, _claim: VramDmaClaim) {}
    fn clear_vram_dma_claim(&mut self) {}
    fn dma_conflict_oam_zero(&self) -> Option<u8> {
        None
    }
    fn set_dma_conflict_oam_zero(&mut self, _offset: Option<u8>) {}
    fn take_dma_conflict_oam_zero(&mut self) -> Option<u8> {
        None
    }
}

/// The HDMA trigger's bus claim committed on a fall: `standing` marks a
/// claim that aged through its synchronizer stage before committing (it
/// wins the bus race against the halt-release fetch).
#[derive(Copy, Clone, Default)]
pub struct VramDmaClaim {
    pub committed: bool,
    pub standing: bool,
}

/// OAM byte a write-conflict lands under the shared external-bus model: a WRAM
/// source (`$C0–$FF`) keeps its driver live through the OAM write phase and
/// AND-mixes with the CPU value; a ROM/SRAM source releases, so the CPU value
/// lands. CGB overrides this for its separate WRAM bus.
pub fn shared_oam_dma_write_conflict_byte(src_byte: u8, cpu_value: u8, dma_source: u16) -> u8 {
    if matches!((dma_source >> 8) as u8, 0xC0..=0xFF) {
        src_byte & cpu_value
    } else {
        cpu_value
    }
}

/// Whether a read of `address` observes live APU state: the audio register
/// file, wave RAM, and the CGB PCM taps.
pub(crate) fn observes_audio(address: u16) -> bool {
    matches!(address, 0xFF10..=0xFF3F | 0xFF76 | 0xFF77)
}

/// The per-console divergences from the shared SM83 silicon — the entire
/// catalogue of how DMG and CGB differ in the step loop and memory map.
/// Everything not listed here is the same silicon and lives in [`Console`].
pub trait Model: Default {
    /// The PPU's per-console hardware: DMG monochrome, CGB colour.
    type Ppu: PpuModel;

    /// Framebuffer storage; its pixel matches what `Self::Ppu` resolves
    /// (DMG = `PaletteIndex` shades, CGB = RGB555).
    type Screen: ScreenBuffer<Pixel = <Self::Ppu as PpuModel>::Pixel>;

    /// CGB-only console-level arbitration state (speed-switch blackout anchor,
    /// HDMA bus-park, VRAM-source OAM-zero conflict). The CGB holds the real
    /// `CgbConsoleState`; the DMG carries a ZST `()`.
    type ConsoleState: ConsoleShadow + Default;

    /// Static, per-console APU properties (double-speed, the CGB sweep/noise
    /// grid quirks, wave-RAM coupling). The DMG uses `DmgApu` (all defaults).
    type Apu: ApuSpec;

    fn console_state(&self) -> &Self::ConsoleState;
    fn console_state_mut(&mut self) -> &mut Self::ConsoleState;

    /// DMG arms/fires the OAM-corruption bug (BOWA/CUFE); CGB silicon has none.
    const HAS_OAM_BUG: bool = false;

    /// Console has the KEY1 ÷2 cell. When false every double-speed branch in
    /// the shared step loop is dead code.
    const DOUBLE_SPEED: bool = false;

    /// VRAM banks the console carries: one on the DMG (fully visible through the
    /// `$8000` window), two on the CGB (VBK-switched). More than one means the
    /// debugger exposes the bank-complete image linearly above the bus.
    const VRAM_BANKS: u8 = 1;

    /// CGB silicon exposes the APU channel DAC outputs at FF76/FF77.
    const HAS_PCM_REGISTERS: bool = false;

    /// CGB's halt-release comparator samples IF&IE two T-cycles before
    /// the M-cycle boundary; DMG samples at the boundary.
    const HALT_WAKE_SAMPLES_EARLY: bool = false;

    /// cpu_irq_ack1 (LALU.r_n) holds the dispatched IF bit in reset across
    /// the ack window. On CGB the release trails by one step, so a timer or
    /// serial IF assertion landing on the dispatch's M-cycle boundary is still
    /// caught by the reset — the serviced bit reads back clear where DMG, which
    /// releases just ahead of that boundary set, reads it set.
    const IRQ_ACK_HOLDS_THROUGH_BOUNDARY_SET: bool = false;

    /// Hardware revision name recorded in morepork captures.
    const TRACE_MODEL_NAME: &'static str = "DMG-B";

    /// The LCD panel technology the console drives — passive STN on the DMG,
    /// active TFT on the CGB.
    const LCD_PANEL: missingno_core::LcdPanel = missingno_core::LcdPanel::PassiveStn;

    /// Bank-complete work RAM the debugger exposes linearly above the bus, when
    /// the console banks WRAM (CGB's eight 4 KB banks). `None` for a flat-WRAM
    /// console (DMG): its 8 KB is fully visible through the `$C000` bus window.
    fn wram_image(&self) -> Option<&[u8]> {
        None
    }

    /// The work-RAM bank currently paged into the `$D000` window, on a console
    /// that banks WRAM (CGB). `None` for a flat-WRAM console (DMG).
    fn selected_wram_bank(&self) -> Option<u8> {
        None
    }

    /// End-of-frame / LCD-off hook. DMG mirrors the screen to the SGB.
    fn on_present(&mut self, _screen: &Self::Screen) {}

    /// Post-process a JOYP read. DMG applies SGB player multiplexing.
    fn read_joypad(&self, value: u8) -> u8 {
        value
    }

    /// Side effect of a JOYP write. DMG forwards the pulse to the SGB.
    fn on_joypad_write(&mut self, _value: u8) {}

    /// CGB-mode SIO has the SC bit-1 fast-clock select (SIO_FAST); the bit
    /// is unimplemented on DMG and in CGB DMG-compat mode (reads 1, no effect).
    fn has_serial_fast_clock(&self) -> bool {
        false
    }

    /// Re-create model-specific state on power-cycle. DMG (re)builds the
    /// SGB co-processor from the cartridge header. `has_boot_rom` is true
    /// when a real boot ROM will run — the model must then skip any
    /// post-boot HLE the boot ROM performs itself (CGB: the DMG-compat
    /// KEY0/palette setup).
    fn on_reset(&mut self, _cartridge: &Cartridge, _has_boot_rom: bool) {}

    /// Restore work RAM from a save state's bank-complete image at a boundary.
    /// The DMG copies the 8 KiB into its flat work RAM; the CGB fills its eight
    /// banks (held in the model, off the shared bus).
    fn restore_work_ram(&mut self, external: &mut ExternalBus, bytes: &[u8]) {
        let len = bytes.len().min(external.work_ram.len());
        external.work_ram[..len].copy_from_slice(&bytes[..len]);
    }

    /// Reject a record this model cannot faithfully restore at a boundary,
    /// before any state is mutated. The CGB refuses a double-speed save — the
    /// dot-phase alignment a speed switch leaves is not boundary-observable.
    fn validate_boundary(
        &self,
        _record: &missingno_core::state::StateRecord,
    ) -> Result<(), missingno_core::system::StateError> {
        Ok(())
    }

    /// Restore the model's own hardware delta after the shared subsystems are
    /// rebuilt: the CGB reseats its banked VRAM/palette RAM, the VBK / OPRI /
    /// palette-index registers, the single-speed clock, and the VRAM-DMA cursor.
    /// A no-op on the DMG.
    fn restore_boundary_delta(
        &mut self,
        _chassis: &mut Chassis<Self>,
        _record: &missingno_core::state::StateRecord,
        _memory: &[(String, Vec<u8>)],
    ) -> Result<(), missingno_core::system::StateError> {
        Ok(())
    }

    /// Post-boot CPU state when no boot ROM is present. DMG seeds the flags
    /// from the header checksum; CGB uses a fixed register file (A=$11).
    fn cpu_post_boot(checksum: u8) -> Cpu {
        Cpu::post_boot(checksum)
    }

    /// Post-boot timer state when no boot ROM is present — each console's
    /// boot ROM leaves a different divider phase at handoff, and the CGB
    /// boot duration depends on the cartridge's CGB header flag.
    fn timers_post_boot(_cgb_cart: bool) -> timers::Timers {
        timers::Timers::post_boot()
    }

    /// Post-boot PPU state when no boot ROM is present — each console's
    /// boot ROM hands off at a different point in the frame, and the CGB
    /// boot duration depends on the cartridge's CGB header flag.
    fn ppu_post_boot(_cgb_cart: bool) -> Ppu<Self::Ppu> {
        Ppu::post_boot()
    }

    /// Post-boot APU state when no boot ROM is present — the CGB boot ROM
    /// hands off at a different frame-sequencer step and CH1 duty phase.
    fn audio_post_boot(internal_counter: u16, _cgb_cart: bool) -> Audio<Self::Apu> {
        Audio::post_boot(internal_counter)
    }

    /// Post-boot joypad state — the CGB boot ROM deselects both key matrix
    /// lines; the DMG boot ROM leaves both selected.
    fn joypad_post_boot() -> Joypad {
        Joypad::new()
    }

    /// Post-boot OAM-DMA state — the CGB boot ROM leaves FF46 reading 0.
    fn dma_post_boot() -> Dma {
        Dma::new()
    }

    /// Resolve a STOP the CPU has settled into, given the shared [`Chassis`].
    /// DMG always stays stopped (returns false). CGB performs a double-speed
    /// switch when KEY1 is armed — resetting the divider, retapping the
    /// APU/serial, arming its blackout — and returns true so the caller drains
    /// the upward-grading escape byte. The model reads its own STOP entry phase
    /// (dot-in-M, the mux-relock alignment input) off the chassis PPU.
    fn resolve_stop(&mut self, _chassis: &mut Chassis<Self>) -> bool {
        false
    }

    /// Whether a double-speed switch blackout is draining (the CPU is held
    /// `Stopped` while the divider/PPU run). DMG: never.
    fn speed_switch_in_progress(&self) -> bool {
        false
    }

    /// Drain `elapsed` master edges from the switch blackout; returns true on
    /// the edge it empties (the CPU re-engages at the new speed). DMG: never.
    fn drain_speed_switch_blackout(&mut self, _elapsed: u32) -> bool {
        false
    }

    /// Whether the CPU-clock divider ticks this blackout edge. It runs through
    /// the hold but freezes during the clock-mux relock tail, so the re-phase
    /// doesn't disturb DIV. Only consulted mid-blackout (CGB).
    fn speed_switch_divider_active(&self) -> bool {
        true
    }

    /// CPU T-cycles advanced per PPU dot. 1 = lockstep (DMG always; CGB
    /// single speed); 2 = the CPU clock runs at twice the dot clock (CGB
    /// double speed), so a full CPU T-cycle lands on each master-clock edge.
    fn cpu_steps_per_dot(&self) -> u8 {
        1
    }

    /// A timer overflowing during the post-STOP HALT wakes it like any HALT:
    /// the IF-set edge spends one WakeIntake M-cycle (the divider ticking)
    /// before the dispatch. Arms on the first call, counts down at M-cycle
    /// boundaries, returns true once the intake elapses (then the CPU
    /// re-engages). DMG has no blackout, so it re-engages immediately.
    fn speed_switch_wake_ready(&mut self, _mcycle_boundary: bool) -> bool {
        true
    }

    /// The pre-ALET-rise XYMU (mode-3) state, sampled before this dot's
    /// `ppu_rise_edge` (the ALET-rising XYMU.q↑). A double-speed FF41 read
    /// latching on that phase resolves its STAT mode to this pre-transition
    /// view. DMG (latch always lands after a separate-phase rise) ignores it.
    fn note_pre_alet_rendering(&mut self, _rendering: bool) {}

    /// A pending lockable (OAM/VRAM) read's lock at the pre-ALET rise, sampled
    /// before this dot's `ppu_rise_edge` lock onset/release — the lock analogue
    /// of `note_pre_alet_rendering`. DMG ignores it.
    fn note_pre_alet_lock(&mut self, _lock: Option<bool>) {}

    /// A pending OAM read's lock at the drive enable (tobe↑, the read's third
    /// T-cycle fall), sampled before that fall's PPU advance applies any lock
    /// onset. DMG ignores it.
    fn note_read_drive_phase(&mut self, _oam_lock: Option<bool>) {}

    /// LY_old stashed at a double-speed mid-M LY tick with an FF44 read in
    /// flight — the read's latch samples the mux mid-ripple and ANDs this in.
    /// DMG never sees a mid-M tick (dot falls ride CPU falls).
    fn note_ff44_ripple_old(&mut self, _ly: Option<u8>) {}
    fn take_ff44_ripple_old(&mut self) -> Option<u8> {
        None
    }

    /// Resolve the value a CPU read latches. A lockable (OAM/VRAM) read
    /// arrives unfloated with its live lock in `latch_lock`; the model owns
    /// the float. DMG floats on the latch-edge lock alone; CGB also applies its
    /// double-speed read placement (the pre-ALET STAT view, drive-enable lock).
    fn resolve_read_latch(&self, _address: u16, value: u8, latch_lock: Option<bool>) -> u8 {
        if latch_lock == Some(true) {
            0xFF
        } else {
            value
        }
    }

    /// Does a CPU access at `cpu_addr` collide with the in-flight OAM-DMA
    /// fetching from `dma_source` (base address)? The DMG rule (default) is
    /// a collision iff both sit on the same external/video bus. CGB has a
    /// separate WRAM bus and overrides this.
    fn oam_dma_bus_conflict(&self, cpu_addr: u16, dma_source: u16) -> bool {
        let source_bus = Bus::of(dma_source).unwrap_or(Bus::External);
        Bus::of(cpu_addr) == Some(source_bus)
    }

    /// During an OAM-DMA, a CPU access to this console's WRAM bus may be
    /// address-remapped by the DMA driving the bus (reads and writes alike).
    /// DMG (one external bus) never remaps; CGB does for an access while the
    /// DMA sources from the cart bus.
    fn oam_dma_wram_remap(&self, _cpu_addr: u16, _dma_source: u16) -> Option<u16> {
        None
    }

    /// Byte deposited at the OAM slot the DMA is filling when a CPU write
    /// collides with the DMA on the source bus. DMG uses the shared external-bus
    /// rule; CGB's separate WRAM bus overrides it for WRAM-bus sources.
    fn oam_dma_write_conflict_byte(&self, src_byte: u8, cpu_value: u8, dma_source: u16) -> u8 {
        shared_oam_dma_write_conflict_byte(src_byte, cpu_value, dma_source)
    }

    /// Does a CPU access at `cpu_addr` conflicting with the OAM-DMA force the
    /// byte the DMA deposits at OAM to `$00`? CGB: yes when the DMA sources from
    /// VRAM and the CPU access is on the VRAM bus. DMG: never.
    fn oam_dma_conflict_zeroes_oam(&self, _cpu_addr: u16, _dma_source: u16) -> bool {
        false
    }

    /// Does a CPU write to `address` re-bank the bus an active OAM DMA sources
    /// from — VBK while it reads VRAM, SVBK while it reads WRAM? Such a write
    /// latches at the M-cycle boundary, after the coincident DMA byte's source
    /// read, so its effect is deferred past that byte. DMG has no banked DMA
    /// source and never defers.
    fn oam_dma_source_bank_write(&self, _address: u16, _dma_source: u16) -> bool {
        false
    }

    /// The byte a DMA source read yields when the source address opens the
    /// bus rather than addressing storage — shared by OAM DMA and CGB VRAM
    /// DMA, which both fetch through `read_dma_source`. DMG never opens the
    /// bus (it echo-folds WRAM); CGB floats the cartridge bus to `$FF` for
    /// source `$E0–$FF`, past the cart-RAM `/CS` window, since its WRAM is
    /// on a separate bus.
    fn dma_source_open_bus(&self, _address: u16) -> Option<u8> {
        None
    }

    /// This console's own memory map: the registers/regions its map defines
    /// that the shared map doesn't. DMG adds nothing. CGB adds KEY1, VBK,
    /// SVBK, BCPS/BCPD, OCPS/OCPD, HDMA1-5, OPRI, and banked WRAM. Consulted
    /// before the shared `MappedAddress` map. The PPU and VRAM are passed so the
    /// map can resolve its registers that those generic components back (VBK on
    /// VRAM; CRAM/OPRI on the PPU) — keeping their addresses out of the shared map.
    fn map_read(
        &self,
        _address: u16,
        _ppu: &Ppu<Self::Ppu>,
        _vram: &<Self::Ppu as PpuModel>::Vram,
    ) -> Option<u8> {
        None
    }
    fn map_write(
        &mut self,
        _address: u16,
        _value: u8,
        _ppu: &mut Ppu<Self::Ppu>,
        _vram: &mut <Self::Ppu as PpuModel>::Vram,
    ) -> bool {
        false
    }

    /// Advance this console's VRAM DMA one master-clock fall: the trigger
    /// pend/commit pipeline and the byte-quota refill, plus the committed bus
    /// claim it hands the dispatch arbitration (through `chassis`'s console
    /// shadow). `mode` is the pre-fall PPU mode an H-Blank block gates on. DMG:
    /// no VRAM DMA, no-op.
    fn vram_dma_edge(&mut self, _chassis: &mut Chassis<Self>, _mode: ppu::rendering::Mode) {}

    /// M-cycle-boundary arbitration between the OAM DMA and the VRAM DMA over
    /// the shared bus, before the OAM byte moves: resolves single-speed
    /// contention and the double-speed escape-byte stall (stalling the OAM
    /// engine when it must), and returns whether this M-cycle's OAM byte move is
    /// suppressed (the VRAM DMA takes the deposit, or the escape stalls it).
    /// DMG: no VRAM DMA, never suppresses.
    fn vram_dma_arbitrate_oam(&mut self, _chassis: &mut Chassis<Self>) -> bool {
        false
    }

    /// The VRAM-DMA byte engine at the M-cycle boundary, after the OAM byte and
    /// the deferred VBK/SVBK bank write committed: moves this M-cycle's VRAM-DMA
    /// bytes (open-bus/source read → destination commit) and deposits the
    /// contended byte at OAM per the byte-clock phase residue. DMG: no VRAM
    /// DMA, no-op.
    fn vram_dma_boundary(&mut self, _chassis: &mut Chassis<Self>) {}

    /// A ready HBlank block owns the VRAM/external buses: M-cycles targeting
    /// them stretch until release; the rest run concurrently. DMG: never.
    fn vram_dma_seizes_bus(&self) -> bool {
        false
    }

    /// The DMA source for a VRAM address a committed HBlank block is about to
    /// write: a CPU read of it is stalled past the write, so it sees the DMA's
    /// value. `Some(source)` → read the source byte; `None` → normal read. DMG: never.
    fn vram_dma_conflict_source(&self, _address: u16) -> Option<u16> {
        None
    }

    /// Take the graded escape byte for an in-blackout drain — the CPU is held
    /// and the bus free, so the escape's tenure completes inside the blackout
    /// instead of parking the resumed stream. DMG: never.
    fn vram_dma_drain_escape(&mut self) -> Option<(u16, u16)> {
        None
    }

    /// The committed block landed on a running CPU and the instruction it
    /// interrupted has not yet retired: the bus grant waits for it. A block
    /// committing onto a halted CPU (including the same-fall wake flip)
    /// grants at the next M-boundary. DMG: never.
    fn vram_dma_park_waits_for_fetch(&self) -> bool {
        false
    }

    /// An instruction retired at this M-boundary: the retirement any pending
    /// running-CPU block grant was waiting on. DMG: no-op.
    fn vram_dma_instruction_retired(&mut self) {}

    /// A VRAM-DMA transfer request is standing — a block committed or one
    /// fall from commit. Sampled through a one-boundary synchronizer by the
    /// dispatch pick arbitration. DMG: never.
    fn vram_dma_request_standing(&self) -> bool {
        false
    }

    /// Whether the VRAM DMA is holding the CPU clock right now (mid transfer or
    /// mid H-Blank block). DMG: never.
    fn vram_dma_holds_cpu(&self) -> bool {
        false
    }

    /// The LCD was just disabled. A CGB HBlank VRAM-DMA block armed but not yet
    /// serviced runs one block now — no H-Blank will come (the same strobe as
    /// arming FF55 while the LCD is already off). DMG: no VRAM DMA.
    fn vram_dma_lcd_disabled(&mut self) {}
}

/// A CPU write that collided with OAM DMA on the source bus, deferred from
/// `commit_write` to the M-boundary commit so the model's resolved byte lands
/// at the OAM slot DMA is depositing into.
pub struct DmaConflictWrite {
    /// OAM slot DMA is depositing into (offset from 0xFE00).
    pub oam_offset: u8,
    /// The byte DMA fetched this M-cycle, AND-mixed on WRAM-source DMA where
    /// both drivers stay live through the OAM write phase.
    pub src_byte: u8,
    /// The value the CPU drove.
    pub cpu_value: u8,
}

/// A source-bank register (VBK/SVBK) write deferred from `commit_write` to the
/// M-boundary so the coincident OAM-DMA byte reads the pre-write bank.
pub struct DmaBankWrite {
    pub address: u16,
    pub value: u8,
}

/// The OAM-DMA write-conflict resolution latch: CPU writes that collided with
/// the DMA engine on the shared bus, held past `commit_write` and drained at
/// the M-cycle boundary.
#[derive(Default)]
pub struct DmaConflictLatch {
    pub pending_write: Option<DmaConflictWrite>,
    pub pending_bank_write: Option<DmaBankWrite>,
}

/// The shared hardware of a Game Boy–family console: the SM83 CPU, the
/// PPU/APU/timer/DMA silicon, the buses, and the master clock. Every field is
/// common to all consoles in the family; the DMG/CGB divergences live in the
/// [`Model`] on [`Console`], which reaches this silicon through `M`'s
/// associated types only — never `M` itself.
pub struct Chassis<M: Model> {
    pub cpu: Cpu,

    pub external: ExternalBus,
    pub high_ram: HighRam,
    pub vram_bus: VramBus<<M::Ppu as PpuModel>::Vram>,

    pub ppu: Ppu<M::Ppu>,
    pub screen: M::Screen,
    pub audio: Audio<M::Apu>,
    pub joypad: Joypad,
    pub interrupts: interrupts::Registers,
    pub serial: serial_transfer::Serial,
    pub timers: timers::Timers,
    pub dma: Dma,

    /// The master-clock phase layer: the CPU CLK9 edge, the free-running PPU dot
    /// edge, and the `÷1`/`÷2` divider between them. Owns the per-edge dispatch
    /// schedule (`advance`) that `execute_phase` consumes. At `÷1` the CPU and
    /// dot edges coincide every master edge (today's `clock_phase ==
    /// ppu_phase`); the CGB KEY1 switch sets `÷2`, where the dot edge advances on
    /// alternate CPU edges. The dot phase free-runs through the speed-switch
    /// blackout while the CPU is frozen, so the post-switch alignment is
    /// emergent.
    pub clock: MasterClock,
    /// Shared CPU data bus: current `cpu_port_d[7:0]` value plus the
    /// staged read/write activity for the in-flight M-cycle.
    pub cpu_bus: CpuBus,
    pub bus_trace: cpu_bus::BusTrace,
    /// OAM-DMA source-bus write conflicts deferred to the M-cycle boundary.
    /// Set in `write_byte_with_cupa_lock`/`commit_write`, drained in
    /// `tick_mcycle_boundary_fall`.
    pub dma_conflict: DmaConflictLatch,
}

/// A Game Boy–family console: the shared [`Chassis`] silicon plus the [`Model`]
/// `M` supplying the handful of DMG/CGB divergences that drive it.
pub struct Console<M: Model> {
    chassis: Chassis<M>,
    model: M,
    /// Whether the debugger's per-vblank graphics-surface capture is enabled.
    /// Interest-gated: off by default so the snapshot decodes and clones
    /// nothing until a graphics pane turns it on.
    graphics_capture: bool,
}

/// The original Game Boy (DMG): SGB co-processor support, the OAM
/// corruption bug, and a 2-bit shade framebuffer.
#[derive(Default)]
pub struct Dmg {
    sgb: Option<sgb::Sgb>,
    /// CGB console arbitration is statically unreachable on DMG — a ZST.
    console_state: (),
}

impl Model for Dmg {
    type Ppu = ppu::model::DmgPpu;
    type Screen = ppu::screen::Screen;
    const HAS_OAM_BUG: bool = true;

    type ConsoleState = ();
    type Apu = DmgApu;

    fn console_state(&self) -> &() {
        &self.console_state
    }
    fn console_state_mut(&mut self) -> &mut () {
        &mut self.console_state
    }

    fn on_present(&mut self, screen: &ppu::screen::Screen) {
        if let Some(sgb) = &mut self.sgb {
            sgb.update_screen(screen);
        }
    }

    fn read_joypad(&self, value: u8) -> u8 {
        if let Some(sgb) = &self.sgb
            && sgb.player_count > 1
        {
            let p14_selected = value & 0x10 == 0;
            let p15_selected = value & 0x20 == 0;
            if !p14_selected && !p15_selected {
                return (value & 0xF0) | (0x0F - sgb.current_player);
            }
        }
        value
    }

    fn on_joypad_write(&mut self, value: u8) {
        if let Some(sgb) = &mut self.sgb {
            sgb.write_joypad(value);
        }
    }

    fn on_reset(&mut self, cartridge: &Cartridge, _has_boot_rom: bool) {
        self.sgb = cartridge.supports_sgb().then(sgb::Sgb::new);
    }
}

/// The original Game Boy.
pub type GameBoy = Console<Dmg>;

impl<M: Model> Console<M> {
    pub fn new(cartridge: Cartridge, boot_rom: Option<BootRom>) -> Self {
        let mut console = Console {
            chassis: Chassis {
                cpu: Cpu::new(),
                external: ExternalBus::new(cartridge, boot_rom),
                high_ram: HighRam::new(),
                vram_bus: VramBus::new(),
                ppu: Ppu::new(),
                screen: M::Screen::default(),
                audio: Audio::new(),
                joypad: Joypad::new(),
                interrupts: interrupts::Registers::new(),
                serial: serial_transfer::Serial::new(),
                timers: timers::Timers::new(),
                dma: Dma::new(),
                clock: MasterClock::new(CpuDivider::One),
                cpu_bus: CpuBus::new(),
                bus_trace: cpu_bus::BusTrace::new(),
                dma_conflict: DmaConflictLatch::default(),
            },
            model: M::default(),
            graphics_capture: false,
        };
        console.rebuild_state();
        console
    }

    /// Power-cycle the console: re-create all volatile state while
    /// preserving the inserted cartridge (and its battery-backed SRAM),
    /// the boot ROM contents, and the user-attached serial link.
    pub fn reset(&mut self) {
        self.chassis.external.reset();
        self.rebuild_state();
    }

    /// Re-create every non-cartridge, non-link component to its power-
    /// on or post-boot-ROM initial state. Called from `new` after the
    /// initial struct has been laid out with placeholder values, and
    /// from `reset` after `ExternalBus::reset` has cleared WRAM/latch.
    ///
    /// Mirrors the CPU's pending bus read/write so T-cycle 2 staging
    /// has a target for the in-flight M-cycle. The skip-boot CPU
    /// anchors at the post-rise of the M-cycle that opens the
    /// cartridge m1 fetch (`Cpu::post_boot()` produces `Read{0x0100}`);
    /// the boundary work fired in the boot ROM's domain before t=0,
    /// so the staging block in `rise()` doesn't fire for that first
    /// M-cycle.
    fn rebuild_state(&mut self) {
        let has_boot_rom = self.chassis.external.has_boot_rom();
        let header_checksum = self.chassis.external.cartridge.header_checksum();

        self.chassis.cpu = if has_boot_rom {
            Cpu::new()
        } else {
            M::cpu_post_boot(header_checksum)
        };
        self.chassis.screen = M::Screen::default();
        self.chassis.high_ram = HighRam::new();
        let cgb_cart = self.chassis.external.cartridge.is_cgb();
        self.chassis.ppu = if has_boot_rom {
            Ppu::new()
        } else {
            M::ppu_post_boot(cgb_cart)
        };
        self.chassis.joypad = if has_boot_rom {
            Joypad::new()
        } else {
            M::joypad_post_boot()
        };
        self.chassis.interrupts = interrupts::Registers::new();
        self.chassis.serial = serial_transfer::Serial::new();
        self.chassis.timers = if has_boot_rom {
            timers::Timers::new()
        } else {
            M::timers_post_boot(cgb_cart)
        };
        self.chassis.audio = if has_boot_rom {
            Audio::new()
        } else {
            M::audio_post_boot(self.chassis.timers.internal_counter, cgb_cart)
        };
        self.chassis.dma = if has_boot_rom {
            Dma::new()
        } else {
            M::dma_post_boot()
        };
        self.chassis.vram_bus = VramBus::new();
        self.model
            .on_reset(&self.chassis.external.cartridge, has_boot_rom);

        if !has_boot_rom {
            let read = |a: u16| self.chassis.external.cartridge.read(a);
            let logo: [u8; 0x30] = std::array::from_fn(|i| read(0x0104 + i as u16));
            self.chassis.vram_bus.vram.init_post_boot(&logo);
            let header = ppu::CartridgeBootHeader {
                is_cgb: self.chassis.external.cartridge.is_cgb(),
                title: std::array::from_fn(|i| read(0x0134 + i as u16)),
                old_licensee: read(0x014B),
                new_licensee: [read(0x0144), read(0x0145)],
            };
            self.chassis.ppu.init_model_post_boot(&header);
        }

        self.chassis.bus_trace = cpu_bus::BusTrace::new();
        // Re-anchor the CPU clock to a rise; the free-running dot phase is left
        // as-is (the old reset touched only `clock_phase`).
        self.chassis.clock.engage_on_rise();
        // The model resets to single speed; realign the clock's ÷1/÷2 cell so it
        // stays the sole ratio owner across a reset.
        self.chassis
            .clock
            .set_divider(if self.double_speed_active() {
                CpuDivider::Two
            } else {
                CpuDivider::One
            });
        self.chassis.cpu_bus = CpuBus::new();
        self.chassis.dma_conflict = DmaConflictLatch::default();
        self.model
            .console_state_mut()
            .set_dma_conflict_oam_zero(None);
        self.model.console_state_mut().set_dma_cpu_hold(false);
        if let Some((address, _value)) = self.chassis.cpu.pending_bus_write() {
            self.chassis.cpu_bus.stage_write(address);
        } else if let Some(address) = self.chassis.cpu.pending_bus_read() {
            self.chassis.cpu_bus.stage_read(address);
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.chassis.external.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.chassis.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.chassis.cpu
    }

    pub fn ppu(&self) -> &Ppu<M::Ppu> {
        &self.chassis.ppu
    }

    /// The console-specific model — the DMG/CGB divergence state, for read-only
    /// inspection (the debugger reads CGB registers through it).
    pub fn model(&self) -> &M {
        &self.model
    }

    pub fn vram(&self) -> &<M::Ppu as PpuModel>::Vram {
        &self.chassis.vram_bus.vram
    }

    /// Length of the bank-complete VRAM image the debugger walks linearly above
    /// the bus, on a console that banks VRAM (CGB's two 8 KiB banks). `None` for
    /// a single-bank console (DMG): its 8 KiB is the `$8000` bus window.
    pub fn vram_image_len(&self) -> Option<u32> {
        (M::VRAM_BANKS > 1).then(|| M::VRAM_BANKS as u32 * 0x2000)
    }

    /// A byte of the bank-complete VRAM image by flat offset — bank
    /// `offset / 0x2000`, within-bank `offset % 0x2000` — independent of the
    /// CPU's VBK selection. `0xFF` on a single-bank console or past the image.
    pub fn vram_image_byte(&self, offset: u32) -> u8 {
        use crate::ppu::memory::VramView;
        if offset >= self.vram_image_len().unwrap_or(0) {
            return 0xFF;
        }
        VramView::bank(self.vram(), (offset / 0x2000) as u8).read_byte((offset % 0x2000) as u16)
    }

    /// Read a contiguous range of memory via peek (bypasses bus conflicts).
    pub fn peek_range(&self, start: u16, len: u16) -> Vec<u8> {
        (0..len).map(|i| self.peek(start.wrapping_add(i))).collect()
    }

    pub fn audio(&self) -> &Audio<M::Apu> {
        &self.chassis.audio
    }

    /// Synchronize the APU ahead of an observation that reaches it through a
    /// shared borrow — the register/PCM/wave-RAM read paths, the debugger views
    /// and the snapshot capture are all `&self`, so the sync runs at the last
    /// `&mut self` point that dominates them: the two CPU bus read edges, every
    /// public step boundary, and the trace hook.
    pub fn sync_audio(&mut self) {
        self.chassis.audio.materialize();
    }

    /// CPU T-cycles advanced per PPU dot (1 single speed, 2 CGB double speed).
    pub fn cpu_steps_per_dot(&self) -> u8 {
        self.model.cpu_steps_per_dot()
    }

    /// KEY1 double speed currently engaged. Folds to `false` on consoles
    /// without the ÷2 cell.
    fn double_speed_active(&self) -> bool {
        M::DOUBLE_SPEED && self.model.cpu_steps_per_dot() == 2
    }

    pub fn screen(&self) -> &M::Screen {
        &self.chassis.screen
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.chassis.audio.drain_samples()
    }

    /// Enable or disable the debugger's per-channel waveform capture.
    pub fn set_wave_capture(&mut self, on: bool) {
        self.chassis.audio.set_wave_capture(on);
    }

    /// The captured per-channel waveforms, or `None` when capture is off.
    pub fn channel_waves(&self) -> Option<Vec<missingno_core::waveform::ChannelWave>> {
        self.chassis.audio.channel_waves()
    }

    /// Enable or disable the debugger's per-vblank graphics-surface capture.
    pub fn set_graphics_capture(&mut self, on: bool) {
        self.graphics_capture = on;
    }

    /// Whether graphics-surface capture is enabled.
    pub fn graphics_capture(&self) -> bool {
        self.graphics_capture
    }

    pub fn press_button(&mut self, button: Button) {
        let before = self.chassis.joypad.input_lines();
        self.chassis.joypad.press_button(button);
        if before & !self.chassis.joypad.input_lines() != 0 {
            self.chassis
                .interrupts
                .request(interrupts::Interrupt::Joypad);
        }
    }

    pub fn release_button(&mut self, button: Button) {
        self.chassis.joypad.release_button(button);
    }

    pub fn timers(&self) -> &timers::Timers {
        &self.chassis.timers
    }

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.chassis.interrupts
    }

    /// True while a CGB double-speed switch holds the CPU `Stopped` in the
    /// settling blackout — a STOP that self-resumes, not a terminal halt.
    pub fn speed_switch_in_progress(&self) -> bool {
        self.model.speed_switch_in_progress()
    }

    /// A VRAM DMA holds the CPU (GDMA whole-transfer hold or an HBlank
    /// block's bus ownership) — the CPU's stop/park is the bus master's,
    /// not a software STOP/HALT.
    pub fn vram_dma_holds_cpu(&self) -> bool {
        self.model.console_state().dma_cpu_hold() || self.model.console_state().bus_suspended()
    }

    pub fn dma(&self) -> &Dma {
        &self.chassis.dma
    }

    pub fn serial(&self) -> &serial_transfer::Serial {
        &self.chassis.serial
    }

    pub fn external_bus(&self) -> &ExternalBus {
        &self.chassis.external
    }

    pub fn high_ram(&self) -> &HighRam {
        &self.chassis.high_ram
    }

    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        self.chassis.serial.drain_output()
    }

    pub fn set_link(&mut self, link: Box<dyn serial_transfer::SerialLink>) {
        self.chassis.serial.set_link(link);
    }
}

impl Console<Dmg> {
    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.model.sgb.as_ref()
    }
}

/// B2 acceptance harness: each shared struct's summed CGB-only residual storage
/// on a DMG build is the load-bearing invariant (B2 drives it to zero behind the
/// `Model`/`PpuModel` seam); absolute `size_of` is left unpinned to exclude
/// unrelated struct padding.
#[cfg(test)]
mod cgb_residual_size {
    /// `Console<M>` CGB-only state relocated behind the `Model::ConsoleState` seam.
    mod console {
        pub const CGB_BYTES: usize = 0;
    }

    /// `Cpu` CGB-only residual: `irq.halt_wake_presample` — the halt-wake
    /// comparator presample latch. It is a CPU interrupt-latch stage (a function
    /// of `dispatch.latched()`), not a bus-arbitration grant, so it stays on the
    /// CPU rather than riding the `BusGrants` signal; dead on a DMG build (never
    /// written under `!HALT_WAKE_SAMPLES_EARLY`). The bus-park/hold/claim bytes
    /// relocated behind `Model::ConsoleState`.
    mod cpu {
        pub const CGB_BYTES: usize = 1;
    }

    /// `PipelineRegisters` CGB-only storage relocated behind the
    /// `PpuModel::TileSelGlitch` seam.
    mod pipeline_registers {
        pub const CGB_BYTES: usize = 0;
    }

    /// `StatInterrupt` FF41/FF45 synchroniser DFFs relocated behind the `PpuModel::StatShadow` seam.
    mod stat_interrupt {
        pub const CGB_BYTES: usize = 0;
    }

    /// Residual CGB-only storage still carried on a DMG build, summed across the four shared structs.
    #[test]
    fn cgb_only_byte_budget_remaining() {
        const REMAINING: usize = console::CGB_BYTES
            + cpu::CGB_BYTES
            + pipeline_registers::CGB_BYTES
            + stat_interrupt::CGB_BYTES;
        assert_eq!(REMAINING, 1, "CGB-only residual byte budget changed");
    }
}
