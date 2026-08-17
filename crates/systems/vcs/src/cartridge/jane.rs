//! The JANE board (the Tarzan prototype): four 4 KB banks on four scattered
//! hotspots.
//!
//! An F-series bankswitcher in every respect but its address map — where F6
//! runs its four selects consecutively, JANE puts two at $1FF0/$1FF1 and two at
//! $1FF8/$1FF9, leaving the addresses between them dead. The select fires on
//! the bus access, read or write, with the data irrelevant.

const BANK_SIZE: usize = 0x1000;

pub struct Jane {
    image: Vec<u8>,
    bank: usize,
}

impl Jane {
    pub fn new(rom: &[u8]) -> Jane {
        Jane {
            image: rom.to_vec(),
            bank: 0,
        }
    }

    fn hotspot(&mut self, address: u16) {
        self.bank = match address & 0x1FFF {
            0x1FF0 => 0,
            0x1FF1 => 1,
            0x1FF8 => 2,
            0x1FF9 => 3,
            _ => return,
        };
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

    pub(super) fn set_bank(&mut self, bank: usize) {
        let banks = self.image.len() / BANK_SIZE;
        if bank < banks {
            self.bank = bank;
        }
    }

    pub fn peek(&self, address: u16) -> u8 {
        super::banked_byte(&self.image, self.bank, address)
    }
}
