//! The E0 board (Parker Bros): 8 KB as eight 1 KB slices, feeding three
//! independently-paged windows plus a fixed one.
//!
//! The 4 KB window carves into four 1 KB sub-windows. The first three each draw
//! any slice from the pool, chosen by their own hotspot run; the fourth is
//! always slice 7, which is where the hotspots live — so code that strobes a
//! select survives its own next opcode fetch.
//!
//! As on the Atari boards a select fires on the bus access, read or write, with
//! the data bus irrelevant. The three windows are orthogonal: paging one leaves
//! the others standing.

const SLICE_SIZE: usize = 0x400;
const SLICES: u16 = 8;
/// One hotspot run per pageable window: $1FE0+N, $1FE8+N, $1FF0+N select slice
/// N into window 0, 1, 2.
const HOTSPOT_BASE: u16 = 0x1FE0;
const PAGEABLE_WINDOWS: usize = 3;
/// The fourth window never moves.
const FIXED_SLICE: usize = 7;

pub struct E0 {
    image: Vec<u8>,
    slices: [usize; PAGEABLE_WINDOWS],
}

impl E0 {
    pub fn new(rom: &[u8]) -> E0 {
        E0 {
            image: rom.to_vec(),
            slices: [0; PAGEABLE_WINDOWS],
        }
    }

    fn hotspot(&mut self, address: u16) {
        let offset = (address & 0x1FFF).wrapping_sub(HOTSPOT_BASE);
        let (window, slice) = (offset / SLICES, offset % SLICES);
        if (window as usize) < PAGEABLE_WINDOWS {
            self.slices[window as usize] = slice as usize;
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

    pub fn peek(&self, address: u16) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        let window = offset / SLICE_SIZE;
        let slice = self.slices.get(window).copied().unwrap_or(FIXED_SLICE);
        self.image[slice * SLICE_SIZE + offset % SLICE_SIZE]
    }
}
