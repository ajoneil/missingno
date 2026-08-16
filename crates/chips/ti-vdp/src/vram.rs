//! The DRAM and the pointer that reaches it.

use crate::Vdp;

pub const VRAM_SIZE: usize = 0x4000;

/// A VRAM pointer value: 14 bits, wrapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct VramAddress(u16);

const POINTER_MASK: u16 = 0x3FFF;

impl VramAddress {
    pub(crate) const ZERO: Self = VramAddress(0);

    pub(crate) const fn new(value: u16) -> Self {
        VramAddress(value & POINTER_MASK)
    }

    pub(crate) const fn value(self) -> u16 {
        self.0
    }

    pub(crate) const fn low(self) -> u8 {
        self.0 as u8
    }

    /// The control port's first byte lands in the low eight bits...
    pub(crate) const fn with_low(self, low: u8) -> Self {
        VramAddress((self.0 & 0x3F00) | low as u16)
    }

    /// ...its second byte in the high six.
    pub(crate) const fn with_high(self, high: u8) -> Self {
        VramAddress((self.0 & 0x00FF) | ((high as u16 & 0x3F) << 8))
    }

    pub(crate) const fn incremented(self) -> Self {
        VramAddress::new(self.0.wrapping_add(1))
    }

    /// The DRAM cell this pointer reaches. In 4K mode the address pins
    /// multiplex differently, permuting the pointer's bits (silicon:
    /// vram/4k-mode).
    pub(crate) const fn cell(self, ram_16k: bool) -> usize {
        if ram_16k {
            self.0 as usize
        } else {
            ((self.0 & 0x2000)
                | ((self.0 & 0x1000) >> 6)
                | ((self.0 & 0x0FC0) << 1)
                | (self.0 & 0x003F)) as usize
        }
    }
}

impl Vdp {
    /// The DRAM as it stands, disturbing nothing — cells in physical order, so
    /// a logical pointer value reaches its byte through
    /// [`vram_cell`](Self::vram_cell) rather than by indexing this.
    pub fn vram(&self) -> &[u8; VRAM_SIZE] {
        &self.vram
    }

    /// The byte a pointer value reaches, disturbing nothing — the renderer's
    /// own fetch, so an inspecting consumer reads what the raster reads.
    pub fn vram_cell(&self, address: u16) -> u8 {
        self.fetch(VramAddress::new(address))
    }

    pub(crate) fn fetch(&self, address: VramAddress) -> u8 {
        self.vram[address.cell(self.ram_16k())]
    }

    pub(crate) fn store(&mut self, address: VramAddress, value: u8) {
        let cell = address.cell(self.ram_16k());
        self.vram[cell] = value;
    }
}
