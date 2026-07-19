//! The Dynacom Megaboy board (F0): sixteen 4 KB banks and no way to ask for one.
//!
//! There is a single hotspot, $1FF0, and any access to it — read or write, the
//! data irrelevant — merely advances to the next bank, wrapping past the last.
//! No address selects a specific bank, so software steps through them in order,
//! and code that strobes must be byte-identical in every bank.

const BANK_SIZE: usize = 0x1000;
const BANKS: usize = 16;
const HOTSPOT: u16 = 0x1FF0;

pub struct F0 {
    image: Vec<u8>,
    bank: usize,
}

impl F0 {
    pub fn new(rom: &[u8]) -> F0 {
        F0 {
            image: rom.to_vec(),
            bank: 0,
        }
    }

    fn hotspot(&mut self, address: u16) {
        if address & 0x1FFF == HOTSPOT {
            self.bank = (self.bank + 1) % BANKS;
        }
    }

    pub fn read(&mut self, address: u16) -> u8 {
        self.hotspot(address);
        self.peek(address)
    }

    pub fn write_access(&mut self, address: u16) {
        self.hotspot(address);
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    /// The 4 KB bank currently paged into the window, for the debugger.
    pub(super) fn selected_bank(&self) -> usize {
        self.bank
    }

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
