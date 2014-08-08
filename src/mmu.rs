use cartridge::Cartridge;

pub struct Mmu {
    cartridge: Cartridge,
    wram: [u8, .. 0x2000],
    hram: [u8, .. 0x80]
}

impl Mmu {
    pub fn new(cartridge: Cartridge) -> Mmu {
        Mmu {
            cartridge: cartridge,
            wram: [0, .. 0x2000],
            hram: [0, .. 0x80]
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..0x7fff => self.cartridge.read(address),
            0xc000..0xdfff => self.wram[address as uint - 0xc000],
            0xe000..0xfdff => self.wram[address as uint - 0xe000],
            0xff80..0xffff => self.hram[address as uint - 0xff80],
            _ => fail!("Unimplemented read from {:x}", address)
        }
    }

    pub fn read_word(&self, address: u16) -> u16 {
        self.read(address) as u16 + (self.read(address + 1) as u16 * 0x100)
    }

    pub fn write(&mut self, address: u16, val: u8) {
        match address {
            0xc000..0xdfff => self.wram[address as uint - 0xc000] = val,
            0xe000..0xfdff => self.wram[address as uint - 0xe000] = val,
            0xff80..0xffff => self.hram[address as uint - 0xff80] = val,
            _ => fail!("Unimplemented write to {:x}", address)
        }
    }

    pub fn write_word(&mut self, address: u16, val: u16) {
        self.write(address, (val % 0x100) as u8);
        self.write(address + 1, (val / 0x100) as u8);
    }
}
