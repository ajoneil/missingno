use rom_info::RomInfo;
use cpu::Cpu;
use cartridge::Cartridge;

pub struct Gameboy {
    info: RomInfo,
    cartridge: Cartridge,
    cpu: Cpu
}

impl Gameboy {
    pub fn new(rom: Vec<u8>) -> Gameboy {
        let info = RomInfo::new(rom.as_slice());
        let cartridge = Cartridge::new(rom, info.mbc_type);

        let gb = Gameboy {
            info: info,
            cartridge: cartridge,
            cpu: Cpu::new()
        };

        println!("{}", gb.info.title);

        gb
    }
}
