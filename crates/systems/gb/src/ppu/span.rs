//! Dot spans: stretches of dots on which every PPU edge body is a fixed point,
//! so the rise and fall skip straight to their sleeping-dot values.
//!
//! Nothing is deferred. `tick_dot`, the divider chain, the CGB clock-domain
//! capture and the standalone register-sync capture all run on every dot, so LY,
//! the mode, the locks and every observation surface stay exact at every
//! instant — a sleeping dot has no state to reconstruct and no sync seam.

use super::rendering::Mode;

/// What a sleeping dot may assume, and what has to happen before sleep resumes.
pub(in crate::ppu) struct DotSpan {
    /// This dot's rise and fall bodies are proven inert. The fall decides
    /// afresh for the next dot.
    asleep: bool,
    /// A PPU-visible write landed. The register crossings capture on named
    /// M-cycle edges (the STAT enables and LYC synchronisers, the window
    /// register file), so this clears on an M-boundary fall that has run them —
    /// never on the write's own dot.
    dirty: bool,
    /// ROPO as the last STAT evaluation saw it.
    ly_eq_lyc: bool,
    /// POPU as the last STAT evaluation saw it.
    vblank: bool,
    /// The mode a sleeping dot's readers take. Every mode transition is
    /// downstream of RUTU, which ends the span.
    mode: Mode,
    /// Slept dots since the stretch began, for the shadow's sampling.
    #[cfg(debug_assertions)]
    slept: u16,
}

impl Default for DotSpan {
    fn default() -> Self {
        Self {
            asleep: false,
            dirty: false,
            ly_eq_lyc: false,
            vblank: false,
            mode: Mode::HorizontalBlank,
            #[cfg(debug_assertions)]
            slept: 0,
        }
    }
}

impl DotSpan {
    pub(super) fn asleep(&self) -> bool {
        self.asleep
    }

    /// The mode the last full dot settled on — what a sleeping dot's readers
    /// take.
    pub(super) fn mode(&self) -> Mode {
        self.mode
    }

    /// Wake now, and hold sleep off until an M-boundary fall has carried the
    /// write into the crossings.
    pub(super) fn invalidate(&mut self) {
        self.asleep = false;
        self.dirty = true;
        #[cfg(debug_assertions)]
        {
            self.slept = 0;
        }
    }

    /// This slept dot carries the shadow's full-state comparison.
    #[cfg(debug_assertions)]
    pub(super) fn deep_check(&self) -> bool {
        self.slept
            .is_multiple_of(super::span_shadow::DEEP_CHECK_STRIDE)
    }

    /// This M-boundary fall ran the register crossings, so every write since
    /// the last one has reached the IRQ block and the window decode.
    pub(super) fn note_crossings_captured(&mut self) {
        self.dirty = false;
    }

    /// No write is waiting on a crossing, and the STAT condition vector still
    /// reads as the last evaluation latched it.
    pub(super) fn quiet(&self, ly_eq_lyc: bool, vblank: bool) -> bool {
        !self.dirty && self.ly_eq_lyc == ly_eq_lyc && self.vblank == vblank
    }

    pub(super) fn sleep(&mut self, mode: Mode) {
        self.asleep = true;
        self.mode = mode;
        #[cfg(debug_assertions)]
        {
            self.slept = self.slept.wrapping_add(1);
        }
    }

    pub(super) fn wake(&mut self) {
        self.asleep = false;
        #[cfg(debug_assertions)]
        {
            self.slept = 0;
        }
    }

    /// A full dot ran: record the STAT conditions its evaluation latched and
    /// the mode it settled on.
    pub(super) fn settle(&mut self, mode: Mode, ly_eq_lyc: bool, vblank: bool) {
        self.mode = mode;
        self.ly_eq_lyc = ly_eq_lyc;
        self.vblank = vblank;
    }
}
