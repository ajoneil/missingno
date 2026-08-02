//! Dot spans: stretches of dots on which every PPU edge body is a fixed point,
//! so the rise and fall skip straight to their sleeping-dot values.
//!
//! Nothing is deferred. `tick_dot`, the divider chain, the CGB clock-domain
//! capture and the standalone register-sync capture all run on every dot, so LY,
//! the mode, the locks and every observation surface stay exact at every
//! instant — a sleeping dot has no state to reconstruct and no sync seam. The
//! stretch length is *predicted*, never advanced: RUTU's next rise is the one
//! edge the eligibility cannot survive, and everything else that could end it
//! (LY, POPU, the LYC coincidence, the WY match) moves only downstream of that
//! rise, so the line-end arithmetic bounds them all.

use super::rendering::Mode;

/// One line — the recurrence of the line-end pulse the arming predicts.
const SPAN_CAP: u32 = 456;

/// Below this the per-dot eligibility test is cheaper than arming, so a short
/// stretch runs dot by dot instead.
const THRESHOLD: u32 = 16;

/// What a sleeping dot may assume, and what has to happen before sleep resumes.
pub(in crate::ppu) struct DotSpan {
    /// This dot's rise and fall bodies are proven inert.
    asleep: bool,
    /// Further dots the arming has already proven inert.
    dots: u32,
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
}

impl Default for DotSpan {
    fn default() -> Self {
        Self {
            asleep: false,
            dots: 0,
            dirty: false,
            ly_eq_lyc: false,
            vblank: false,
            mode: Mode::HorizontalBlank,
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

    /// Spend one of the armed dots.
    pub(super) fn take_armed_dot(&mut self) -> bool {
        if self.dots == 0 {
            return false;
        }
        self.dots -= 1;
        true
    }

    /// Arm the dots between this one and the line-end pulse. A stretch too
    /// short to pay for the arithmetic is left to the per-dot test.
    pub(super) fn arm(&mut self, dots_to_line_end: u32) {
        let dots = dots_to_line_end.min(SPAN_CAP);
        self.dots = if dots >= THRESHOLD { dots - 1 } else { 0 };
    }

    /// Wake now, and hold sleep off until an M-boundary fall has carried the
    /// write into the crossings.
    pub(super) fn invalidate(&mut self) {
        self.asleep = false;
        self.dots = 0;
        self.dirty = true;
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
    }

    pub(super) fn wake(&mut self) {
        self.asleep = false;
    }

    /// A full dot ran: record the STAT conditions its evaluation latched.
    pub(super) fn settle(&mut self, ly_eq_lyc: bool, vblank: bool) {
        self.ly_eq_lyc = ly_eq_lyc;
        self.vblank = vblank;
    }
}
