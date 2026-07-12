//! Cartridge ROM. The 6507 sees a 4 KB window; smaller ROMs mirror up,
//! larger boards bank-switch it through address hotspots.

pub enum Cartridge {
    Rom2K(Box<[u8; 0x800]>),
    Rom4K(Box<[u8; 0x1000]>),
    Banked(Banked),
}

/// A hotspot-switched board (F8/F6/F4): any access to a hotspot address
/// selects its 4 KB bank; the data bus is irrelevant.
pub struct Banked {
    data: Vec<u8>,
    bank: usize,
    hotspot_base: u16,
    banks: usize,
    /// Superchip (SARA) cart RAM: write port at window offsets
    /// $000–$07F, read port at $080–$0FF.
    superchip_ram: Option<Box<[u8; SUPERCHIP_RAM_SIZE]>>,
}

const SUPERCHIP_RAM_SIZE: usize = 0x80;

impl Banked {
    fn hotspot(&mut self, address: u16) {
        let decoded = address & 0x1FFF;
        let offset = decoded.wrapping_sub(self.hotspot_base) as usize;
        if decoded >= self.hotspot_base && offset < self.banks {
            self.bank = offset;
        }
    }

    fn byte(&self, address: u16) -> u8 {
        let offset = (address & 0x0FFF) as usize;
        if let Some(ram) = &self.superchip_ram
            && offset < 2 * SUPERCHIP_RAM_SIZE
        {
            return ram[offset % SUPERCHIP_RAM_SIZE];
        }
        self.data[self.bank * 0x1000 + offset]
    }
}

/// The RAM ports shadow the bottom 256 bytes of every bank, so a Superchip
/// dump repeats each bank's first 128 bytes of filler into the next 128.
fn has_superchip_signature(rom: &[u8]) -> bool {
    rom.chunks_exact(0x1000)
        .all(|bank| bank[..SUPERCHIP_RAM_SIZE] == bank[SUPERCHIP_RAM_SIZE..2 * SUPERCHIP_RAM_SIZE])
}

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    UnsupportedSize(usize),
}

impl Cartridge {
    pub fn load(rom: &[u8]) -> Result<Cartridge, CartridgeError> {
        let banked = |banks: usize, hotspot_base: u16| {
            Cartridge::Banked(Banked {
                data: rom.to_vec(),
                bank: 0,
                hotspot_base,
                banks,
                superchip_ram: has_superchip_signature(rom)
                    .then(|| Box::new([0; SUPERCHIP_RAM_SIZE])),
            })
        };
        match rom.len() {
            0x800 => Ok(Cartridge::Rom2K(Box::new(rom.try_into().unwrap()))),
            0x1000 => Ok(Cartridge::Rom4K(Box::new(rom.try_into().unwrap()))),
            0x2000 => Ok(banked(2, 0x1FF8)),
            0x4000 => Ok(banked(4, 0x1FF6)),
            0x8000 => Ok(banked(8, 0x1FF4)),
            size => Err(CartridgeError::UnsupportedSize(size)),
        }
    }

    pub fn read(&mut self, address: u16, bus: u8) -> u8 {
        if let Cartridge::Banked(banked) = self {
            banked.hotspot(address);
            if let Some(ram) = &mut banked.superchip_ram {
                let offset = (address & 0x0FFF) as usize;
                // The cart slot has no R/W line: a write-port read still
                // stores, latching the floating bus byte the CPU also sees.
                if offset < SUPERCHIP_RAM_SIZE {
                    ram[offset] = bus;
                    return bus;
                }
            }
        }
        self.peek(address)
    }

    /// A write cycle on the cart bus: no data lands in ROM, but the address
    /// still drives the hotspot decode and the Superchip write port.
    pub fn write_access(&mut self, address: u16, data: u8) {
        if let Cartridge::Banked(banked) = self {
            banked.hotspot(address);
            if let Some(ram) = &mut banked.superchip_ram {
                let offset = (address & 0x0FFF) as usize;
                if offset < SUPERCHIP_RAM_SIZE {
                    ram[offset] = data;
                }
            }
        }
    }

    /// Side-effect-free read for inspection: never trips a hotspot.
    pub fn peek(&self, address: u16) -> u8 {
        match self {
            Cartridge::Rom2K(rom) => rom[(address & 0x7FF) as usize],
            Cartridge::Rom4K(rom) => rom[(address & 0xFFF) as usize],
            Cartridge::Banked(banked) => banked.byte(address),
        }
    }
}
