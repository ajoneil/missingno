//! Just enough of a Z80 assembler for the hand-written kernels the board
//! suites drive their consoles with. Raw opcode bytes, so nothing here
//! depends on a CPU crate.

#[derive(Default)]
pub struct Z80Asm {
    bytes: Vec<u8>,
}

impl Z80Asm {
    pub fn new() -> Self {
        Self::default()
    }

    /// The address the next emitted byte lands at, for a backward jump.
    pub fn here(&self) -> u16 {
        self.bytes.len() as u16
    }

    pub fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// Zero-fill up to `address` — the way a kernel reaches a fixed vector.
    pub fn pad_to(&mut self, address: u16) {
        assert!(self.here() <= address);
        self.bytes.resize(address as usize, 0);
    }

    pub fn ld_a(&mut self, value: u8) {
        self.emit(&[0x3E, value]);
    }
    pub fn inc_a(&mut self) {
        self.emit(&[0x3C]);
    }
    pub fn ld_addr_a(&mut self, address: u16) {
        self.emit(&[0x32, address as u8, (address >> 8) as u8]);
    }
    pub fn ld_a_addr(&mut self, address: u16) {
        self.emit(&[0x3A, address as u8, (address >> 8) as u8]);
    }
    pub fn in_port(&mut self, port: u8) {
        self.emit(&[0xDB, port]);
    }
    pub fn out_port(&mut self, port: u8) {
        self.emit(&[0xD3, port]);
    }
    pub fn ei(&mut self) {
        self.emit(&[0xFB]);
    }
    pub fn im1(&mut self) {
        self.emit(&[0xED, 0x56]);
    }
    pub fn ret(&mut self) {
        self.emit(&[0xC9]);
    }
    pub fn jp(&mut self, target: u16) {
        self.emit(&[0xC3, target as u8, (target >> 8) as u8]);
    }

    /// Zero-pad to `size` — the flat image size the board decodes.
    pub fn into_rom(mut self, size: usize) -> Vec<u8> {
        self.bytes.resize(size, 0);
        self.bytes
    }
}
