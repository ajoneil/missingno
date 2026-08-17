//! A plain ROM board: the image behind the two chip selects and nothing else.
//! A board with no address decoder of its own cannot tell the halves of the
//! window apart, so an image shorter than `/EXM2` repeats through it — the way
//! the documented multi-ROM boards mirror. Every other board carries this image
//! beneath its RAM.

use super::EXM2_WINDOW;

pub struct Flat {
    image: Vec<u8>,
    /// Set for a power-of-two image inside `/EXM2`, which then repeats through
    /// the window on its own address lines.
    mirror_mask: Option<usize>,
}

impl Flat {
    pub fn new(rom: &[u8]) -> Flat {
        let mirror_mask =
            (rom.len().is_power_of_two() && rom.len() <= EXM2_WINDOW).then(|| rom.len() - 1);
        Flat {
            image: rom.to_vec(),
            mirror_mask,
        }
    }

    /// The byte the ROM drives, or `None` where its address lines don't reach.
    pub fn read(&self, address: u16) -> Option<u8> {
        let offset = address as usize;
        match self.mirror_mask {
            Some(mask) if offset < EXM2_WINDOW => Some(self.image[offset & mask]),
            _ => self.image.get(offset).copied(),
        }
    }
}
