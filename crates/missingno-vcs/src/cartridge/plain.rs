//! A plain ROM board: no banking, no RAM, no hotspots.
//!
//! The image fills the window as far as its address lines reach. A 4 KB cart
//! wires A0-A11 and fills the window once; a 2 KB cart wires only A0-A10 and
//! leaves A11 unconnected, so it cannot tell the window's halves apart and
//! answers to both — the one image appears twice, at $F000 and again at $F800.

pub struct Plain {
    image: Vec<u8>,
    /// The address lines the board actually wires.
    decoded: u16,
}

impl Plain {
    pub fn new(rom: &[u8]) -> Plain {
        Plain {
            image: rom.to_vec(),
            decoded: (rom.len() - 1) as u16,
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        self.image[(address & self.decoded) as usize]
    }
}
