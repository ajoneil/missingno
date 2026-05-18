// CUFE (OAM SRAM clock) derives from the CPU address bus, not the OAM bus, so a CPU access
// in the OAM range during Mode 2 clocks the SRAM while the scanner owns the buses.
// Corruption formulas are empirical (Pan Docs / SameBoy); they depend on the SRAM cell layout.

use super::Ppu;

/// Read corruption takes priority over write if both are armed in the same M-cycle.
pub(super) enum OamBugKind {
    Read,
    Write,
}

/// `armed = Some` means a CUFE pulse fired in the BOWA→MOPA window.
#[derive(Default)]
pub(crate) struct OamCorruption {
    pub(super) armed: Option<OamBugKind>,
}

const OAM_RANGE: std::ops::RangeInclusive<u16> = 0xFE00..=0xFEFF;

impl Ppu {
    pub(crate) fn arm_oam_bug_for_read(&mut self, address: u16) {
        if OAM_RANGE.contains(&address) {
            self.oam_corruption.armed = Some(OamBugKind::Read);
        }
    }

    /// Does not overwrite an existing Read arming.
    pub(crate) fn arm_oam_bug_for_write(&mut self, address: u16) {
        if OAM_RANGE.contains(&address)
            && !matches!(self.oam_corruption.armed, Some(OamBugKind::Read))
        {
            self.oam_corruption.armed = Some(OamBugKind::Write);
        }
    }

    /// Fired at MOPA (dot 2 rise) — possibly in the M-cycle following arming.
    pub(crate) fn apply_pending_oam_bug(&mut self) {
        match self.oam_corruption.armed.take() {
            Some(OamBugKind::Read) => self.oam_bug_read(),
            Some(OamBugKind::Write) => self.oam_bug_write(),
            None => {}
        }
    }
}

impl Ppu {
    fn oam_bug_write(&mut self) {
        let row = match self.corrupted_oam_row() {
            Some(row) if (8..160).contains(&row) => row,
            _ => return,
        };

        let oam = &mut self.oam;

        let row_word0 = oam.oam_word(row);
        let prev_word0 = oam.oam_word(row - 8);
        let prev_word2 = oam.oam_word(row - 4);

        let glitched = ((row_word0 ^ prev_word2) & (prev_word0 ^ prev_word2)) ^ prev_word2;
        oam.set_oam_word(row, glitched);

        // Bytes 2–7 of the row are overwritten with the preceding row's bytes 2–7.
        for i in 2..8u8 {
            let val = oam.oam_byte(row - 8 + i);
            oam.set_oam_byte(row + i, val);
        }
    }

    /// Read corruption variant selected by `row & 0x18` (SRAM row group).
    fn oam_bug_read(&mut self) {
        let row = match self.corrupted_oam_row() {
            Some(row) if (8..160).contains(&row) => row,
            _ => return,
        };

        let oam = &mut self.oam;

        match row & 0x18 {
            0x10 => {
                // Secondary: 4-input formula on the preceding row, then copy to current and two-back.
                if row < 0x98 {
                    let two_back_word0 = oam.oam_word(row - 16);
                    let prev_word0 = oam.oam_word(row - 8);
                    let row_word0 = oam.oam_word(row);
                    let prev_word2 = oam.oam_word(row - 4);

                    let glitched = (prev_word0 & (two_back_word0 | row_word0 | prev_word2))
                        | (two_back_word0 & row_word0 & prev_word2);
                    oam.set_oam_word(row - 8, glitched);

                    for i in 0..8u8 {
                        let val = oam.oam_byte(row - 8 + i);
                        oam.set_oam_byte(row - 16 + i, val);
                        oam.set_oam_byte(row + i, val);
                    }
                }
            }
            0x00 => {
                // Tertiary/quaternary: involves more distant rows; DMG-specific, varies by unit.
                if row < 0x98 {
                    if row == 0x40 {
                        // Quaternary 8-input formula; emulating deterministic-output units.
                        let row_word0 = oam.oam_word(row);
                        let prev_word2 = oam.oam_word(row - 4);
                        let prev_word1 = oam.oam_word(row - 6);
                        let prev_word0 = oam.oam_word(row - 8);
                        let two_back_word3 = oam.oam_word(row - 14);
                        let two_back_word0 = oam.oam_word(row - 16);
                        let four_back_word0 = oam.oam_word(row - 32);

                        let glitched = (prev_word0
                            & (four_back_word0
                                | two_back_word0
                                | (!prev_word1 & two_back_word3)
                                | prev_word2
                                | row_word0))
                            | (prev_word2 & two_back_word0 & four_back_word0);
                        oam.set_oam_word(row - 8, glitched);
                    } else {
                        // Tertiary 5-input formula; varies by row position.
                        let row_word0 = oam.oam_word(row);
                        let prev_word2 = oam.oam_word(row - 4);
                        let prev_word0 = oam.oam_word(row - 8);
                        let two_back_word0 = oam.oam_word(row - 16);
                        let four_back_word0 = oam.oam_word(row - 32);

                        let glitched = match row {
                            0x20 => {
                                (prev_word0
                                    & (row_word0 | prev_word2 | two_back_word0 | four_back_word0))
                                    | (row_word0 & prev_word2 & two_back_word0 & four_back_word0)
                            }
                            0x60 => {
                                (prev_word0
                                    & (row_word0 | prev_word2 | two_back_word0 | four_back_word0))
                                    | (prev_word2 & two_back_word0 & four_back_word0)
                            }
                            _ => {
                                prev_word0
                                    | (row_word0 & prev_word2 & two_back_word0 & four_back_word0)
                            }
                        };
                        oam.set_oam_word(row - 8, glitched);
                    }

                    for i in 0..8u8 {
                        let val = oam.oam_byte(row - 8 + i);
                        oam.set_oam_byte(row - 16 + i, val);
                        oam.set_oam_byte(row + i, val);
                    }
                }
            }
            _ => {
                // Simple read corruption (Pan Docs formula) for `row & 0x18` = 0x08 or 0x18.
                let row_word0 = oam.oam_word(row);
                let prev_word0 = oam.oam_word(row - 8);
                let prev_word2 = oam.oam_word(row - 4);

                let glitched = prev_word0 | (row_word0 & prev_word2);
                oam.set_oam_word(row - 8, glitched);
                oam.set_oam_word(row, glitched);

                for i in 0..8u8 {
                    let val = oam.oam_byte(row - 8 + i);
                    oam.set_oam_byte(row + i, val);
                }
            }
        }

        // Row 0x80 also copies to row 0 (SRAM physical-layout wraparound).
        if row == 0x80 {
            for i in 0..8u8 {
                let val = oam.oam_byte(row + i);
                oam.set_oam_byte(i, val);
            }
        }
    }

    /// Scanner's current 8-byte row, rounded to the next row boundary.
    fn corrupted_oam_row(&self) -> Option<u8> {
        self.pixel_pipeline
            .as_ref()
            .and_then(|ppu| ppu.scanner_oam_address())
            .map(|address| (address / 8 + 1) * 8)
    }
}
