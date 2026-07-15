//! The SB "SuperBanking" board: thirty-two 4 KB banks selected from low memory.
//!
//! The window is plain ROM and never switches. The selects sit at $0800-$0FFF
//! instead: any access with A11 high and A12 low pages in the bank named by the
//! low address bits, read or write, with the data irrelevant. A8-A10 are
//! don't-cares in that compare, so every $100 page from $0800 up mirrors the
//! same select.
//!
//! Unlike the hotspot boards it powers on in the *last* bank, not the first.

const BANK_SIZE: usize = 0x1000;
/// A11 high, A12 low.
const DECODE_MASK: u16 = 0x1800;
const SELECT: u16 = 0x0800;

pub struct Sb {
    image: Vec<u8>,
    bank: usize,
    banks: usize,
}

impl Sb {
    pub fn new(rom: &[u8]) -> Sb {
        let banks = rom.len() / BANK_SIZE;
        Sb {
            image: rom.to_vec(),
            bank: banks - 1,
            banks,
        }
    }

    fn hotspot(&mut self, address: u16) {
        if address & DECODE_MASK == SELECT {
            self.bank = usize::from(address) & (self.banks - 1);
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
