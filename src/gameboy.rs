use rom_info::RomInfo;
use cpu::Cpu;
use cartridge::Cartridge;
use mmu::Mmu;

pub struct Gameboy {
    info: RomInfo,
    cpu: Cpu,
    mmu: Mmu
}

impl Gameboy {
    pub fn new(rom: Vec<u8>) -> Gameboy {
        let info = RomInfo::new(rom.as_slice());
        let cartridge = Cartridge::new(rom, info.mbc_type);
        let mmu = Mmu::new(cartridge);

        let gb = Gameboy {
            info: info,
            cpu: Cpu::new(),
            mmu: mmu
        };

        println!("{}", gb.info.title);

        gb
    }
}
