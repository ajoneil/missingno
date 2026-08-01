//! The CommaVid board: 2 KB of ROM above 1 KB of RAM, with the RAM's ports
//! wired the opposite way round from every other Atari-era RAM board.
//!
//! The ROM occupies the upper half of the window, $F800-$FFFF, and never
//! banks. The RAM fills the lower half through two ports — the cart edge has
//! no R/W line, so the board infers direction from where in the window you
//! touch — but CommaVid splits it READ-low, $F000-$F3FF, and WRITE-high,
//! $F400-$F7FF: the mirror image of the Superchip and FA arrangement.

const RAM_SIZE: usize = 0x400;
/// The RAM's write port; below it lies the read port, above it the ROM.
const WRITE_PORT: usize = 0x400;
const ROM_BASE: usize = 0x800;

pub struct Cv {
    image: Vec<u8>,
    ram: Box<[u8; RAM_SIZE]>,
}

impl Cv {
    pub fn new(rom: &[u8]) -> Cv {
        Cv {
            image: rom.to_vec(),
            ram: Box::new([0; RAM_SIZE]),
        }
    }

    pub fn read(&mut self, address: u16, bus: u8) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        // No R/W line: a write-port read still stores, latching the floating
        // bus byte the CPU also sees.
        if (WRITE_PORT..ROM_BASE).contains(&offset) {
            self.ram[offset - WRITE_PORT] = bus;
            return bus;
        }
        self.peek(address)
    }

    pub fn write_access(&mut self, address: u16, data: u8) {
        let offset = (address & 0x0FFF) as usize;
        if (WRITE_PORT..ROM_BASE).contains(&offset) {
            self.ram[offset - WRITE_PORT] = data;
        }
    }

    /// The 1 KB cart RAM, all of it.
    pub(super) fn ram(&self) -> &[u8] {
        self.ram.as_slice()
    }

    pub(super) fn ram_mut(&mut self) -> &mut [u8] {
        self.ram.as_mut_slice()
    }

    pub fn peek(&self, address: u16) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        match offset {
            ROM_BASE.. => self.image[offset - ROM_BASE],
            WRITE_PORT.. => self.ram[offset - WRITE_PORT],
            _ => self.ram[offset],
        }
    }
}
