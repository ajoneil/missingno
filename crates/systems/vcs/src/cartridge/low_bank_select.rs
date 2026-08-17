//! The mechanism UA Ltd, Fotomania and EconoBanking share: two 4 KB banks, one
//! hotspot each, both sitting below the cart window.
//!
//! The port has no chip select, so a board of this shape simply watches the bus:
//! any access to a hotspot, read or write, selects its bank with the data value
//! irrelevant. Each board compares its own handful of address lines and treats
//! every other line as a don't-care, so a hotspot is a whole family of aliases
//! rather than one address. Only the decode differs between the boards.

/// The address lines a board compares, and the patterns its two hotspots
/// answer to.
pub struct Decode {
    pub lines: u16,
    pub bank_0: u16,
    pub bank_1: u16,
}

pub struct LowBankSelect {
    image: Vec<u8>,
    bank: usize,
    decode: Decode,
}

impl LowBankSelect {
    pub fn new(rom: &[u8], decode: Decode) -> LowBankSelect {
        LowBankSelect {
            image: rom.to_vec(),
            bank: 0,
            decode,
        }
    }

    fn hotspot(&mut self, address: u16) {
        let decoded = address & self.decode.lines;
        if decoded == self.decode.bank_0 {
            self.bank = 0;
        } else if decoded == self.decode.bank_1 {
            self.bank = 1;
        }
    }

    /// The board watches every cycle for its hotspots, but only drives the bus
    /// inside the window.
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
