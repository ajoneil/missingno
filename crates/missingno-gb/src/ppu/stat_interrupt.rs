//! STAT Interrupt Generation subsystem.
//!
//! Owns STAT register primary state (LYC register, enable bits, LYC-match
//! pipeline) and the LALU edge-detection state. Reads LY from LineCounter
//! via method arguments. Does not own `stat_line_active` or `check_stat_edge`
//! — those are Ppu aggregator methods that compose STAT state with
//! rendering and line state.
//!
//! Hardware LYC-match pipeline: PALY (combinational LY==LYC) → ROPO
//! (DFF captured on TALU rising) → RUPO (NOR latch that is transparent
//! during normal operation, since PAGO is static-1). STAT bit 2 on the
//! CPU bus tracks ROPO.Q directly via RUPO's transparent path. STAT IRQ
//! blocking emerges naturally from the edge-triggered LALU capture.
//!
//! STAT bit 2 visible value is derived from `comparison_latched` (ROPO.Q)
//! directly — no separate `comparison_stat_visible` field. Earlier
//! versions modelled RUPO as an asymmetric latch with distinct set/clear
//! methods; that was compensation for a MYTA-edge timing gap closed by
//! the LineEndPipeline extraction (missingno commits `132f8c2` 5a +
//! `88ffcd5` 5b). Compensation simplification follows the hardware-
//! faithful endpoint per the compensation-simplification analysis.

use bitflags::bitflags;

bitflags! {
    #[derive(Copy, Clone)]
    pub struct InterruptFlags: u8 {
        const DUMMY                = 0b10000000;
        const CURRENT_LINE_COMPARE = 0b01000000;
        const OAM_SCAN             = 0b00100000;
        const VERTICAL_BLANK       = 0b00010000;
        const HORIZONTAL_BLANK     = 0b00001000;
    }
}

pub struct StatInterrupt {
    /// LYC register ($FF45). CPU-writable comparison target.
    pub(in crate::ppu) lyc: u8,

    /// Pending LY==LYC comparison result (PALY combinational comparator).
    /// Recomputed at LX counter clock fall and on CPU writes to LYC.
    /// Promoted to `comparison_latched` at LX counter clock rise.
    pub(in crate::ppu) comparison_pending: bool,

    /// Latched LY==LYC result (ROPO DFF output). Unconditionally latched
    /// from `comparison_pending` at each LX counter clock rise. Drives the
    /// LYC STAT interrupt source. Reset only by SYS_RST, NOT by VID_RST.
    ///
    /// Also drives STAT bit 2 directly via the transparent RUPO path:
    /// `ly_eq_lyc_stat()` returns this field.
    pub(in crate::ppu) comparison_latched: bool,

    /// STAT interrupt enable bits (FF41 bits 3-6: ROXE/RUFO/REFE/RUGU
    /// drlatch_ee cells) plus the DUMMY pull-up bit 7 on the read path.
    pub(in crate::ppu) enables: InterruptFlags,

    /// Previous STAT line state for LALU edge detection.
    pub(in crate::ppu) line_was_high: bool,
}

impl StatInterrupt {
    /// Boot-ROM-handoff STAT/LYC state: the
    /// PALY=(LY==LYC) comparator has been combinationally true for many
    /// TALU cycles with LY-register=0 (MYTA-smoothed) and LYC=0, and ROPO
    /// has been capturing PALY=1 on every TALU rising edge. `enables`
    /// preserves the FF41 bit-7 DUMMY pull-up so `read_register` returns
    /// 0x80 with no other enables set.
    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            lyc: 0,
            comparison_pending: true,
            comparison_latched: true,
            enables: InterruptFlags::DUMMY,
            line_was_high: false,
        }
    }

    /// PALY combinational comparator recompute. Called at LX counter clock
    /// fall and on CPU writes to LYC. Computes `pending = (ly == lyc)`.
    pub(in crate::ppu) fn update_comparison(&mut self, ly: u8) {
        self.comparison_pending = ly == self.lyc;
    }

    /// ROPO DFF capture: unconditionally latch `comparison_pending` on
    /// LX counter clock rising.
    pub(in crate::ppu) fn latch_comparison(&mut self) {
        self.comparison_latched = self.comparison_pending;
    }

    /// Whether the ROPO-latched LY==LYC is currently true (LYC IRQ term).
    pub(in crate::ppu) fn ly_eq_lyc(&self) -> bool {
        self.comparison_latched
    }

    /// STAT bit 2 visible value. RUPO is transparent during normal
    /// operation (PAGO static-1), so STAT bit 2 tracks ROPO.Q directly.
    pub(in crate::ppu) fn ly_eq_lyc_stat(&self) -> bool {
        self.comparison_latched
    }

    pub(in crate::ppu) fn lyc(&self) -> u8 {
        self.lyc
    }

    pub(in crate::ppu) fn enables(&self) -> InterruptFlags {
        self.enables
    }

    pub(in crate::ppu) fn line_was_high(&self) -> bool {
        self.line_was_high
    }

    /// Direct setter for `enables`. Used by the STAT write glitch path
    /// (ppu/mod.rs) to install the transient all-bits-high state; glitch
    /// orchestration stays on Ppu.
    pub(in crate::ppu) fn set_enables(&mut self, flags: InterruptFlags) {
        self.enables = flags;
    }

    /// LYC register write: apply the value then recompute PALY against
    /// the provided LY.
    pub(in crate::ppu) fn write_lyc(&mut self, value: u8, ly: u8) {
        self.lyc = value;
        self.update_comparison(ly);
    }

    /// CPU STAT register write primitive. Installs the enable bits from
    /// the written byte (truncated to valid bits). Glitch-edge orchestration
    /// handled by the caller (ppu/mod.rs).
    pub(in crate::ppu) fn write_stat_bits(&mut self, value: u8) {
        self.enables = InterruptFlags::from_bits_truncate(value);
    }

    /// LALU edge-detection primitive. Given the current STAT-line state
    /// (computed by Ppu's `stat_line_active()` aggregator), returns true on
    /// a rising edge and updates `line_was_high`.
    pub(in crate::ppu) fn detect_line_edge(&mut self, stat_line_high: bool) -> bool {
        let edge = stat_line_high && !self.line_was_high;
        self.line_was_high = stat_line_high;
        edge
    }

    /// Directly set `line_was_high`. Used to prime the edge detector at
    /// LCD-enable so the first evaluation produces no false edge.
    pub(in crate::ppu) fn set_line_was_high(&mut self, value: bool) {
        self.line_was_high = value;
    }
}
