use audio::Audio;
use cartridge::Cartridge;
use cpu::{Cpu, cycles::Cycles};
use joypad::Joypad;
use memory::Ram;
use video::Video;

pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod execute;
pub mod interrupts;
pub mod joypad;
pub mod memory;
pub mod serial_transfer;
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
}

pub struct GameBoy {
    cpu: Cpu,
    mapped: MemoryMapped,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> GameBoy {
        let cpu = Cpu::new(cartridge.header_checksum());

        GameBoy {
            cpu,
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
            },
        }
    }

    pub fn reset(&mut self) {
        self.cpu = Cpu::new(self.mapped.cartridge.header_checksum());
        self.mapped.ram = Ram::new();
        self.mapped.video = Video::new();
        self.mapped.audio = Audio::new();
        self.mapped.joypad = Joypad::new();
        self.mapped.interrupts = interrupts::Registers::new();
        self.mapped.serial = serial_transfer::Registers::new();
        self.mapped.timers = timers::Timers::new();
        self.mapped.dma_transfer_cycles = None;
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

    pub fn interrupts(&self) -> &interrupts::Registers {
        &self.mapped.interrupts
    }
}
