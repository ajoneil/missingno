//! The cartridge as the board sees it: a flat ROM image behind the two chip
//! selects the '139 decodes — `/EXM2` over $0000-$7FFF and `/EXM1` over
//! $8000-$BFFF. No SG-1000 board switches banks; the mapper carts are
//! Mark III-era.

/// The `/EXM2` window. A multi-ROM board carries no address decoder of its
/// own, so a ROM smaller than the window answers all of it.
const MIRRORED_WINDOW: usize = 0x8000;
/// Both cartridge windows together: $0000-$BFFF.
const CARTRIDGE_SPAN: usize = 0xC000;

/// What the Z80 reads where nothing on the board drives the data bus.
pub const UNDRIVEN: u8 = 0xFF;

pub struct Cartridge {
    rom: Vec<u8>,
    /// Set for a power-of-two image inside `/EXM2`, which then repeats
    /// through the window on its own address lines.
    mirror_mask: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CartridgeError {
    /// A flat image spans at most the two cartridge windows.
    UnsupportedSize(usize),
}

impl Cartridge {
    pub fn load(rom: &[u8]) -> Result<Cartridge, CartridgeError> {
        if rom.is_empty() || rom.len() > CARTRIDGE_SPAN {
            return Err(CartridgeError::UnsupportedSize(rom.len()));
        }
        let mirror_mask =
            (rom.len().is_power_of_two() && rom.len() <= MIRRORED_WINDOW).then(|| rom.len() - 1);
        Ok(Cartridge {
            rom: rom.to_vec(),
            mirror_mask,
        })
    }

    pub fn read(&self, address: u16) -> u8 {
        let offset = address as usize;
        match self.mirror_mask {
            Some(mask) if offset < MIRRORED_WINDOW => self.rom[offset & mask],
            _ => self.rom.get(offset).copied().unwrap_or(UNDRIVEN),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_image_repeats_through_the_lower_window() {
        let mut rom = vec![0; 0x2000];
        rom[0] = 0x42;
        let cart = Cartridge::load(&rom).unwrap();
        assert_eq!(cart.read(0x0000), 0x42);
        assert_eq!(cart.read(0x2000), 0x42);
        assert_eq!(cart.read(0x6000), 0x42);
        // ROM1's region belongs to a second chip this image does not carry.
        assert_eq!(cart.read(0x8000), UNDRIVEN);
    }

    #[test]
    fn a_large_image_runs_flat_into_the_upper_window() {
        let mut rom = vec![0; 0xC000];
        rom[0xA000] = 0x5A;
        let cart = Cartridge::load(&rom).unwrap();
        assert_eq!(cart.read(0xA000), 0x5A);
        assert_eq!(cart.read(0x2000), 0x00);
    }

    #[test]
    fn an_image_past_the_windows_is_rejected() {
        let too_big = Cartridge::load(&vec![0; 0x10000]).err();
        assert_eq!(too_big, Some(CartridgeError::UnsupportedSize(0x10000)));
        assert_eq!(
            Cartridge::load(&[]).err(),
            Some(CartridgeError::UnsupportedSize(0))
        );
    }
}
