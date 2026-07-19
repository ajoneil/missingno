//! The E7 board (M-Network): 16 KB as eight 2 KB banks, plus two separately
//! addressed cart RAMs, sharing the 4 KB window.
//!
//! The window splits three ways:
//!   $F000-$F7FF  lower window  — ROM bank N ($1FE0+N, N=0..6), or the 1 KB RAM
//!                                ($1FE7 — a RAM select, not an eighth bank, so
//!                                bank 7 never reaches this window)
//!   $F800-$F9FF  page window   — one of four 256-byte RAM pages ($1FE8-$1FEB)
//!   $FA00-$FFFF  fixed         — always the top 1.5 KB of bank 7
//!
//! Both RAMs split write-low / read-high, the cart edge having no R/W line: the
//! 1 KB RAM writes at $F000-$F3FF and reads at $F400-$F7FF, a 256-byte page
//! writes at $F800-$F8FF and reads at $F900-$F9FF.
//!
//! The page-select run is $1FE8-$1FEB: kevtris's sizes_v6 contradicts itself,
//! giving $1FF8-$1FFB in one place and $1FE8-$1FEB in another, and the latter is
//! what Stella, Gopher2600 and MAME all decode.

const BANK_SIZE: usize = 0x800;
const ROM_BANK_HOTSPOT: u16 = 0x1FE0;
/// $1FE7 selects the 1 KB RAM into the lower window instead of a ROM bank.
const RAM_SELECT: u16 = 0x1FE7;
const PAGE_HOTSPOT: u16 = 0x1FE8;
const PAGES: usize = 4;

const RAM_1K_SIZE: usize = 0x400;
const PAGE_SIZE: usize = 0x100;
/// The lower window ends, and the 256-byte page window begins, here.
const PAGE_WINDOW: usize = 0x800;
/// The fixed region begins here, and runs to the top of the window.
const FIXED_REGION: usize = 0xA00;
const FIXED_BANK: usize = 7;

/// What the lower window currently answers with.
enum LowerWindow {
    Rom(usize),
    Ram,
}

pub struct E7 {
    image: Vec<u8>,
    lower: LowerWindow,
    page: usize,
    ram_1k: Box<[u8; RAM_1K_SIZE]>,
    pages: Box<[u8; PAGES * PAGE_SIZE]>,
}

impl E7 {
    pub fn new(rom: &[u8]) -> E7 {
        E7 {
            image: rom.to_vec(),
            lower: LowerWindow::Rom(0),
            page: 0,
            ram_1k: Box::new([0; RAM_1K_SIZE]),
            pages: Box::new([0; PAGES * PAGE_SIZE]),
        }
    }

    fn hotspot(&mut self, address: u16) {
        let decoded = address & 0x1FFF;
        let page = decoded.wrapping_sub(PAGE_HOTSPOT) as usize;
        let bank = decoded.wrapping_sub(ROM_BANK_HOTSPOT) as usize;
        if decoded == RAM_SELECT {
            self.lower = LowerWindow::Ram;
        } else if page < PAGES {
            self.page = page;
        } else if bank < FIXED_BANK {
            self.lower = LowerWindow::Rom(bank);
        }
    }

    pub fn read(&mut self, address: u16, bus: u8) -> u8 {
        self.hotspot(address);
        // No R/W line: a write-port read still stores, latching the floating
        // bus byte the CPU also sees.
        if let Some(cell) = self.write_port(address) {
            *cell = bus;
            return bus;
        }
        self.peek(address)
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        self.hotspot(address);
        if let Some(cell) = self.write_port(address) {
            *cell = data;
        }
    }

    /// The RAM cell an address writes, if it lands on either RAM's write port.
    fn write_port(&mut self, address: u16) -> Option<&mut u8> {
        let offset = (address & 0x0FFF) as usize;
        match offset {
            _ if offset < RAM_1K_SIZE && matches!(self.lower, LowerWindow::Ram) => {
                Some(&mut self.ram_1k[offset])
            }
            PAGE_WINDOW..FIXED_REGION if offset - PAGE_WINDOW < PAGE_SIZE => {
                Some(&mut self.pages[self.page * PAGE_SIZE + offset - PAGE_WINDOW])
            }
            _ => None,
        }
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        match offset {
            FIXED_REGION.. => self.image[FIXED_BANK * BANK_SIZE + (offset & (BANK_SIZE - 1))],
            PAGE_WINDOW.. => {
                let cell = (offset - PAGE_WINDOW) % PAGE_SIZE;
                self.pages[self.page * PAGE_SIZE + cell]
            }
            _ => match self.lower {
                LowerWindow::Rom(bank) => self.image[bank * BANK_SIZE + offset],
                // Write port low, read port high, the same cells.
                LowerWindow::Ram => self.ram_1k[offset % RAM_1K_SIZE],
            },
        }
    }
}
