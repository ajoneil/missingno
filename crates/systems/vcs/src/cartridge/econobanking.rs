//! The 0840 EconoBanking board (Fred Quimby, homebrew): two banks selected from
//! low memory, on a decode that collapses onto a single address line.
//!
//! The board watches the $0800-$0FFF band and any access there — read or write,
//! data irrelevant — selects. It compares three lines, A12, A11 and A6, but
//! inside the band the first two are already fixed, so the choice reduces to A6
//! alone: there is no inert address in the band at all, and every other line is
//! a don't-care. The only near miss is the band's own twin up in the window,
//! where A12 is high and the board is not listening.

const BANK_SIZE: usize = 0x1000;
/// A12, A11, A6 — the only lines the board compares.
const DECODE_MASK: u16 = 0x1840;
const BANK_0_HOTSPOT: u16 = 0x0800;
const BANK_1_HOTSPOT: u16 = 0x0840;

pub struct Econobanking {
    image: Vec<u8>,
    bank: usize,
}

impl Econobanking {
    pub fn new(rom: &[u8]) -> Econobanking {
        Econobanking {
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

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
