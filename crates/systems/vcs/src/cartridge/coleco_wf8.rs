//! The Coleco white-label WF8 board: F8's two banks, chosen by the data bus.
//!
//! Plain F8 gives each bank its own hotspot and ignores the data. WF8 has a
//! single hotspot, $1FF8, and takes the bank from one bit of the value written
//! there; $1FF9 does nothing at all.
//!
//! The bit is D2. Stella's header comment says D3, but its own code reads D2 and
//! so does Gopher2600 — the prose is a documentation bug, not a board. What a
//! *read* of $1FF8 does is undefined by the board, and the two implementations
//! disagree, so nothing is done here.

const HOTSPOT: u16 = 0x1FF8;
const BANK_SELECT: u8 = 0x04;

pub struct ColecoWf8 {
    image: Vec<u8>,
    bank: usize,
}

impl ColecoWf8 {
    pub fn new(rom: &[u8]) -> ColecoWf8 {
        ColecoWf8 {
            image: rom.to_vec(),
            bank: 0,
        }
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        if address & 0x1FFF == HOTSPOT {
            self.bank = usize::from(data & BANK_SELECT != 0);
        }
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
        self.bank = bank & 1;
    }

    pub fn peek(&self, address: u16) -> u8 {
        super::banked_byte(&self.image, self.bank, address)
    }
}
