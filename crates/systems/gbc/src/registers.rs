//! The address map the CGB adds over the DMG's: the eight banked work-RAM
//! banks, the extra OAM rows, and the CGB register file.

use missingno_gb::ppu::Ppu;
use missingno_gb::ppu::memory::Vram;

use crate::vram_dma::TransferMode;
use crate::{Cgb, CgbPpu, CgbVram, ColorRegister};

impl Cgb {
    /// Index into `extra_oam` for a $FEA0-$FEFF address: row from address
    /// bits 6-5, offset from bits 2-0 (bits 3-4 ignored by the decoder).
    fn extra_oam_index(address: u16) -> usize {
        let row = ((address >> 5) & 0x7) as usize - 5;
        row * 8 + (address & 0x7) as usize
    }

    /// Index into `wram` for a work-RAM or echo-RAM address, else `None`.
    fn wram_index(&self, address: u16) -> Option<usize> {
        let banked = |within: u16| self.wram_bank() as usize * 0x1000 + within as usize;
        match address {
            0xC000..=0xCFFF => Some((address - 0xC000) as usize),
            0xD000..=0xDFFF => Some(banked(address - 0xD000)),
            0xE000..=0xEFFF => Some((address - 0xE000) as usize),
            0xF000..=0xFDFF => Some(banked(address - 0xF000)),
            _ => None,
        }
    }

    pub(crate) fn map_read_byte(
        &self,
        address: u16,
        ppu: &Ppu<CgbPpu>,
        vram: &CgbVram,
    ) -> Option<u8> {
        if let Some(i) = self.wram_index(address) {
            return Some(self.wram[i]);
        }
        match address {
            0xFEA0..=0xFEFF => Some(self.extra_oam[Self::extra_oam_index(address)]),
            // DMG-compat locks out the speed/banking/priority registers and
            // the $FF74 scratch byte — open bus for the rest of the session.
            0xFF4C | 0xFF4D | 0xFF6C | 0xFF70 | 0xFF74 if self.dmg_compat => Some(0xFF),
            // KEY0: boot-locked; reads the latched mode ($00 = CGB).
            0xFF4C => Some(0x00),
            0xFF4D => Some(0x7E | ((self.double_speed as u8) << 7) | self.key1_armed as u8), // KEY1
            0xFF4F => Some(vram.read_bank_select()),                                         // VBK
            // HDMA1-4 are write-only.
            0xFF51..=0xFF54 => Some(0xFF),
            // HDMA5 status: bit 7 = 0 while an HDMA is active, blocks-left-minus-1
            // in bits 6-0. Idle/done/stopped reads bit 7 = 1 (done = $FF). A GDMA
            // is never observable here — it holds the CPU for its whole duration.
            0xFF55 => {
                let visible = (self.vram_dma.cursor.remaining / 16)
                    .saturating_sub(self.vram_dma.arb.granted_ahead as u16);
                let active = self.vram_dma.cursor.mode == TransferMode::HBlank && visible > 0;
                Some(((!active as u8) << 7) | (visible.wrapping_sub(1) & 0x7F) as u8)
            }
            0xFF68 => Some(
                ppu.model()
                    .read_color_register(ColorRegister::BackgroundIndex),
            ), // BCPS
            0xFF69 => Some(
                ppu.model()
                    .read_color_register(ColorRegister::BackgroundData),
            ), // BCPD
            0xFF6A => Some(ppu.model().read_color_register(ColorRegister::ObjectIndex)), // OCPS
            0xFF6B => Some(ppu.model().read_color_register(ColorRegister::ObjectData)),  // OCPD
            0xFF6C => Some(ppu.read_object_priority()),                                  // OPRI
            0xFF70 => Some(self.svbk | 0xF8), // SVBK: bits 0-2
            0xFF72 => Some(self.ff72),
            0xFF73 => Some(self.ff73),
            0xFF74 => Some(self.ff74),
            0xFF75 => Some(0x8F | self.ff75),
            _ => None,
        }
    }

    pub(crate) fn map_write_byte(
        &mut self,
        address: u16,
        value: u8,
        ppu: &mut Ppu<CgbPpu>,
        vram: &mut CgbVram,
    ) -> bool {
        if let Some(i) = self.wram_index(address) {
            self.wram[i] = value;
            return true;
        }
        match address {
            0xFEA0..=0xFEFF => {
                self.extra_oam[Self::extra_oam_index(address)] = value;
                true
            }
            // DMG-compat locks out the speed/banking/priority/VRAM-DMA
            // registers and the $FF74 scratch byte.
            0xFF4D | 0xFF51..=0xFF55 | 0xFF6C | 0xFF70 | 0xFF74 if self.dmg_compat => true,
            0xFF4C => true, // KEY0: boot-locked, ignore
            0xFF4D => {
                self.key1_armed = value & 0x01 != 0;
                true
            }
            0xFF4F => {
                vram.write_bank_select(value); // VBK
                true
            }
            0xFF51 => {
                self.vram_dma.cursor.source =
                    (self.vram_dma.cursor.source & 0x00FF) | ((value as u16) << 8);
                true
            }
            0xFF52 => {
                self.vram_dma.cursor.source =
                    (self.vram_dma.cursor.source & 0xFF00) | (value & 0xF0) as u16;
                true
            }
            0xFF53 => {
                self.vram_dma.cursor.dest =
                    ((value as u16) << 8) | (self.vram_dma.cursor.dest & 0x00FF);
                true
            }
            0xFF54 => {
                self.vram_dma.cursor.dest =
                    (self.vram_dma.cursor.dest & 0xFF00) | (value & 0xF0) as u16;
                true
            }
            0xFF55 => {
                let length = ((value & 0x7F) as u16 + 1) * 16;
                self.vram_dma.arb.granted_ahead = 0;
                self.vram_dma.arb.grant_counted = false;
                self.vram_dma.arb.pend_granted = false;
                if value & 0x80 != 0 {
                    // Arm HDMA: one 16-byte block per HBlank. A block already
                    // latched by the trigger is immune and keeps flowing; an
                    // arm landing during mode 0 pends at this fall's trigger
                    // evaluation. With the LCD off no HBlank will come — the
                    // arm strobe services one block immediately.
                    self.vram_dma.cursor.mode = TransferMode::HBlank;
                    self.vram_dma.cursor.remaining = length;
                    self.vram_dma.arb.armed_this_fall = true;
                    if !ppu.control().video_enabled() {
                        self.vram_dma.block.remaining = 16;
                        self.vram_dma.arb.pend_from_arm = true;
                        self.vram_dma.block.setup_cells.clear();
                        self.vram_dma.block.ready_in.arm(2);
                    }
                } else if self.vram_dma.cursor.mode == TransferMode::HBlank {
                    // bit 7 = 0 while an HDMA runs clears the arming only (no
                    // GDMA starts); a latched block completes. Bits 6-0 are
                    // the length register and store on every write — the
                    // status read reflects them.
                    self.vram_dma.cursor.mode = TransferMode::Idle;
                    self.vram_dma.cursor.remaining = length;
                } else {
                    // GDMA: copy the whole length while holding the CPU.
                    self.vram_dma.cursor.mode = TransferMode::General;
                    self.vram_dma.cursor.remaining = length;
                }
                true
            }
            0xFF68 => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::BackgroundIndex, value); // BCPS
                true
            }
            0xFF69 => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::BackgroundData, value); // BCPD
                true
            }
            0xFF6A => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::ObjectIndex, value); // OCPS
                true
            }
            0xFF6B => {
                ppu.model_mut()
                    .write_color_register(ColorRegister::ObjectData, value); // OCPD
                true
            }
            0xFF6C => {
                ppu.write_object_priority(value); // OPRI
                true
            }
            0xFF70 => {
                self.svbk = value & 0x07;
                true
            }
            0xFF72 => {
                self.ff72 = value;
                true
            }
            0xFF73 => {
                self.ff73 = value;
                true
            }
            0xFF74 => {
                self.ff74 = value;
                true
            }
            0xFF75 => {
                self.ff75 = value & 0x70;
                true
            }
            _ => false,
        }
    }
}
