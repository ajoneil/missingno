use cartridge::Cartridge;

pub struct Mmu {
    cartridge: Cartridge,
    wram: [u8, .. 2000]
}

impl Mmu {
    pub fn new(cartridge: Cartridge) -> Mmu {
        Mmu {
            cartridge: cartridge,
            wram: [0, .. 2000]
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..0x7fff => self.cartridge.read(address),
            0xc000..0xdfff => self.wram[address as uint - 0xc000],
            0xe000..0xfdff => self.wram[address as uint - 0xe000],
            _ => fail!("Unimplemented read from {}", address)
        }
    }

    pub fn read_word(&self, address: u16) -> u16 {
        self.read(address) as u16 + (self.read(address + 1) as u16 * 256)
    }
}
