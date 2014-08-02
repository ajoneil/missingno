pub struct Cartridge {
    rom: Vec<u8>
}

impl Cartridge {
    pub fn new(rom: Vec<u8>) -> Cartridge {
        Cartridge {
            rom: rom
        }
    }
}
