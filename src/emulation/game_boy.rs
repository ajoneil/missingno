use super::cartridge::Cartridge;

pub struct GameBoy {
    cartridge: Cartridge,
    // cpu: Cpu,
    // timers: Timers,
    // mmu: Mmu,
    // video: Video,
    // joypad: Joypad,
}

impl GameBoy {
    pub fn new(cartridge: Cartridge) -> GameBoy {
        // let video = Video::new();
        // let mmu = Mmu::new(cartridge);

        GameBoy {
            cartridge,
            // cpu: Cpu::new(info.checksum),
            // timers: Timers::new(),
            // mmu: mmu,
            // video: video,
            // joypad: Joypad::new(),
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        &self.cartridge
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
