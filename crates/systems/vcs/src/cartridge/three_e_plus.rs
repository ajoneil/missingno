//! The 3E+ board (Thomas Jentzsch): 3E rebuilt around four independently banked
//! 1 KB segments, so a program can mix ROM and RAM anywhere in the window.
//!
//! One hotspot write both picks a segment and banks it, because the value
//! carries both: the top two bits are the segment, the low six the bank. $3F
//! banks a 1 KB ROM bank into it, $3E a 512-byte RAM bank. No segment is fixed,
//! which is what lets the image be any multiple of 1 KB without the board
//! knowing its size.
//!
//! The RAM reads low within each segment: a bank reads at the segment base and
//! is written 512 bytes higher.
//!
//! Only segment 3 has a defined power-on bank — ROM bank 0, so the reset vector
//! at the top of the window can boot the machine. The rest are undefined.

const SEGMENTS: usize = 4;
const SEGMENT_SIZE: usize = 0x400;
const ROM_BANK_SIZE: usize = 0x400;
const RAM_BANK_SIZE: usize = 0x200;
/// The bank field reaches 64 RAM banks as well, and every one is real memory.
const RAM_BANKS: usize = 64;

/// The hotspot value's bank field is six bits, so the scheme reaches 64 banks.
const MAX_ROM_BANKS: usize = 64;

/// Whether an image is a whole number of ROM banks within the bank field's
/// reach. As on 3E, the board has no one size: the image says how many banks
/// the cart carries.
pub fn holds(len: usize) -> bool {
    len.is_multiple_of(ROM_BANK_SIZE) && (1..=MAX_ROM_BANKS).contains(&(len / ROM_BANK_SIZE))
}

const ROM_HOTSPOT: u16 = 0x3F;
const RAM_HOTSPOT: u16 = 0x3E;
/// A12, A7 and A6 low: the access arms the latch and leaves A12 free to rise.
const ARM_MASK: u16 = 0x10C0;

/// The segment holding the reset vector, and so the only one that boots defined.
const BOOT_SEGMENT: usize = 3;

#[derive(Clone, Copy)]
enum Mapping {
    Rom(usize),
    Ram(usize),
}

pub struct ThreeEPlus {
    image: Vec<u8>,
    ram: Vec<u8>,
    segments: [Mapping; SEGMENTS],
    rom_banks: usize,
    armed: Option<u16>,
}

impl ThreeEPlus {
    pub fn new(rom: &[u8]) -> ThreeEPlus {
        let mut segments = [Mapping::Rom(0); SEGMENTS];
        segments[BOOT_SEGMENT] = Mapping::Rom(0);
        ThreeEPlus {
            image: rom.to_vec(),
            ram: vec![0; RAM_BANKS * RAM_BANK_SIZE],
            segments,
            rom_banks: rom.len() / ROM_BANK_SIZE,
            armed: None,
        }
    }

    fn cycle(&mut self, address: u16, residue: u8) {
        if super::selects_window(address) {
            // The value picks the segment in its top two bits and the bank in
            // the rest.
            let segment = usize::from(residue >> 6);
            let bank = usize::from(residue & 0x3F);
            match self.armed {
                Some(ROM_HOTSPOT) => self.segments[segment] = Mapping::Rom(bank % self.rom_banks),
                Some(RAM_HOTSPOT) => self.segments[segment] = Mapping::Ram(bank % RAM_BANKS),
                _ => {}
            }
        }
        self.armed = match address & ARM_MASK == 0 {
            true => Some(address & 0x3F),
            false => None,
        };
    }

    /// The RAM cell an address writes, if its segment holds RAM and the address
    /// lands on the upper half of it — the write port.
    fn write_port(&self, address: u16) -> Option<usize> {
        let offset = usize::from(address & 0x0FFF);
        let within = offset % SEGMENT_SIZE;
        match self.segments[offset / SEGMENT_SIZE] {
            Mapping::Ram(bank) if within >= RAM_BANK_SIZE => {
                Some(bank * RAM_BANK_SIZE + within - RAM_BANK_SIZE)
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

    /// The cart RAM segments as one linear space.
    pub(super) fn ram(&self) -> &[u8] {
        &self.ram
    }

    pub(super) fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    /// The four segment mappings (ROM/RAM bank each), for a state save, as
    /// `[tag, bank]` pairs. RAM contents travel as the linear cart-RAM region;
    /// this restores the selects. The arm latch is transient and stays cleared.
    pub(super) fn bank_state(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SEGMENTS * 2);
        for mapping in self.segments {
            match mapping {
                Mapping::Rom(bank) => out.extend_from_slice(&[0, bank as u8]),
                Mapping::Ram(bank) => out.extend_from_slice(&[1, bank as u8]),
            }
        }
        out
    }

    pub(super) fn restore_bank_state(&mut self, bytes: &[u8]) {
        for (segment, pair) in self.segments.iter_mut().zip(bytes.chunks_exact(2)) {
            *segment = if pair[0] == 1 {
                Mapping::Ram(pair[1] as usize % RAM_BANKS)
            } else {
                Mapping::Rom(pair[1] as usize % self.rom_banks.max(1))
            };
        }
        self.armed = None;
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = usize::from(address & 0x0FFF);
        let within = offset % SEGMENT_SIZE;
        match self.segments[offset / SEGMENT_SIZE] {
            Mapping::Rom(bank) => self.image[bank * ROM_BANK_SIZE + within],
            Mapping::Ram(bank) => self.ram[bank * RAM_BANK_SIZE + within % RAM_BANK_SIZE],
        }
    }
}
