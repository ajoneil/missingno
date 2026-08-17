//! The UA Ltd board: two 4 KB banks, selected from hotspots that live outside
//! the cartridge window and are decoded loosely enough to alias widely.
//!
//! $0220 selects bank 0 and $0240 bank 1 — down in the low address space, where
//! A12 is low and the cart drives nothing. The port has no chip select, so the
//! board simply watches the bus: any access to a hotspot, read or write, flips
//! the bank with the data value irrelevant.
//!
//! The decode examines only A12, A9, A6 and A5 and treats every other line as a
//! don't-care, so each hotspot is a whole family of aliases rather than one
//! address ($0320 and $02A0 both reduce to $0220).
//!
//! Because the hotspots sit at A12=0 they also land on TIA and RIOT mirrors: a
//! write to $0220 pokes HMP0 as well as paging the bank. The console still
//! routes them there — the board only listens.

const BANK_SIZE: usize = 0x1000;
/// The only address lines the board examines: A12, A9, A6, A5.
const DECODE_MASK: u16 = 0x1260;
const BANK_0_HOTSPOT: u16 = 0x0220;
const BANK_1_HOTSPOT: u16 = 0x0240;

pub struct UaLtd {
    image: Vec<u8>,
    bank: usize,
}

impl UaLtd {
    pub fn new(rom: &[u8]) -> UaLtd {
        UaLtd {
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

    pub fn peek(&self, address: u16) -> u8 {
        self.image[self.bank * BANK_SIZE + (address & 0x0FFF) as usize]
    }
}
