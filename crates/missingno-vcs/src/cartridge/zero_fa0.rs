//! The 0FA0 board (the Brazilian Fotomania): two banks selected from low
//! memory, through a decode loose enough to alias each hotspot into a family.
//!
//! Like UA, the selects sit below the window and the board just watches the bus:
//! any access, read or write, flips the bank with the data irrelevant. It
//! examines six address lines and treats the rest as don't-cares, so $06A0,
//! $07A0, $0EA0 and $0FA0 all select bank 0 — the last of them naming the board.
//! An address on those pages that misses the pattern selects nothing.

const BANK_SIZE: usize = 0x1000;
/// A12, A10, A9, A7, A6, A5 — the only lines the board examines.
const DECODE_MASK: u16 = 0x16E0;
const BANK_0_HOTSPOT: u16 = 0x06A0;
const BANK_1_HOTSPOT: u16 = 0x06C0;

pub struct ZeroFa0 {
    image: Vec<u8>,
    bank: usize,
}

impl ZeroFa0 {
    pub fn new(rom: &[u8]) -> ZeroFa0 {
        ZeroFa0 {
            image: rom.to_vec(),
            bank: 0,
        }
    }

    fn hotspot(&mut self, address: u16) {
        match address & DECODE_MASK {
            BANK_0_HOTSPOT => self.bank = 0,
            BANK_1_HOTSPOT => self.bank = 1,
            _ => {}
        }
    }

    pub fn read(&mut self, address: u16) -> Option<u8> {
        self.hotspot(address);
        super::selects_window(address).then(|| self.peek(address))
    }

    pub fn write_access(&mut self, address: u16) {
        self.hotspot(address);
    }

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
