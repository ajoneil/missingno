//! Just enough of an assembler for test kernels. Included via
//! `#[path]` — integration tests are separate crates, so each pulls in
//! its own copy and uses the subset it needs.
#![allow(dead_code)]

pub struct Asm {
    origin: u16,
    bytes: Vec<u8>,
}

impl Asm {
    pub fn new(origin: u16) -> Self {
        Asm {
            origin,
            bytes: Vec::new(),
        }
    }

    pub fn here(&self) -> u16 {
        self.origin + self.bytes.len() as u16
    }

    pub fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub fn cld(&mut self) {
        self.emit(&[0xD8]);
    }
    pub fn lda_imm(&mut self, value: u8) {
        self.emit(&[0xA9, value]);
    }
    pub fn ldx_imm(&mut self, value: u8) {
        self.emit(&[0xA2, value]);
    }
    pub fn txs(&mut self) {
        self.emit(&[0x9A]);
    }
    pub fn sta_zp(&mut self, address: u8) {
        self.emit(&[0x85, address]);
    }
    pub fn stx_zp(&mut self, address: u8) {
        self.emit(&[0x86, address]);
    }
    pub fn inx(&mut self) {
        self.emit(&[0xE8]);
    }
    pub fn dex(&mut self) {
        self.emit(&[0xCA]);
    }
    pub fn cpx_imm(&mut self, value: u8) {
        self.emit(&[0xE0, value]);
    }
    pub fn bne_to(&mut self, target: u16) {
        let offset = target as i32 - (self.here() as i32 + 2);
        self.emit(&[0xD0, i8::try_from(offset).unwrap() as u8]);
    }
    pub fn jmp_abs(&mut self, target: u16) {
        self.emit(&[0x4C, target as u8, (target >> 8) as u8]);
    }

    /// Pad to 4 KB with the reset vector pointing at the origin.
    pub fn into_rom(self) -> Vec<u8> {
        let mut rom = self.bytes;
        rom.resize(0x1000, 0);
        rom[0xFFC] = self.origin as u8;
        rom[0xFFD] = (self.origin >> 8) as u8;
        rom
    }
}
