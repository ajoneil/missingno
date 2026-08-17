//! The Tigervision 3F board: the lower half of the window pages from the DATA
//! BUS, clocked by an address edge rather than selected by a hotspot.
//!
//! The upper 2 KB is permanently the last 2 KB of the image — the program, the
//! vectors and all switching code live there, so a page can never pull the
//! ground out from under the CPU. The lower 2 KB is a window onto any 2 KB bank.
//!
//! The board carries a single '173 latch, and the port has no R/W line, so the
//! latch physically cannot tell a store from a load. What it watches is
//! addresses and edges: an access with A6 and A7 low arms it, and if A12 rises
//! on the very next cycle the latch clocks, capturing whatever the data bus
//! still carries at that instant. The bus is capacitive, so at the rise it
//! still holds the previous cycle's byte — after `sta $3F` that residue is the
//! stored value, which is the whole select mechanism.
//!
//! A read below $40 pages too: nothing drives an unimplemented TIA address, so
//! the residue is the stale open-bus byte. This is why Tigervision code speaks
//! to the TIA only through its $40-$7F mirrors, reads included — A6 high never
//! arms the latch. alex_79 measured the rule on real Tigervision hardware
//! (AtariAge 329888); the write-only convention some emulators model describes
//! the habit of real games, not the trigger.

const BANK_SIZE: usize = 0x800;
/// A12, A7 and A6 all low: the access arms the latch, and leaves A12 able to
/// rise on the next cycle.
const ARM_MASK: u16 = 0x10C0;

pub struct Tigervision {
    image: Vec<u8>,
    bank: usize,
    banks: usize,
    /// The latch is armed and will clock on the next A12 rise.
    armed: bool,
}

impl Tigervision {
    pub fn new(rom: &[u8]) -> Tigervision {
        Tigervision {
            image: rom.to_vec(),
            bank: 0,
            banks: rom.len() / BANK_SIZE,
            armed: false,
        }
    }

    /// Every cycle at the cart edge. `residue` is the byte the bus still
    /// carries entering this cycle — what the latch samples at an A12 rise.
    fn cycle(&mut self, address: u16, residue: u8) {
        if super::selects_window(address) && self.armed {
            self.bank = residue as usize % self.banks;
        }
        self.armed = address & ARM_MASK == 0;
    }

    pub fn read(&mut self, address: u16, residue: u8) -> Option<u8> {
        self.cycle(address, residue);
        super::selects_window(address).then(|| self.peek(address))
    }

    pub fn write_access(&mut self, address: u16, residue: u8) {
        self.cycle(address, residue);
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        // The upper half never moves; the lower half is the selected bank.
        let bank = match offset < BANK_SIZE {
            true => self.bank,
            false => self.banks - 1,
        };
        self.image[bank * BANK_SIZE + offset % BANK_SIZE]
    }
}
