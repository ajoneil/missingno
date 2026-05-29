//! Game Boy Color emulation.
//!
//! This crate models the CGB as a distinct system that reuses the
//! shared SM83-based hardware modules from `missingno-gb`. CGB-specific
//! behaviour (palette memory, VRAM/WRAM banking, double-speed, HDMA,
//! object priority) lives here.
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

pub mod execute;
pub mod memory;
pub mod screen;
#[cfg(feature = "gbtrace")]
pub mod trace;

use missingno_gb::{
    ClockPhase,
    audio::Audio,
    cartridge::Cartridge,
    cpu::Cpu,
    cpu_bus::{self, BusTrace, CpuBus},
    dma::Dma,
    interrupts,
    joypad::{Button, Joypad},
    memory::{ExternalBus, HighRam, VramBus},
    ppu::{self, Ppu},
    serial_transfer::{Serial, SerialLink},
    timers::Timers,
};

use crate::screen::Screen;

pub struct GameBoyColor {
    cpu: Cpu,

    external: ExternalBus,
    high_ram: HighRam,
    vram_bus: VramBus,

    ppu: Ppu,
    screen: Screen,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: Serial,
    timers: Timers,
    dma: Dma,

    clock_phase: ClockPhase,
    cpu_bus: CpuBus,
    bus_trace: BusTrace,
}

impl GameBoyColor {
    pub fn new(cartridge: Cartridge, boot_rom: Option<Box<[u8; 256]>>) -> GameBoyColor {
        let mut gbc = GameBoyColor {
            cpu: Cpu::new(),
            external: ExternalBus::new(cartridge, boot_rom),
            high_ram: HighRam::new(),
            vram_bus: VramBus::new(),
            ppu: Ppu::new(),
            screen: Screen::default(),
            audio: Audio::new(),
            joypad: Joypad::new(),
            interrupts: interrupts::Registers::new(),
            serial: Serial::new(),
            timers: Timers::new(),
            dma: Dma::new(),
            clock_phase: ClockPhase::Low,
            cpu_bus: CpuBus::new(),
            bus_trace: BusTrace::new(),
        };
        gbc.rebuild_state();
        gbc
    }

    /// Power-cycle the console: re-create all volatile state while
    /// preserving the inserted cartridge (and its battery-backed SRAM),
    /// the boot ROM contents, and the user-attached serial link.
    pub fn reset(&mut self) {
        self.external.reset();
        self.rebuild_state();
    }

    fn rebuild_state(&mut self) {
        let has_boot_rom = self.external.has_boot_rom();
        let header_checksum = self.external.cartridge.header_checksum();

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
        self.joypad = Joypad::new();
        self.interrupts = interrupts::Registers::new();
        self.serial = Serial::new();
        self.timers = if has_boot_rom {
            Timers::new()
        } else {
            Timers::post_boot()
        };
        self.audio = if has_boot_rom {
            Audio::new()
        } else {
            Audio::post_boot(self.timers.internal_counter)
        };
        self.dma = Dma::new();
        self.vram_bus = VramBus::new();

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

    pub fn timers(&self) -> &Timers {
        &self.timers
    }

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.interrupts
    }

    pub fn dma(&self) -> &Dma {
        &self.dma
    }

    pub fn serial(&self) -> &Serial {
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

    pub fn set_link(&mut self, link: Box<dyn SerialLink>) {
        self.serial.set_link(link);
    }
}

#[cfg(feature = "test-support")]
impl missingno_gb::test_support::System for GameBoyColor {
    fn step(&mut self) -> missingno_gb::execute::StepResult {
        GameBoyColor::step(self)
    }
    fn read(&self, address: u16) -> u8 {
        GameBoyColor::read(self, address)
    }
    fn cpu(&self) -> &Cpu {
        GameBoyColor::cpu(self)
    }
    fn cpu_mut(&mut self) -> &mut Cpu {
        GameBoyColor::cpu_mut(self)
    }
    fn drain_serial_output(&mut self) -> Vec<u8> {
        GameBoyColor::drain_serial_output(self)
    }
    fn interrupts(&self) -> &interrupts::Registers {
        GameBoyColor::interrupts(self)
    }
    fn peek_range(&self, start: u16, len: u16) -> Vec<u8> {
        GameBoyColor::peek_range(self, start, len)
    }
    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        GameBoyColor::drain_audio_samples(self)
    }
}
