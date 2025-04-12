mod cartridge;
pub mod cpu;
mod execute;
mod interrupts;
mod memory;
mod serial_transfer;
mod video;
// mod joypad;
// mod mbc;
// mod timers;

pub use cartridge::Cartridge;
pub use cpu::{Cpu, Flags as CpuFlags, Instruction};
use memory::Ram;
use video::Video;

// Anything accessible via a memory address is stored in a separate
// struct to allow borrowing independently of the Cpu
pub struct MemoryMapped {
    cartridge: Cartridge,
    ram: Ram,
    video: Video,
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
}
