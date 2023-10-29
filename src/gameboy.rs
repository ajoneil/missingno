use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::joypad::Joypad;
use crate::mmu::Mmu;
use crate::rom_info::RomInfo;
use crate::timers::timers::Timers;
use crate::video::Video;

pub struct Gameboy {
    info: RomInfo,
    cpu: Cpu,
    timers: Timers,
    mmu: Mmu,
    video: Video,
    joypad: Joypad,
}

impl Gameboy {
    pub fn new(rom: Vec<u8>) -> Gameboy {
        let info = RomInfo::new(rom.as_slice());
        let cartridge = Cartridge::new(rom, info.mbc_type);
        let video = Video::new();
        let mmu = Mmu::new(cartridge);

        let gb = Gameboy {
            cpu: Cpu::new(info.checksum),
            timers: Timers::new(),
            info: info,
            mmu: mmu,
            video: video,
            joypad: Joypad::new(),
        };

        println!("{}", gb.info.title);

        gb
    }

    pub fn video(&self) -> &Video {
        &self.video
    }

    pub fn take_frame(&mut self) {
        self.video.take_frame()
    }

    pub fn step(&mut self) {
        let cycles = self.cpu.step(
            &mut self.mmu,
            &mut self.video,
            &mut self.timers,
            &mut self.joypad,
        );
        self.timers.step(cycles, &mut self.mmu);
        self.video.step(cycles, &mut self.mmu);
        //println!("{:?}", self.cpu);
    }

    pub fn rom_info(&self) -> &RomInfo {
        &self.info
    }
}
