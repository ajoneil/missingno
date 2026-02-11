use audio::Audio;
use cartridge::Cartridge;
use cpu::{Cpu, cycles::Cycles};
use joypad::{Button, Joypad};
use memory::Ram;
use video::{Video, screen::Screen};

pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod execute;
pub mod interrupts;
pub mod joypad;
pub mod memory;
pub mod recording;
pub mod save_state;
pub mod serial_transfer;
pub mod sgb;
pub mod timers;
pub mod video;

// Anything accessible via a memory address is stored in a separate
// struct to allow borrowing independently of the Cpu
pub struct MemoryMapped {
    cartridge: Cartridge,
    ram: Ram,
    video: Video,
    audio: Audio,
    joypad: Joypad,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Registers,
    timers: timers::Timers,
    dma_transfer_cycles: Option<Cycles>,
    dma_source: u8,
    sgb: Option<sgb::Sgb>,
}

pub struct GameBoy {
    cpu: Cpu,
    screen: Screen,
    mapped: MemoryMapped,
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
            mapped: MemoryMapped {
                cartridge,
                ram: Ram::new(),
                video: Video::new(),
                audio: Audio::new(),
                joypad: Joypad::new(),
                interrupts: interrupts::Registers::new(),
                serial: serial_transfer::Registers::new(),
                timers: timers::Timers::new(),
                dma_transfer_cycles: None,
                dma_source: 0,
                sgb,
            },
        }
    }

    pub fn reset(&mut self) {
        self.cpu = Cpu::new(self.mapped.cartridge.header_checksum());
        self.screen = Screen::new();
        self.mapped.ram = Ram::new();
        self.mapped.video = Video::new();
        self.mapped.audio = Audio::new();
        self.mapped.joypad = Joypad::new();
        self.mapped.interrupts = interrupts::Registers::new();
        self.mapped.serial = serial_transfer::Registers::new();
        self.mapped.timers = timers::Timers::new();
        self.mapped.dma_transfer_cycles = None;
        self.mapped.dma_source = 0;
        self.mapped.sgb = if self.mapped.cartridge.supports_sgb() {
            Some(sgb::Sgb::new())
        } else {
            None
        };
    }

    pub fn memory_mapped(&self) -> &MemoryMapped {
        &self.mapped
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.mapped.cartridge
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn video(&self) -> &Video {
        &self.mapped.video
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
