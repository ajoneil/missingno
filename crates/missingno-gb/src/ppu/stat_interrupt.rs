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
//! CPU bus tracks ROPO.Q via RUPO's transparent path. STAT IRQ blocking
//! emerges naturally from the edge-triggered LALU capture.
//!
//! **Compensation-model note**: `comparison_stat_visible` and its
//! asymmetric set/clear pair (`latch_stat_visible` /
//! `clear_stat_visible_if_no_match`) are **not** hardware-faithful.
//! Hardware RUPO is transparent; this emulator models an asymmetric
//! update cadence to compensate for per-dot resolution plus the
//! register-smoothed LY used by `update_comparison`. The compensation
//! satisfies most hardware-captured tests but produces a known
//! one-M-cycle-early clearing divergence at the LYC=153 scanline
//! boundary. A hardware-faithful rewrite is deferred to a broader
//! STAT-pipeline review.

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
    pub(in crate::ppu) comparison_latched: bool,

    /// STAT bit 2 visible value (compensation-model field; see module
    /// doc). Hardware RUPO is transparent during normal operation and
    /// STAT bit 2 tracks ROPO.Q directly. This field plus the asymmetric
    /// `latch_stat_visible` / `clear_stat_visible_if_no_match` pair
    /// compensates for per-dot resolution + register-smoothed LY in
    /// `update_comparison`.
    pub(in crate::ppu) comparison_stat_visible: bool,

    /// STAT interrupt enable bits (FF41 bits 3-6: ROXE/RUFO/REFE/RUGU
    /// drlatch_ee cells) plus the DUMMY pull-up bit 7 on the read path.
    /// Renamed from `stat_flags` per OQ.8 container-context naming.
    pub(in crate::ppu) enables: InterruptFlags,

    /// Previous STAT line state for LALU edge detection.
    pub(in crate::ppu) line_was_high: bool,
}

impl StatInterrupt {
    /// PALY combinational comparator recompute. Called at LX counter clock
    /// fall and on CPU writes to LYC. When `suppress_onset` is true, only
    /// false-to-true transitions (new match onset) are suppressed; clearing
    /// passes through, modelling the MYTA propagation-race window (see
    /// `myta-investigation.md`).
    pub(in crate::ppu) fn update_comparison(&mut self, ly: u8, suppress_onset: bool) {
        let result = ly == self.lyc;
        if suppress_onset {
            if !result {
                self.comparison_pending = false;
            }
            return;
        }
        self.comparison_pending = result;
    }

    /// ROPO DFF capture: unconditionally latch `comparison_pending` on
    /// LX counter clock rising.
    pub(in crate::ppu) fn latch_comparison(&mut self) {
        self.comparison_latched = self.comparison_pending;
    }

    /// Compensation-model set path for STAT bit 2: capture
    /// `comparison_pending` on LX counter clock rising.
    ///
    /// Hardware RUPO is transparent during normal operation; STAT bit 2
    /// tracks ROPO.Q directly. This method pairs with
    /// `clear_stat_visible_if_no_match` to model an asymmetric set/clear
    /// cadence that compensates for per-dot resolution + register-
    /// smoothed LY. See module doc for full context.
    pub(in crate::ppu) fn latch_stat_visible(&mut self) {
        self.comparison_stat_visible = self.comparison_pending;
    }

    /// Compensation-model clear path for STAT bit 2: clear
    /// `comparison_stat_visible` when `comparison_pending` is false,
    /// called at LX counter clock falling on scanline boundaries.
    ///
    /// Not hardware-faithful. Hardware RUPO is transparent (PAGO is
    /// static-1 during normal operation; the "PAGO drives immediate
    /// clear" framing in prior versions of this comment was hardware-
    /// incorrect). The asymmetric set/clear pair compensates for
    /// per-dot resolution + register-smoothed LY in `update_comparison`.
    /// See module doc.
    pub(in crate::ppu) fn clear_stat_visible_if_no_match(&mut self) {
        if !self.comparison_pending {
            self.comparison_stat_visible = false;
        }
    }

    /// Whether the ROPO-latched LY==LYC is currently true (LYC IRQ term).
    pub(in crate::ppu) fn ly_eq_lyc(&self) -> bool {
        self.comparison_latched
    }

    /// RUPO output: STAT bit 2 visible value. Clears immediately when
    /// comparison goes false; onset follows ROPO latch cadence.
    pub(in crate::ppu) fn ly_eq_lyc_stat(&self) -> bool {
        self.comparison_stat_visible
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
    /// (ppu/mod.rs) to install the transient all-bits-high state per PW.2
    /// — glitch orchestration stays on Ppu.
    pub(in crate::ppu) fn set_enables(&mut self, flags: InterruptFlags) {
        self.enables = flags;
    }

    /// LYC register write: apply the value then recompute PALY against
    /// the provided LY, subject to `suppress_onset`.
    pub(in crate::ppu) fn write_lyc(&mut self, value: u8, ly: u8, suppress_onset: bool) {
        self.lyc = value;
        self.update_comparison(ly, suppress_onset);
    }

    /// CPU STAT register write primitive. Installs the enable bits from
    /// the written byte (truncated to valid bits). Glitch-edge orchestration
    /// handled by the caller (ppu/mod.rs) per PW.2.
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
