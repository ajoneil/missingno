//! The Wickstead Design prototype board (the unreleased Pink Panther cart): the
//! most unusual 8 KB scheme, with eight 1 KB banks and no fixed segment at all.
//!
//! The window splits into four 1 KB segments, and a read of TIA space $30-$3F
//! loads one of eight arrangements naming the bank in each. Every segment moves,
//! so code that strobes has to sit where the same bytes survive the switch.
//!
//! The switch is delayed: the arrangement settles about three CPU cycles after
//! the hotspot read, so a segment read taken immediately after one still sees
//! the old bank.
//!
//! It also carries 64 bytes of RAM, read-low at $F000-$F03F and write-high at
//! $F040-$F07F — the opposite of a Superchip — shadowing segment 0's ROM there.

const BANK_SIZE: usize = 0x400;
const SEGMENTS: usize = 4;
const RAM_SIZE: usize = 0x40;
/// The RAM's write port; below it lies the read port.
const WRITE_PORT: usize = 0x40;

/// CPU cycles from the hotspot read until the arrangement settles.
const SWITCH_DELAY: u8 = 3;

/// Which 1 KB bank sits in each segment, per hotspot. The low three address
/// bits pick the arrangement, so $38-$3F mirror $30-$37.
const ARRANGEMENTS: [[usize; SEGMENTS]; 8] = [
    [0, 0, 1, 3],
    [0, 1, 2, 3],
    [4, 5, 6, 7],
    [7, 4, 2, 3],
    [0, 0, 6, 7],
    [0, 1, 7, 6],
    [2, 3, 4, 5],
    [6, 0, 5, 1],
];

pub struct Wd {
    image: Vec<u8>,
    arrangement: usize,
    /// An arrangement selected but not yet settled, and the cycles left.
    pending: Option<(usize, u8)>,
    ram: [u8; RAM_SIZE],
}

impl Wd {
    pub fn new(rom: &[u8]) -> Wd {
        Wd {
            image: rom.to_vec(),
            arrangement: 0,
            pending: None,
            ram: [0; RAM_SIZE],
        }
    }

    /// The arrangement a read selects. The hotspots live in TIA space, below
    /// the window: A12 and A7 low. Only three bits pick the arrangement, so
    /// $38-$3F mirror $30-$37.
    fn hotspot(address: u16) -> Option<usize> {
        match address & 0x1080 == 0 && address & 0x3F >= 0x30 {
            true => Some(usize::from(address & 0x07)),
            false => None,
        }
    }

    /// One cycle of the settling delay.
    fn settle(&mut self) {
        if let Some((arrangement, remaining)) = self.pending {
            match remaining - 1 {
                0 => {
                    self.arrangement = arrangement;
                    self.pending = None;
                }
                remaining => self.pending = Some((arrangement, remaining)),
            }
        }
    }

    fn bank(&self, address: u16) -> usize {
        let segment = usize::from(address & 0x0FFF) / BANK_SIZE;
        ARRANGEMENTS[self.arrangement][segment]
    }

    pub fn read(&mut self, address: u16, residue: u8) -> Option<u8> {
        self.settle();
        if let Some(arrangement) = Wd::hotspot(address) {
            self.pending = Some((arrangement, SWITCH_DELAY));
        }
        if !super::selects_window(address) {
            return None;
        }
        let offset = usize::from(address & 0x0FFF);
        // No R/W line: a write-port read still stores, latching the floating
        // bus byte the CPU also sees.
        if (WRITE_PORT..2 * WRITE_PORT).contains(&offset) {
            self.ram[offset - WRITE_PORT] = residue;
            return Some(residue);
        }
        Some(self.peek(address))
    }

    /// Only a read selects an arrangement, so a write just walks the delay on.
    pub fn write_access(&mut self, address: u16, data: u8) {
        self.settle();
        if !super::selects_window(address) {
            return;
        }
        let offset = usize::from(address & 0x0FFF);
        if (WRITE_PORT..2 * WRITE_PORT).contains(&offset) {
            self.ram[offset - WRITE_PORT] = data;
        }
    }

    /// The full ROM image, all banks in file order, for the debugger's
    /// bank-complete `rom` region.
    pub(super) fn rom(&self) -> &[u8] {
        &self.image
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = usize::from(address & 0x0FFF);
        if offset < RAM_SIZE {
            return self.ram[offset];
        }
        self.image[self.bank(address) * BANK_SIZE + offset % BANK_SIZE]
    }
}
