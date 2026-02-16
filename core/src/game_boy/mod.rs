use audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use dma::Dma;
use joypad::{Button, Joypad};
use memory::{ExternalBus, VramBus};
use video::{Video, screen::Screen};

pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod dma;
pub mod execute;
pub mod interrupts;
pub mod joypad;
pub mod memory;
pub mod recording;
pub mod serial_transfer;
pub mod sgb;
pub mod timers;
pub mod video;

// Anything accessible via a memory address is stored in a separate
// struct to allow borrowing independently of the Cpu
pub struct MemoryMapped {
    external: ExternalBus,
    high_ram: [u8; 0x80],
    video: Video,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Registers,
    timers: timers::Timers,
    dma: Dma,
    sgb: Option<sgb::Sgb>,

    vram_bus: VramBus,
}

pub struct GameBoy {
    cpu: Cpu,
    screen: Screen,
    mapped: MemoryMapped,
    /// Counts T-cycles 0–3 within each M-cycle, used to call
    /// audio/serial/DMA once per M-cycle (every 4th T-cycle).
    mcycle_counter: u8,
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
            mapped: MemoryMapped {
                external: ExternalBus::new(cartridge),
                high_ram: [0; 0x80],
                video: Video::new(),
                audio: Audio::new(),
                joypad: Joypad::new(),
                interrupts: interrupts::Registers::new(),
                serial: serial_transfer::Registers::new(),
                timers: timers::Timers::new(),
                dma: Dma::new(),
                sgb,
                vram_bus: VramBus::new(),
            },
        }
    }

    pub fn reset(&mut self) {
        self.cpu = Cpu::new(self.mapped.external.cartridge.header_checksum());
        self.screen = Screen::new();
        self.mcycle_counter = 0;
        self.mapped.external.work_ram = [0; 0x2000];
        self.mapped.external.latch = 0xFF;
        self.mapped.external.decay = 0;
        self.mapped.high_ram = [0; 0x80];
        self.mapped.video = Video::new();
        self.mapped.audio = Audio::new();
        self.mapped.joypad = Joypad::new();
        self.mapped.interrupts = interrupts::Registers::new();
        self.mapped.serial = serial_transfer::Registers::new();
        self.mapped.timers = timers::Timers::new();
        self.mapped.dma = Dma::new();
        self.mapped.vram_bus = VramBus::new();
        self.mapped.sgb = if self.mapped.external.cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };
    }

    pub fn memory_mapped(&self) -> &MemoryMapped {
        &self.mapped
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.mapped.external.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn video(&self) -> &Video {
        &self.mapped.video
    }

    pub fn vram(&self) -> &video::memory::Vram {
        &self.mapped.vram_bus.vram
    }

    pub fn audio(&self) -> &Audio {
        &self.mapped.audio
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.mapped.audio.drain_samples()
    }

    pub fn press_button(&mut self, button: Button) {
        self.mapped.joypad.press_button(button);
    }

    pub fn release_button(&mut self, button: Button) {
        self.mapped.joypad.release_button(button);
    }

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.mapped.interrupts
    }

    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.mapped.sgb.as_ref()
    }

    #[allow(dead_code)]
    pub fn drain_serial_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.mapped.serial.output)
    }
}
