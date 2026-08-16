use crate::audio::{ApuSpec, Audio};
use crate::cartridge::Cartridge;
use crate::chassis::Chassis;
use crate::cpu::Cpu;
use crate::dma::Dma;
use crate::joypad::Joypad;
use crate::memory::{Bus, ExternalBus};
use crate::ppu::model::PpuModel;
use crate::ppu::{self, Ppu};
use crate::screen::{ConsoleShadow, ScreenBuffer};
use crate::timers;

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
/// Everything not listed here is the same silicon and lives in [`Console`](crate::Console).
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
