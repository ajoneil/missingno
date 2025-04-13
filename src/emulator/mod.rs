pub mod audio;
pub mod cartridge;
pub mod cpu;
pub mod execute;
pub mod interrupts;
pub mod memory;
pub mod serial_transfer;
pub mod video;
// mod joypad;
// mod mbc;
// mod timers;

use audio::Audio;
use cartridge::Cartridge;
use cpu::Cpu;
use memory::Ram;
use video::Video;

// Anything accessible via a memory address is stored in a separate
// struct to allow borrowing independently of the Cpu
pub struct MemoryMapped {
    cartridge: Cartridge,
    ram: Ram,
    video: Video,
    audio: Audio,
    interrupts: interrupts::Registers,
    serial: serial_transfer::Registers,
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
                interrupts: interrupts::Registers::new(),
                serial: serial_transfer::Registers::new(),
            },
        }
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
