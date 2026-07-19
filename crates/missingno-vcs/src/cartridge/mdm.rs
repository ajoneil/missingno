//! The MDM Menu Driven Megacart board (Edwin Blink, homebrew): a bank named by
//! the low byte of a low-memory access, and a one-way lock.
//!
//! The board watches the $0800-$0BFF band — any access there, read or write,
//! data irrelevant — and takes the bank from the address's low byte, folded into
//! the image's bank count. A10 bounds the band, so the page just above it is
//! inert; the page bits inside it are don't-cares.
//!
//! Selecting any value with bit 7 set performs that switch and then freezes the
//! board until a console reset. The switch still happens first, through the same
//! fold, so a menu can lock itself into the bank it means to run.

const BANK_SIZE: usize = 0x1000;
/// A12, A11, A10: the band's compare.
const DECODE_MASK: u16 = 0x1C00;
const BAND: u16 = 0x0800;
/// A selected value with this bit set locks the board.
const LOCK: u8 = 0x80;

pub struct Mdm {
    image: Vec<u8>,
    bank: usize,
    banks: usize,
    locked: bool,
}

impl Mdm {
    pub fn new(rom: &[u8]) -> Mdm {
        Mdm {
            image: rom.to_vec(),
            bank: 0,
            banks: rom.len() / BANK_SIZE,
            locked: false,
        }
    }

    fn hotspot(&mut self, address: u16) {
        if self.locked || address & DECODE_MASK != BAND {
            return;
        }
        let value = address as u8;
        self.bank = usize::from(value) % self.banks;
        self.locked = value & LOCK != 0;
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

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
