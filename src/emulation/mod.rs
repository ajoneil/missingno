mod cartridge;
mod cpu;
mod memory;
// mod joypad;
// mod mbc;
// mod ops;
// mod timers;
// mod video;

pub use cartridge::Cartridge;
pub use cpu::{Cpu, Flags as CpuFlags, Instruction};
pub use memory::MemoryBus;

pub struct GameBoy {
    cpu: Cpu,
    memory_bus: MemoryBus,
    // timers: Timers,
    // video: Video,
    // joypad: Joypad,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> GameBoy {
        let cpu = Cpu::new(cartridge.header_checksum());
        let memory_bus = MemoryBus::new(cartridge);
        // let video = Video::new();

        GameBoy {
            cpu,
            memory_bus,
            // timers: Timers::new(),
            // video: video,
            // joypad: Joypad::new(),
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.memory_bus.cartridge()
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn step(&mut self) {
        self.cpu.step(&mut self.memory_bus);
    }

    // pub fn video(&self) -> &Video {
    //     &self.video
    // }

    // pub fn take_frame(&mut self) {
    //     self.video.take_frame()
    // }

    // pub fn step(&mut self) {
    //     let cycles = self.cpu.step(
    //         &mut self.mmu,
    //         &mut self.video,
    //         &mut self.timers,
    //         &mut self.joypad,
    //     );
    //     self.timers.step(cycles, &mut self.mmu);
    //     self.video.step(cycles, &mut self.mmu);
    //     // println!("{:?}", self.cpu);
    // }

    // pub fn rom_info(&self) -> &RomInfo {
    //     &self.info
    // }
}
