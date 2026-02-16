use audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use dma::Dma;
use joypad::{Button, Joypad};
use memory::{ExternalBus, HighRam, VramBus};
use ppu::{Ppu, screen::Screen};

pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod dma;
pub mod execute;
pub mod interrupts;
pub mod joypad;
pub mod memory;
pub mod ppu;
pub mod recording;
pub mod serial_transfer;
pub mod sgb;
pub mod timers;

pub struct GameBoy {
    cpu: Cpu,
    screen: Screen,
    /// Counts T-cycles 0–3 within each M-cycle, used to call
    /// audio/serial/DMA once per M-cycle (every 4th T-cycle).
    mcycle_counter: u8,

    external: ExternalBus,
    high_ram: HighRam,
    ppu: Ppu,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Registers,
    timers: timers::Timers,
    dma: Dma,
    sgb: Option<sgb::Sgb>,
    vram_bus: VramBus,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> GameBoy {
        let cpu = Cpu::new(cartridge.header_checksum());
        let sgb = if cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };

        GameBoy {
            cpu,
            screen: Screen::new(),
            mcycle_counter: 0,
            external: ExternalBus::new(cartridge),
            high_ram: HighRam::new(),
            ppu: Ppu::new(),
            audio: Audio::new(),
            joypad: Joypad::new(),
            interrupts: interrupts::Registers::new(),
            serial: serial_transfer::Registers::new(),
            timers: timers::Timers::new(),
            dma: Dma::new(),
            sgb,
            vram_bus: VramBus::new(),
        }
    }

    pub fn reset(&mut self) {
        self.cpu = Cpu::new(self.external.cartridge.header_checksum());
        self.screen = Screen::new();
        self.mcycle_counter = 0;
        self.external.work_ram = [0; 0x2000];
        self.external.latch = 0xFF;
        self.external.decay = 0;
        self.high_ram = HighRam::new();
        self.ppu = Ppu::new();
        self.audio = Audio::new();
        self.joypad = Joypad::new();
        self.interrupts = interrupts::Registers::new();
        self.serial = serial_transfer::Registers::new();
        self.timers = timers::Timers::new();
        self.dma = Dma::new();
        self.vram_bus = VramBus::new();
        self.sgb = if self.external.cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.external.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    pub fn vram(&self) -> &ppu::memory::Vram {
        &self.vram_bus.vram
    }

    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.audio.drain_samples()
    }

    pub fn press_button(&mut self, button: Button) {
        self.joypad.press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.joypad.release_button(button);
    }

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.interrupts
    }

    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.sgb.as_ref()
    }

    #[allow(dead_code)]
    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.serial.output)
    }
}
