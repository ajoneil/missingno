//! The 03E0 board (the Brazilian Parker Bros): E0's three pageable 1 KB
//! segments, selected active-low from hotspots down in low memory.
//!
//! The window carves into four 1 KB segments; the first three each draw any
//! slice from the pool and the fourth is always the last slice, which is where
//! the code lives. The selects sit at $0380-$03FF rather than in the window, and
//! each has its own active-low enable line, so one access can page all three
//! segments at once — or, with every enable high, none of them. The slice is the
//! low three address bits, and a select fires on the bus access, read or write.

const PAGEABLE_SEGMENTS: usize = 3;

/// A9-A7 high with A12 low: the hotspot page.
const HOTSPOT_DECODE: u16 = 0x1F80;
const HOTSPOT_SELECT: u16 = 0x0380;
/// Each segment's enable line, active low.
const ENABLES: [u16; PAGEABLE_SEGMENTS] = [0x10, 0x20, 0x40];

pub struct ParkerBrosBrazil {
    image: Vec<u8>,
    slices: [usize; PAGEABLE_SEGMENTS],
}

impl ParkerBrosBrazil {
    pub fn new(rom: &[u8]) -> ParkerBrosBrazil {
        ParkerBrosBrazil {
            image: rom.to_vec(),
            slices: [0; PAGEABLE_SEGMENTS],
        }
    }

    fn hotspot(&mut self, address: u16) {
        if address & HOTSPOT_DECODE != HOTSPOT_SELECT {
            return;
        }
        for (segment, enable) in ENABLES.iter().enumerate() {
            if address & enable == 0 {
                self.slices[segment] = usize::from(address & 0x07);
            }
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

    /// The window maps exactly as E0's does.
    pub fn peek(&self, address: u16) -> u8 {
        super::parker_bros::sliced_byte(&self.image, &self.slices, address)
    }
}
