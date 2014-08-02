use rom_info::RomInfo;

pub struct Gameboy {
    info: RomInfo,
    rom: Vec<u8>
}

impl Gameboy {
    pub fn new(rom: Vec<u8>) -> Gameboy {
        let gb = Gameboy {
            info: RomInfo::new(rom.as_slice()),
            rom: rom
        };

        println!("{}", gb.info.title());

        gb
    }
}
