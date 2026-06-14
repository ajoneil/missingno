pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod cpu_bus;
pub mod debugger;
pub mod dma;
pub mod dmg_sram;
pub mod execute;
pub mod interrupts;
pub mod joypad;
pub mod master_clock;
pub mod memory;
pub mod ppu;
pub mod recording;
pub mod serial_transfer;
pub mod sgb;
#[cfg(feature = "gbtrace")]
pub mod snapshot;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod timers;
#[cfg(feature = "gbtrace")]
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
pub use master_clock::ClockPhase;
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
}

/// What a STOP the CPU has settled into resolves to (decided by the model).
pub enum StopAction {
    /// Stay stopped — DMG stop-mode, or CGB STOP with no armed speed switch.
    Remain,
    /// CGB double-speed switch: the model has toggled its speed; the system
    /// resets the divider and re-engages the CPU.
    SpeedSwitch,
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

/// The per-console divergences from the shared SM83 silicon — the entire
/// catalogue of how DMG and CGB differ in the step loop and memory map.
/// Everything not listed here is the same silicon and lives in [`Console`].
pub trait Model: Default {
    /// The PPU's per-console hardware: DMG monochrome, CGB colour.
    type Ppu: PpuModel;

    /// Framebuffer storage; its pixel matches what `Self::Ppu` resolves
    /// (DMG = `PaletteIndex` shades, CGB = RGB555).
    type Screen: ScreenBuffer<Pixel = <Self::Ppu as PpuModel>::Pixel>;

    /// DMG arms/fires the OAM-corruption bug (BOWA/CUFE); CGB silicon has none.
    const HAS_OAM_BUG: bool = false;

    /// How the CPU couples to CH3's wave SRAM while the channel is active:
    /// DMG only during the fetch strobe, CGB always at the channel's byte.
    const WAVE_RAM_COUPLING: WaveRamCoupling = WaveRamCoupling::FetchStrobe;

    /// CGB silicon exposes the APU channel DAC outputs at FF76/FF77.
    const HAS_PCM_REGISTERS: bool = false;

    /// CGB's halt-release comparator samples IF&IE two T-cycles before
    /// the M-cycle boundary; DMG samples at the boundary.
    fn halt_wake_samples_early(&self) -> bool {
        false
    }

    /// Hardware revision name recorded in gbtrace captures.
    const TRACE_MODEL_NAME: &'static str = "DMG-B";

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

    /// Post-boot joypad state — the CGB boot ROM deselects both key matrix
    /// lines; the DMG boot ROM leaves both selected.
    fn joypad_post_boot() -> Joypad {
        Joypad::new()
    }

    /// Post-boot OAM-DMA state — the CGB boot ROM leaves FF46 reading 0.
    fn dma_post_boot() -> Dma {
        Dma::new()
    }

    /// Resolve a STOP the CPU has settled into. DMG always stays stopped;
    /// CGB performs a double-speed switch when KEY1 is armed (toggling its
    /// own speed bit, arming its blackout) and otherwise stays stopped.
    fn resolve_stop(&mut self) -> StopAction {
        StopAction::Remain
    }

    /// Whether a double-speed switch blackout is draining (the CPU is held
    /// `Stopped` while the divider/PPU run). DMG: never.
    fn speed_switch_in_progress(&self) -> bool {
        false
    }

    /// Drain `elapsed` CPU T-cycles from the switch blackout; returns true on
    /// the M-cycle it empties (the CPU re-engages at the new speed). DMG: never.
    fn drain_speed_switch_blackout(&mut self, _elapsed: u32) -> bool {
        false
    }

    /// CPU T-cycles advanced per PPU dot. 1 = lockstep (DMG always; CGB
    /// single speed); 2 = the CPU clock runs at twice the dot clock (CGB
    /// double speed), so a full CPU T-cycle lands on each master-clock edge.
    fn cpu_steps_per_dot(&self) -> u8 {
        1
    }

    /// CPU T-cycles the CPU stays `Stopped` across a double-speed switch (the
    /// blackout while the divider/PPU keep running). DMG never switches speed.
    fn speed_switch_blackout_tcycles(&self) -> u32 {
        0
    }

    /// Extra PPU master edges the switch's clock dynamics give the dot clock
    /// while the CPU is mid-switch — the post-switch CPU↔PPU re-phase. Advances
    /// `ppu_phase` against the held `clock_phase`. Queried after `resolve_stop`
    /// returns `SpeedSwitch` (the model's speed bit already holds the new
    /// speed). DMG never switches.
    fn speed_switch_ppu_nudge_edges(&self) -> u32 {
        0
    }

    /// Sampled at the top of `rise_work`, before this dot's `ppu_rise_edge`
    /// (the ALET-rising XYMU.q↑ / OAM-lock-onset ALET edge). `rendering` is the
    /// pre-ALET-rise XYMU (mode-3) state — the only STAT mode signal VOGA captures one
    /// ALET edge too far on the Low arm; `read_lock` is the pre-ALET-edge lock of a
    /// pending OAM/VRAM read (`None` when no lockable read is staged). A model
    /// whose CPU latch can land on the same phase as the ALET edge stores these
    /// to resolve that read's `data_phase_n↑` to the pre-ALET-edge view. DMG (the CPU
    /// latch always lands after a separate-phase rise) ignores them.
    fn note_pre_alet_read_view(&mut self, _rendering: bool, _read_lock: Option<bool>) {}

    /// A pending OAM read's lock, sampled when the CPU asserts the address
    /// (the M-cycle's first T-cycle). The OAM decoder grants the read there —
    /// a RUTU lock onset later in the M-cycle cannot float a read already
    /// granted. DMG ignores it.
    fn note_read_address_phase(&mut self, _oam_lock: Option<bool>) {}

    /// A pending OAM read's lock at the drive enable (tobe↑, the read's third
    /// T-cycle fall), sampled before that fall's PPU advance applies any lock
    /// onset. DMG ignores it.
    fn note_read_drive_phase(&mut self, _oam_lock: Option<bool>) {}

    /// Resolve the value a CPU read latches. A lockable (OAM/VRAM) read
    /// arrives unfloated with its live lock in `latch_lock`; the model owns
    /// the float. DMG floats on the latch-edge lock alone. CGB also consults
    /// its address-phase / pre-ALET-edge samples (`on_low_arm` marks the
    /// double-speed Low master-arm, where the latch shares its phase with the
    /// ALET edge and resolves to the pre-ALET-edge view).
    fn resolve_read_latch(
        &self,
        _address: u16,
        value: u8,
        _on_low_arm: bool,
        latch_lock: Option<bool>,
    ) -> u8 {
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

    /// Advance this console's VRAM DMA one M-cycle, refilling the bytes it may
    /// move this M-cycle. `mode` lets an H-Blank transfer gate on mode 0.
    /// Returns the claim committed this fall: `committed` (the cancel-immune
    /// bus claim) and `standing` (the claim aged through its synchronizer —
    /// it wins the bus race against a halt-release fetch). DMG: no VRAM DMA.
    fn vram_dma_tick(
        &mut self,
        _mode: ppu::rendering::Mode,
        _engine_gated: bool,
        _cpu_halted: bool,
    ) -> VramDmaClaim {
        VramDmaClaim::default()
    }

    /// A ready HBlank block owns the VRAM/external buses: M-cycles targeting
    /// them stretch until release; the rest run concurrently. DMG: never.
    fn vram_dma_seizes_bus(&self) -> bool {
        false
    }

    /// An entry-triggered block spends one leading no-data cell — the engine
    /// loading its working pointers from the HDMA1-4 holding registers (the
    /// FF55 arm strobe performs that load itself). Consumed once per block.
    fn vram_dma_take_setup_cell(&mut self) -> bool {
        false
    }

    /// The next byte the VRAM DMA moves this M-cycle — `(source, destination)`
    /// resolved addresses — advancing its cursor. `None` once this M-cycle's
    /// quota is spent. DMG: never.
    fn vram_dma_next_byte(&mut self) -> Option<(u16, u16)> {
        None
    }

    /// Whether the VRAM DMA is holding the CPU clock right now (mid transfer or
    /// mid H-Blank block). DMG: never.
    fn vram_dma_holds_cpu(&self) -> bool {
        false
    }
}

/// A Game Boy–family console: the SM83 CPU, the shared PPU/APU/timer/DMA
/// silicon, and the step loop + memory map that drive them. The handful of
/// DMG/CGB divergences are supplied by the [`Model`] parameter `M`.
pub struct Console<M: Model> {
    cpu: Cpu,

    external: ExternalBus,
    high_ram: HighRam,
    vram_bus: VramBus<<M::Ppu as PpuModel>::Vram>,

    ppu: Ppu<M::Ppu>,
    screen: M::Screen,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Serial,
    timers: timers::Timers,
    dma: Dma,

    /// Master clock signal level. Toggles each half-T-cycle.
    clock_phase: ClockPhase,
    /// The PPU's own master-clock phase: which edge (`Low` = rise, `High` =
    /// fall) the PPU advances next. The PPU is a free-running state machine on
    /// the master clock, so this toggles every PPU step independently of the
    /// CPU — during the speed-switch blackout it keeps advancing while the CPU
    /// is frozen, so the post-switch CPU↔PPU alignment is emergent (no slip).
    /// At single speed it tracks `clock_phase`; double speed is where it
    /// diverges.
    ppu_phase: ClockPhase,
    /// Shared CPU data bus: current `cpu_port_d[7:0]` value plus the
    /// staged read/write activity for the in-flight M-cycle.
    cpu_bus: CpuBus,
    bus_trace: cpu_bus::BusTrace,
    /// Conflict write deferred from `commit_write` to after DMA's
    /// `mcycle()` commit. Tuple is `(oam_offset, src_byte, cpu_value)`:
    /// `src_byte` is the byte DMA fetched this M-cycle, used to
    /// AND-mix on WRAM-source DMA where both drivers stay live through
    /// the OAM write phase. Set in `write_byte_with_cupa_lock`, drained
    /// in `tick_mcycle_boundary_fall`.
    dma_conflict_write_pending: Option<(u8, u8, u8)>,

    /// OAM offset whose DMA-deposited byte a VRAM-source bus conflict forces to
    /// `$00` (CGB). Set by a conflicting read or write on the VRAM bus, drained
    /// in `tick_mcycle_boundary_fall`.
    dma_conflict_oam_zero: Option<u8>,

    /// A CGB VRAM DMA is holding the CPU clock this M-cycle (bus master owns the
    /// bus). The CPU spins in `Stopped` and the DMA's bytes flow per M-cycle;
    /// `manage_dma_hold` releases it when the DMA stops asserting the hold.
    dma_cpu_hold: bool,

    model: M,
}

/// The original Game Boy (DMG): SGB co-processor support, the OAM
/// corruption bug, and a 2-bit shade framebuffer.
#[derive(Default)]
pub struct Dmg {
    sgb: Option<sgb::Sgb>,
}

impl Model for Dmg {
    type Ppu = ppu::model::DmgPpu;
    type Screen = ppu::screen::Screen;
    const HAS_OAM_BUG: bool = true;

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
            clock_phase: ClockPhase::Low,
            ppu_phase: ClockPhase::Low,
            cpu_bus: CpuBus::new(),
            bus_trace: cpu_bus::BusTrace::new(),
            dma_conflict_write_pending: None,
            dma_conflict_oam_zero: None,
            dma_cpu_hold: false,
            model: M::default(),
        };
        console.rebuild_state();
        console
    }

    /// Power-cycle the console: re-create all volatile state while
    /// preserving the inserted cartridge (and its battery-backed SRAM),
    /// the boot ROM contents, and the user-attached serial link.
    pub fn reset(&mut self) {
        self.external.reset();
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
        let has_boot_rom = self.external.has_boot_rom();
        let header_checksum = self.external.cartridge.header_checksum();

        self.cpu = if has_boot_rom {
            Cpu::new()
        } else {
            M::cpu_post_boot(header_checksum)
        };
        self.screen = M::Screen::default();
        self.high_ram = HighRam::new();
        let cgb_cart = self.external.cartridge.is_cgb();
        self.ppu = if has_boot_rom {
            Ppu::new()
        } else {
            M::ppu_post_boot(cgb_cart)
        };
        self.joypad = if has_boot_rom {
            Joypad::new()
        } else {
            M::joypad_post_boot()
        };
        self.interrupts = interrupts::Registers::new();
        self.serial = serial_transfer::Serial::new();
        self.timers = if has_boot_rom {
            timers::Timers::new()
        } else {
            M::timers_post_boot(cgb_cart)
        };
        self.audio = if has_boot_rom {
            Audio::new()
        } else {
            Audio::post_boot(self.timers.internal_counter)
        };
        self.dma = if has_boot_rom {
            Dma::new()
        } else {
            M::dma_post_boot()
        };
        self.vram_bus = VramBus::new();
        self.model.on_reset(&self.external.cartridge, has_boot_rom);

        if !has_boot_rom {
            let read = |a: u16| self.external.cartridge.read(a);
            let logo: [u8; 0x30] = std::array::from_fn(|i| read(0x0104 + i as u16));
            self.vram_bus.vram.init_post_boot(&logo);
            let header = ppu::CartridgeBootHeader {
                is_cgb: self.external.cartridge.is_cgb(),
                title: std::array::from_fn(|i| read(0x0134 + i as u16)),
                old_licensee: read(0x014B),
                new_licensee: [read(0x0144), read(0x0145)],
            };
            self.ppu.init_model_post_boot(&header);
        }

        self.bus_trace = cpu_bus::BusTrace::new();
        self.clock_phase = ClockPhase::Low;
        self.cpu_bus = CpuBus::new();
        self.dma_conflict_write_pending = None;
        self.dma_conflict_oam_zero = None;
        self.dma_cpu_hold = false;
        if let Some((address, _value)) = self.cpu.pending_bus_write() {
            self.cpu_bus.stage_write(address);
        } else if let Some(address) = self.cpu.pending_bus_read() {
            self.cpu_bus.stage_read(address);
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.external.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    pub fn ppu(&self) -> &Ppu<M::Ppu> {
        &self.ppu
    }

    pub fn vram(&self) -> &<M::Ppu as PpuModel>::Vram {
        &self.vram_bus.vram
    }

    /// Read a contiguous range of memory via peek (bypasses bus conflicts).
    pub fn peek_range(&self, start: u16, len: u16) -> Vec<u8> {
        (0..len).map(|i| self.peek(start.wrapping_add(i))).collect()
    }

    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    pub fn clock_phase(&self) -> ClockPhase {
        self.clock_phase
    }

    /// CPU T-cycles advanced per PPU dot (1 single speed, 2 CGB double speed).
    pub fn cpu_steps_per_dot(&self) -> u8 {
        self.model.cpu_steps_per_dot()
    }

    pub fn screen(&self) -> &M::Screen {
        &self.screen
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.audio.drain_samples()
    }

    pub fn press_button(&mut self, button: Button) {
        let before = self.joypad.input_lines();
        self.joypad.press_button(button);
        if before & !self.joypad.input_lines() != 0 {
            self.interrupts.request(interrupts::Interrupt::Joypad);
        }
    }

    pub fn release_button(&mut self, button: Button) {
        self.joypad.release_button(button);
    }

    pub fn timers(&self) -> &timers::Timers {
        &self.timers
    }

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.interrupts
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
        self.dma_cpu_hold || self.cpu.bus_suspended
    }

    pub fn dma(&self) -> &Dma {
        &self.dma
    }

    pub fn serial(&self) -> &serial_transfer::Serial {
        &self.serial
    }

    pub fn external_bus(&self) -> &ExternalBus {
        &self.external
    }

    pub fn high_ram(&self) -> &HighRam {
        &self.high_ram
    }

    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        self.serial.drain_output()
    }

    pub fn set_link(&mut self, link: Box<dyn serial_transfer::SerialLink>) {
        self.serial.set_link(link);
    }
}

impl Console<Dmg> {
    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.model.sgb.as_ref()
    }
}
