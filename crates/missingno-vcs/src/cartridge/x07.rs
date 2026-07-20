//! The X07 board (Payson/Quimby homebrew, Stella's "Stocking"): sixteen banks
//! and two independent switch mechanisms, the second of which fires on ordinary
//! TIA writes.
//!
//! The board watches the whole low space for two patterns. The direct select
//! needs A12 low, A11 high and a low nibble of $D, and takes the bank from
//! address bits 4-7 — which are outside the compare, so each bank has its own
//! base hotspot, and A8-A10 alias them.
//!
//! The other fires when A12, A11 and A7 are all low, which is true of virtually
//! every TIA register access — but only while the board is already in one of the
//! top two banks, and then it flips between them by address bit 6. The register
//! is still written; the flip is a side effect. So the same store through two
//! TIA mirrors parks two different banks.

const BANK_SIZE: usize = 0x1000;

/// A12, A11, A3-A0: the direct select's compare.
const DIRECT_MASK: u16 = 0x180F;
const DIRECT_SELECT: u16 = 0x080D;

/// A12, A11, A7: the shadow switch's compare.
const SHADOW_MASK: u16 = 0x1880;
/// The shadow switch only answers from the top two banks, and only ever moves
/// between them.
const TOP_PAIR: usize = 0x0E;

pub struct X07 {
    image: Vec<u8>,
    bank: usize,
}

impl X07 {
    pub fn new(rom: &[u8]) -> X07 {
        X07 {
            image: rom.to_vec(),
            bank: 0,
        }
    }

    fn hotspot(&mut self, address: u16) {
        if address & DIRECT_MASK == DIRECT_SELECT {
            self.bank = usize::from(address & 0xF0) >> 4;
        } else if address & SHADOW_MASK == 0 && self.bank & TOP_PAIR == TOP_PAIR {
            self.bank = usize::from(address & 0x40) >> 6 | TOP_PAIR;
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

    /// The 4 KB bank currently paged into the window, for the debugger.
    pub(super) fn selected_bank(&self) -> usize {
        self.bank
    }

    pub(super) fn set_bank(&mut self, bank: usize) {
        self.bank = bank & 0x0F;
    }

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
