use missingno_gb::ppu::memory::{Vram, VramAddress, VramBank};

/// A CGB BG map attribute byte (VRAM bank 1, one per tile-map cell): bits 2-0
/// BG palette, bit 3 tile VRAM bank, bit 5 X-flip, bit 6 Y-flip, bit 7 BG-to-OBJ
/// priority (bit 4 unused). Rides the BG shifter across its tile's 8 pixels.
#[derive(Copy, Clone, Default)]
pub struct BgAttribute(pub u8);

impl BgAttribute {
    pub fn palette(self) -> u8 {
        self.0 & 0x07
    }

    pub fn tile_bank(self) -> u8 {
        (self.0 >> 3) & 0x01
    }

    pub fn flip_x(self) -> bool {
        self.0 & 0x20 != 0
    }

    pub fn flip_y(self) -> bool {
        self.0 & 0x40 != 0
    }

    /// BG-to-OBJ priority (bit 7): BG colour indices 1-3 of this tile draw over OBJ.
    pub fn priority(self) -> bool {
        self.0 & 0x80 != 0
    }
}

/// CGB video RAM: two 8 KiB banks selected by VBK ($FF4F). Bank 1 additionally
/// carries the BG map attributes (read by the colour fetch as it lands).
#[derive(Default, Clone)]
pub struct CgbVram {
    banks: [VramBank; 2],
    /// VBK bit 0 — the bank the CPU sees at $8000-$9FFF.
    selected: u8,
}

impl CgbVram {
    /// VBK ($FF4F) bit 0 — the bank the CPU sees at $8000-$9FFF.
    pub fn selected_bank(&self) -> u8 {
        self.selected
    }
}

impl Vram for CgbVram {
    fn cpu_read(&self, address: VramAddress) -> u8 {
        self.banks[self.selected as usize].read(address)
    }

    fn cpu_write(&mut self, address: VramAddress, value: u8) {
        self.banks[self.selected as usize].write(address, value);
    }

    fn bank(&self, bank: u8) -> &VramBank {
        &self.banks[bank as usize]
    }

    fn read_bank_select(&self) -> u8 {
        0xFE | self.selected
    }

    fn write_bank_select(&mut self, value: u8) {
        self.selected = value & 0x01;
    }

    fn init_post_boot(&mut self, logo: &[u8; 0x30]) {
        self.banks[0].seed_post_boot(logo);
    }

    /// Rebuild both 8 KiB banks from a linear 16 KiB image (bank 0 then bank 1).
    /// The VBK selection is restored separately through `write_bank_select`.
    fn restore_image(&mut self, bytes: &[u8]) {
        for (bank, chunk) in self.banks.iter_mut().zip(bytes.chunks(0x2000)) {
            *bank = VramBank::from_bytes(chunk);
        }
    }
}
