use super::cartridge::Cartridge;
use super::cpu::Cpu;
use super::joypad::Joypad;
use super::mmu::Mmu;
use super::rom_info::RomInfo;
use super::timers::timers::Timers;
use super::video::Video;

pub struct GameBoy {
    info: RomInfo,
    cpu: Cpu,
    timers: Timers,
    mmu: Mmu,
    video: Video,
    joypad: Joypad,
}

impl GameBoy {
    pub fn new(rom: Vec<u8>) -> GameBoy {
        let info = RomInfo::new(rom.as_slice());
        let cartridge = Cartridge::new(rom, info.mbc_type);
        let video = Video::new();
        let mmu = Mmu::new(cartridge);

        let gb = GameBoy {
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
        // println!("{:?}", self.cpu);
    }

    pub fn rom_info(&self) -> &RomInfo {
        &self.info
    }
}
