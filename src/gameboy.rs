use rom_info::RomInfo;
use cpu::Cpu;

pub struct Gameboy {
    info: RomInfo,
    rom: Vec<u8>,
    cpu: Cpu
}

impl Gameboy {
    pub fn new(rom: Vec<u8>) -> Gameboy {
        let gb = Gameboy {
            info: RomInfo::new(rom.as_slice()),
            rom: rom,
            cpu: Cpu::new()
        };

        println!("{}", gb.info.title());

        gb
    }
}
