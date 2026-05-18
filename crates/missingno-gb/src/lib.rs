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
pub mod timers;
#[cfg(feature = "gbtrace")]
pub mod trace;

use audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use cpu_bus::CpuBus;
use dma::Dma;
use joypad::{Button, Joypad};
use memory::{ExternalBus, HighRam, VramBus};
use ppu::{Ppu, screen::Screen};

pub use master_clock::ClockPhase;

pub struct GameBoy {
    cpu: Cpu,

    external: ExternalBus,
    high_ram: HighRam,
    vram_bus: VramBus,

    ppu: Ppu,
    screen: Screen,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Serial,
    timers: timers::Timers,
    dma: Dma,
    sgb: Option<sgb::Sgb>,

    /// Master clock signal level. Toggles each half-T-cycle.
    clock_phase: ClockPhase,
    /// Shared CPU data bus: current `cpu_port_d[7:0]` value plus the
    /// staged read/write activity for the in-flight M-cycle.
    cpu_bus: CpuBus,
    bus_trace: cpu_bus::BusTrace,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge, boot_rom: Option<Box<[u8; 256]>>) -> GameBoy {
        let mut gb = GameBoy {
            cpu: Cpu::new(),
            external: ExternalBus::new(cartridge, boot_rom),
            high_ram: HighRam::new(),
            vram_bus: VramBus::new(),
            ppu: Ppu::new(),
            screen: Screen::default(),
            audio: Audio::new(),
            joypad: Joypad::new(),
            interrupts: interrupts::Registers::new(),
            serial: serial_transfer::Serial::new(),
            timers: timers::Timers::new(),
            dma: Dma::new(),
            sgb: None,
            clock_phase: ClockPhase::Low,
            cpu_bus: CpuBus::new(),
            bus_trace: cpu_bus::BusTrace::new(),
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
        let supports_sgb = self.external.cartridge.supports_sgb();

        self.cpu = if has_boot_rom {
            Cpu::new()
        } else {
            Cpu::post_boot(header_checksum)
        };
        self.screen = Screen::default();
        self.high_ram = HighRam::new();
        self.ppu = if has_boot_rom {
            Ppu::new()
        } else {
            Ppu::post_boot()
        };
        self.audio = if has_boot_rom {
            Audio::new()
        } else {
            Audio::post_boot()
        };
        self.joypad = Joypad::new();
        self.interrupts = interrupts::Registers::new();
        self.serial = serial_transfer::Serial::new();
        self.timers = if has_boot_rom {
            timers::Timers::new()
        } else {
            timers::Timers::post_boot()
        };
        self.dma = Dma::new();
        self.vram_bus = VramBus::new();
        self.sgb = supports_sgb.then(sgb::Sgb::new);

        if !has_boot_rom {
            let logo: [u8; 0x30] =
                std::array::from_fn(|i| self.external.cartridge.read(0x0104 + i as u16));
            self.vram_bus.vram.init_post_boot(&logo);
        }

        self.bus_trace = cpu_bus::BusTrace::new();
        self.clock_phase = ClockPhase::Low;
        self.cpu_bus = CpuBus::new();
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

    pub fn serial(&self) -> &serial_transfer::Serial {
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
        self.serial.drain_output()
    }

    pub fn set_link(&mut self, link: Box<dyn serial_transfer::SerialLink>) {
        self.serial.set_link(link);
    }
}
