use crate::audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use cpu::mcycle::{BusDot, DotAction};
use dma::Dma;
use joypad::{Button, Joypad};
use memory::{ExternalBus, HighRam, VramBus};
use ppu::{Ppu, screen::Screen};

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
pub mod memory;
pub mod ppu;
pub mod recording;
pub mod serial_transfer;
pub mod sgb;
#[cfg(feature = "gbtrace")]
pub mod snapshot;
pub mod timers;
#[cfg(feature = "gbtrace")]
pub mod trace;

use cpu_bus::{BusAccess, CpuBus};

/// Master clock signal level. The clock alternates High → Low
/// uniformly. Edge logic runs at transitions: `rise()` at the
/// Low→High edge, `fall()` at the High→Low edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockPhase {
    High,
    Low,
}

impl ClockPhase {
    pub fn next(self) -> ClockPhase {
        match self {
            ClockPhase::High => ClockPhase::Low,
            ClockPhase::Low => ClockPhase::High,
        }
    }
}

pub struct GameBoy {
    cpu: Cpu,
    screen: Screen,

    external: ExternalBus,
    high_ram: HighRam,
    ppu: Ppu,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Registers,
    link: Box<dyn serial_transfer::SerialLink>,
    timers: timers::Timers,
    dma: Dma,
    sgb: Option<sgb::Sgb>,
    vram_bus: VramBus,

    /// Last read value from the bus, persisted across dots for step_dot().
    last_read_value: u8,
    bus_trace: Option<Vec<BusAccess>>,

    /// Master clock phase — alternates Rising/Falling uniformly.
    clock_phase: ClockPhase,
    /// Action for the current dot, set on Rising and consumed during Falling.
    current_dot_action: DotAction,
    /// Dot position for the current dot, set on Rising and consumed during Falling.
    current_dot: BusDot,
    /// A CPU bus write staged at the M-cycle boundary, applied at
    /// dot 2 to drive `cpu_bus.data` with the write value (CUPA-
    /// rising / `cpu_wr` asserted equivalent).
    staged_bus_write: Option<cpu_bus::StagedBusWrite>,
    /// A CPU bus read staged at the M-cycle boundary, applied at
    /// dot 2 to drive the value onto `cpu_bus`.
    staged_bus_read: Option<cpu_bus::StagedBusRead>,
    /// Pending OAM-corruption arming. Set when an OAM-range bus
    /// address (CPU read/write or IDU step) appears on the bus;
    /// cleared and applied at the next MOPA (dot 2 rise) — which
    /// may belong to the next instruction.
    pending_oam_bug: Option<execute::OamBugKind>,
    /// Shared CPU data bus state.
    cpu_bus: CpuBus,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge, boot_rom: Option<Box<[u8; 256]>>) -> GameBoy {
        let mut gb = GameBoy {
            cpu: Cpu::new(0),
            screen: Screen::default(),
            external: ExternalBus::new(cartridge, boot_rom),
            high_ram: HighRam::new(),
            ppu: Ppu::new(),
            audio: Audio::new(),
            joypad: Joypad::new(),
            interrupts: interrupts::Registers::new(),
            serial: serial_transfer::Registers::new(),
            link: Box::new(serial_transfer::Disconnected::new()),
            timers: timers::Timers::new(),
            dma: Dma::new(),
            sgb: None,
            vram_bus: VramBus::new(),
            last_read_value: 0,
            bus_trace: None,
            clock_phase: ClockPhase::Low,
            current_dot_action: DotAction::Idle,
            current_dot: BusDot::ZERO,
            staged_bus_write: None,
            staged_bus_read: None,
            pending_oam_bug: None,
            cpu_bus: CpuBus::new(),
        };
        gb.rebuild_state();
        gb
    }

    /// Power-cycle the console: re-create all volatile state while
    /// preserving the inserted cartridge (and its battery-backed SRAM),
    /// the boot ROM contents, and the user-attached serial link.
    pub fn reset(&mut self) {
        self.external.reset();
        self.rebuild_state();
    }

    /// Re-create every non-cartridge, non-link component to its post-
    /// boot or power-on initial state. Called from `new` after the
    /// initial struct has been laid out with placeholder values, and
    /// from `reset` after `ExternalBus::reset` has cleared WRAM/latch.
    ///
    /// Mirrors the CPU's pending bus read/write so dot-2 staging has a
    /// target for the in-flight M-cycle. The skip-boot CPU anchors at
    /// the post-rise of the M-cycle that opens the cartridge m1 fetch
    /// (`Cpu::new()` produces `Read{0x0100}`); the boundary work fired
    /// in the boot ROM's domain before t=0, so the staging block in
    /// `rise()` doesn't fire for that first M-cycle.
    fn rebuild_state(&mut self) {
        let has_boot_rom = self.external.has_boot_rom();
        let header_checksum = self.external.cartridge.header_checksum();
        let supports_sgb = self.external.cartridge.supports_sgb();

        self.cpu = if has_boot_rom {
            Cpu::power_on()
        } else {
            Cpu::new(header_checksum)
        };
        self.screen = Screen::default();
        self.high_ram = HighRam::new();
        self.ppu = if has_boot_rom {
            Ppu::power_on()
        } else {
            Ppu::new()
        };
        self.audio = if has_boot_rom {
            Audio::power_on()
        } else {
            Audio::new()
        };
        self.joypad = Joypad::new();
        self.interrupts = interrupts::Registers::new();
        self.serial = serial_transfer::Registers::new();
        self.timers = if has_boot_rom {
            timers::Timers::power_on()
        } else {
            timers::Timers::new()
        };
        self.dma = Dma::new();
        self.vram_bus = VramBus::new();
        self.sgb = supports_sgb.then(sgb::Sgb::new);

        if !has_boot_rom {
            self.init_post_boot_vram();
        }

        self.last_read_value = 0;
        self.bus_trace = None;
        self.clock_phase = ClockPhase::Low;
        self.current_dot_action = DotAction::Idle;
        self.current_dot = BusDot::ZERO;
        self.staged_bus_read = self.cpu.pending_bus_read().map(cpu_bus::StagedBusRead::new);
        self.staged_bus_write = self
            .cpu
            .pending_bus_write()
            .map(|(address, _value)| cpu_bus::StagedBusWrite::new(address));
        self.pending_oam_bug = None;
        self.cpu_bus = CpuBus::new();
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

    pub fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    pub fn vram(&self) -> &ppu::memory::Vram {
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

    pub fn screen(&self) -> &Screen {
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

    pub fn dma(&self) -> &Dma {
        &self.dma
    }

    pub fn serial(&self) -> &serial_transfer::Registers {
        &self.serial
    }

    pub fn external_bus(&self) -> &ExternalBus {
        &self.external
    }

    pub fn high_ram(&self) -> &HighRam {
        &self.high_ram
    }

    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.sgb.as_ref()
    }

    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        self.link.drain_output()
    }

    pub fn set_link(&mut self, link: Box<dyn serial_transfer::SerialLink>) {
        self.link = link;
    }

    fn init_post_boot_vram(&mut self) {
        let mut logo = [0u8; 0x30];
        for (i, byte) in logo.iter_mut().enumerate() {
            *byte = self.external.cartridge.read(0x0104 + i as u16);
        }
        self.vram_bus.vram.init_post_boot(&logo);
    }
}
