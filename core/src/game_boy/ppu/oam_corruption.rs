// --- OAM corruption bug ---
//
// On DMG hardware, a design flaw in the OAM SRAM clock generation
// causes corruption when the CPU accesses OAM during Mode 2
// (scanning). The OAM clock signal CUFE is derived from the CPU's
// internal address bus — not the OAM address bus. ASAM blocks the
// CPU from driving the OAM *address* bus during scanning, but CUFE
// still sees the CPU address and generates spurious SRAM clock edges.
// This clocks the SRAM while the scanner owns the address/data
// buses, producing garbled reads and writes.
//
// The corruption formulas below are empirical — they describe the
// analog result of SRAM cells being disturbed during bus contention.
// The exact formulas depend on the physical SRAM cell layout (bit
// line routing, parasitic capacitance) and vary by die revision.
// They cannot be derived from a digital gate-level model; GateBoy's
// tri_bus asserts on the collision and fails the oam_bug tests.
//
// OAM is organized as 20 rows of 8 bytes (4 words of 16 bits).
// The scanner advances through one row pair (2 entries = 8 bytes)
// per M-cycle. Corruption targets the row the scanner is currently
// accessing, with effects spilling into adjacent rows.
//
// Sources:
//   Trigger mechanism: GateBoy die analysis (CUFE, BYCU, ASAM)
//   Corruption formulas: Pan Docs "OAM Corruption Bug"
//   Position-dependent read variants: SameBoy (Core/memory.c)

use super::Ppu;

impl Ppu {
    /// Trigger OAM bug write corruption during Mode 2.
    ///
    /// Fires when the CPU's IDU or a CPU write places an OAM-range
    /// address on the bus while the scanner owns the OAM SRAM.
    /// The spurious SRAM clock causes a garbled write to the
    /// scanner's current row.
    pub fn oam_bug_write(&mut self) {
        let row = match self.corrupted_oam_row() {
            Some(row) if (8..160).contains(&row) => row,
            _ => return,
        };

        let oam = &mut self.oam;

        // Corruption of the first word in the row. The three inputs
        // are the row's own first word and two words from the
        // preceding row (its first and third words). The formula
        // models the SRAM cell output under bus contention.
        let row_word0 = oam.oam_word(row);
        let prev_word0 = oam.oam_word(row - 8);
        let prev_word2 = oam.oam_word(row - 4);

        let glitched = ((row_word0 ^ prev_word2) & (prev_word0 ^ prev_word2)) ^ prev_word2;
        oam.set_oam_word(row, glitched);

        // The last 3 words of the row are overwritten with the
        // preceding row's last 3 words (bytes 2–7 copied).
        for i in 2..8u8 {
            let val = oam.oam_byte(row - 8 + i);
            oam.set_oam_byte(row + i, val);
        }
    }

    /// Trigger OAM bug read corruption during Mode 2.
    ///
    /// Read corruption has position-dependent variants because
    /// different SRAM row positions have different physical bit line
    /// routing, producing different parasitic coupling patterns.
    /// The variant is selected by `row & 0x18` (which 8-row group
    /// the row falls into within the SRAM array).
    ///
    /// These variants are revision-specific and even unit-specific.
    /// The formulas here target DMG behaviour.
    pub fn oam_bug_read(&mut self) {
        let row = match self.corrupted_oam_row() {
            Some(row) if (8..160).contains(&row) => row,
            _ => return,
        };

        let oam = &mut self.oam;

        match row & 0x18 {
            0x10 => {
                // Secondary read corruption.
                // The 4-input formula corrupts the preceding row's
                // first word, then the preceding row is copied to
                // both the current row and two rows back.
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
                // Tertiary/quaternary read corruption.
                // These involve more distant rows due to the SRAM
                // physical layout at these addresses. The formulas
                // are DMG-specific and vary even between DMG units.
                if row < 0x98 {
                    if row == 0x40 {
                        // Quaternary (8 inputs). Some DMG units produce
                        // non-deterministic results here; we emulate
                        // the units that produce deterministic output.
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
                        // Tertiary (5 inputs). The exact formula varies
                        // by row position within the SRAM array.
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
                // Simple read corruption (rows where `row & 0x18`
                // is 0x08 or 0x18). This is the Pan Docs "read"
                // formula — the simplest coupling pattern.
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

        // Row 0x80 additionally copies to row 0 — an SRAM array
        // wraparound effect at the physical layout boundary.
        if row == 0x80 {
            for i in 0..8u8 {
                let val = oam.oam_byte(row + i);
                oam.set_oam_byte(i, val);
            }
        }
    }

    /// Which OAM row the scanner is currently accessing.
    ///
    /// OAM is organized as 8-byte rows (2 entries per row). The
    /// scanner's byte address is rounded to the next row boundary.
    /// The corruption fires at T2 of the M-cycle (matching the
    /// hardware CUFE clock).
    fn corrupted_oam_row(&self) -> Option<u8> {
        self.pixel_pipeline
            .as_ref()
            .and_then(|ppu| ppu.scanner_oam_address())
            .map(|address| (address / 8 + 1) * 8)
    }
}
