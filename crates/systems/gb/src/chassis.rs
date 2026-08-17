use crate::audio::{ApuSpec, Audio};
use crate::cartridge::Cartridge;
use crate::clock::{CpuDivider, MasterClock};
use crate::cpu::Cpu;
use crate::cpu_bus::{self, CpuBus};
use crate::dma::Dma;
use crate::joypad::{Button, Joypad};
use crate::memory::{BootRom, ExternalBus, HighRam, VramBus};
use crate::model::Model;
use crate::ppu::Ppu;
use crate::ppu::memory::Vram;
use crate::ppu::model::PpuModel;
use crate::screen::ConsoleShadow;
use crate::{interrupts, ppu, serial_transfer, timers};

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
    /// schedule (`advance`) that `execute_tcycle` consumes. At `÷1` the CPU and
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
    /// Set in `write_byte_with_write_strobe_lock`/`commit_write`, drained in
    /// `tick_mcycle_boundary_fall`.
    pub dma_conflict: DmaConflictLatch,
}

/// A Game Boy–family console: the shared [`Chassis`] silicon plus the [`Model`]
/// `M` supplying the handful of DMG/CGB divergences that drive it.
pub struct Console<M: Model> {
    pub(crate) chassis: Chassis<M>,
    pub(crate) model: M,
    /// Whether the debugger's per-vblank graphics-surface capture is enabled.
    /// Interest-gated: off by default so the snapshot decodes and clones
    /// nothing until a graphics pane turns it on.
    pub(crate) graphics_capture: bool,
}

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
    /// `&mut self` point that dominates them: the two CPU bus read edges, the
    /// run→observe boundaries (a completed frame, a debugger step), and the
    /// trace hook. A free-running frame never syncs mid-flight.
    pub fn sync_audio(&mut self) {
        self.chassis.audio.materialize();
    }

    /// Synchronize the PPU ahead of an observation that reaches LX or the
    /// divider phase — the save-state and morepork `dot_position` columns, the
    /// debugger's pipeline views, the CGB STOP phase. Everything else a span
    /// touches is constant across it, so an ordinary bus read (LY and STAT
    /// polls included) needs no sync. Placed like [`Console::sync_audio`], at
    /// the last `&mut self` point dominating each `&self` observation.
    pub fn sync_ppu(&mut self) {
        self.chassis.ppu.sync_span();
    }

    /// CPU T-cycles advanced per PPU dot (1 single speed, 2 CGB double speed).
    pub fn cpu_steps_per_dot(&self) -> u8 {
        self.model.cpu_steps_per_dot()
    }

    /// KEY1 double speed currently engaged. Folds to `false` on consoles
    /// without the ÷2 cell.
    pub(crate) fn double_speed_active(&self) -> bool {
        <M::Apu as ApuSpec>::DOUBLE_SPEED && self.model.cpu_steps_per_dot() == 2
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
