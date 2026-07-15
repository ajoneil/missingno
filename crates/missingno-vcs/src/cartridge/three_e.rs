//! The 3E board: Tigervision's 3F with a RAM path bolted on.
//!
//! A homebrew extension of the commercial 3F, credited to B. Watson and Thomas
//! Jentzsch. The upper 2 KB is permanently the last of the image, as on 3F, and
//! the lower 2 KB pages — but what pages in depends on which hotspot spoke:
//! writing $3F selects a 2 KB ROM bank, writing $3E a 1 KB RAM bank, and the
//! written value is the bank number either way. RAM paged out keeps its
//! contents; it is real static RAM on the cart.
//!
//! The RAM splits read-low, the CommaVid arrangement: it reads at $F000-$F3FF
//! and is written at $F400-$F7FF.
//!
//! The port has no R/W line, so as on 3F the latch clocks the bus residue at an
//! A12 rise after the hotspot access rather than decoding a write.

const ROM_BANK_SIZE: usize = 0x800;
const RAM_BANK_SIZE: usize = 0x400;
/// Stella's ceiling for the scheme, and more than any image uses.
const RAM_BANKS: usize = 32;

const ROM_HOTSPOT: u16 = 0x3F;
const RAM_HOTSPOT: u16 = 0x3E;
/// A12, A7 and A6 low: the access arms the latch and leaves A12 free to rise.
const ARM_MASK: u16 = 0x10C0;

/// What the lower window currently shows.
enum LowerWindow {
    Rom(usize),
    Ram(usize),
}

pub struct ThreeE {
    image: Vec<u8>,
    ram: Vec<u8>,
    lower: LowerWindow,
    rom_banks: usize,
    /// The hotspot that armed the latch, waiting for A12 to rise.
    armed: Option<u16>,
}

impl ThreeE {
    pub fn new(rom: &[u8]) -> ThreeE {
        ThreeE {
            image: rom.to_vec(),
            ram: vec![0; RAM_BANKS * RAM_BANK_SIZE],
            lower: LowerWindow::Rom(0),
            rom_banks: rom.len() / ROM_BANK_SIZE,
            armed: None,
        }
    }

    /// `residue` is the byte the bus still carries entering the cycle — what
    /// the latch samples at the rise.
    fn cycle(&mut self, address: u16, residue: u8) {
        if super::selects_window(address) {
            match self.armed {
                Some(ROM_HOTSPOT) => {
                    self.lower = LowerWindow::Rom(usize::from(residue) % self.rom_banks);
                }
                Some(RAM_HOTSPOT) => {
                    self.lower = LowerWindow::Ram(usize::from(residue) % RAM_BANKS);
                }
                _ => {}
            }
        }
        self.armed = match address & ARM_MASK == 0 {
            true => Some(address & 0x3F),
            false => None,
        };
    }

    /// The RAM cell an address writes, if the RAM is paged in and the address
    /// lands on its write port.
    fn write_port(&self, address: u16) -> Option<usize> {
        let offset = usize::from(address & 0x0FFF);
        match self.lower {
            LowerWindow::Ram(bank) if (RAM_BANK_SIZE..2 * RAM_BANK_SIZE).contains(&offset) => {
                Some(bank * RAM_BANK_SIZE + offset - RAM_BANK_SIZE)
            }
            _ => None,
        }
    }

    pub fn read(&mut self, address: u16, residue: u8) -> Option<u8> {
        self.cycle(address, residue);
        if !super::selects_window(address) {
            return None;
        }
        // No R/W line: a write-port read still stores, latching the floating
        // bus byte the CPU also sees.
        if let Some(cell) = self.write_port(address) {
            self.ram[cell] = residue;
            return Some(residue);
        }
        Some(self.peek(address))
    }

    pub fn write_access(&mut self, address: u16, residue: u8, data: u8) {
        self.cycle(address, residue);
        if let Some(cell) = self.write_port(address) {
            self.ram[cell] = data;
        }
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = usize::from(address & 0x0FFF);
        if offset >= ROM_BANK_SIZE {
            // The upper half never moves.
            return self.image[(self.rom_banks - 1) * ROM_BANK_SIZE + offset - ROM_BANK_SIZE];
        }
        match self.lower {
            LowerWindow::Rom(bank) => self.image[bank * ROM_BANK_SIZE + offset],
            LowerWindow::Ram(bank) => self.ram[bank * RAM_BANK_SIZE + offset % RAM_BANK_SIZE],
        }
    }
}
