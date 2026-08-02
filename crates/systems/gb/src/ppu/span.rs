//! Dot spans: stretches of dots on which every PPU edge body is a fixed point.
//! The callers skip the whole per-dot dispatch across an armed stretch, and the
//! two quantities that do move — LX and the WUVU/VENA phase — are owed to the
//! next materialisation rather than advanced dot by dot.
//!
//! Everything else a span could be asked about is constant by construction:
//! the stretch's single ender is the RUTU rise, and LY, POPU, the mode, the
//! OAM/VRAM/CRAM locks and the STAT condition vector all move on or downstream
//! of that rise. So a mid-span bus read needs no seam — it reads constants —
//! and only the LX/divider-phase consumers (save states, the morepork columns,
//! the CGB STOP phase) sync before they look.

use super::rendering::Mode;

#[cfg(debug_assertions)]
use super::span_shadow::SpanShadow;
#[cfg(debug_assertions)]
use super::stat_interrupt::StatShadow;
#[cfg(debug_assertions)]
use super::video_control::VideoControl;

/// One line — the recurrence of the line-end pulse the arming predicts.
const SPAN_CAP: u32 = 456;

/// Below this the per-dot eligibility test is cheaper than arming, so a short
/// stretch runs dot by dot instead.
const THRESHOLD: u32 = 16;

/// What a sleeping dot may assume, and what has to happen before sleep resumes.
pub(in crate::ppu) struct DotSpan {
    /// This dot's rise and fall bodies are proven inert.
    asleep: bool,
    /// Further dots the arming has already proven inert. While this stands the
    /// callers do not enter the PPU at all.
    dots: u32,
    /// Dot falls skipped without running the divider chain, owed to the next
    /// materialisation.
    deferred: u32,
    /// A PPU-visible write landed. The register crossings capture on named
    /// M-cycle edges (the STAT enables and LYC synchronisers, the window
    /// register file), so this clears on an M-boundary fall that has run them —
    /// never on the write's own dot.
    dirty: bool,
    /// An M-boundary capture has run since the span went to sleep, so the CGB
    /// clock-domain pair holds the (not drawing, not drawing) fixed point. The
    /// capture before the span may have latched drawing through the XYMU dot-
    /// fall lag, so the first in-span boundary still has to run.
    clock_domain_captured: bool,
    /// ROPO as the last STAT evaluation saw it.
    ly_eq_lyc: bool,
    /// POPU as the last STAT evaluation saw it.
    vblank: bool,
    /// The mode a sleeping dot's readers take. Every mode transition is
    /// downstream of RUTU, which ends the span.
    mode: Mode,
    #[cfg(debug_assertions)]
    shadow: Option<SpanShadow>,
}

impl Default for DotSpan {
    fn default() -> Self {
        Self {
            asleep: false,
            dots: 0,
            deferred: 0,
            dirty: false,
            clock_domain_captured: false,
            ly_eq_lyc: false,
            vblank: false,
            mode: Mode::HorizontalBlank,
            #[cfg(debug_assertions)]
            shadow: None,
        }
    }
}

impl DotSpan {
    pub(super) fn asleep(&self) -> bool {
        self.asleep
    }

    /// Dots proven inert are still standing, so the callers skip the PPU whole.
    pub(super) fn armed(&self) -> bool {
        self.dots > 0
    }

    pub(super) fn deferred(&self) -> u32 {
        self.deferred
    }

    /// The mode the last full dot settled on — what a sleeping dot's readers
    /// take.
    pub(super) fn mode(&self) -> Mode {
        self.mode
    }

    /// Spend one armed dot without running it; its divider and LX arithmetic
    /// joins what the next materialisation owes.
    pub(super) fn defer_dot(&mut self) -> bool {
        if self.dots == 0 {
            return false;
        }
        self.dots -= 1;
        self.deferred += 1;
        true
    }

    /// Take the deferred dot count for materialisation.
    pub(super) fn take_deferred(&mut self) -> u32 {
        std::mem::take(&mut self.deferred)
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
        debug_assert_eq!(self.deferred, 0, "a span was torn down unmaterialised");
        self.asleep = false;
        self.dots = 0;
        self.dirty = true;
        self.clock_domain_captured = false;
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
        self.clock_domain_captured = false;
    }

    /// The CGB clock-domain pair has reached its sleeping fixed point.
    pub(super) fn clock_domain_settled(&self) -> bool {
        self.asleep && self.clock_domain_captured
    }

    pub(super) fn note_clock_domain_captured(&mut self) {
        self.clock_domain_captured = self.asleep;
    }

    /// A full dot ran: record the STAT conditions its evaluation latched.
    pub(super) fn settle(&mut self, ly_eq_lyc: bool, vblank: bool) {
        self.ly_eq_lyc = ly_eq_lyc;
        self.vblank = vblank;
    }

    #[cfg(debug_assertions)]
    pub(super) fn step_shadow(&mut self, video: &VideoControl, stat: &impl StatShadow) {
        self.shadow
            .get_or_insert_with(|| SpanShadow::seed(video))
            .step(video, stat);
    }

    #[cfg(debug_assertions)]
    pub(super) fn compare_shadow(&mut self, video: &VideoControl) {
        if let Some(shadow) = self.shadow.take() {
            shadow.compare(video);
        }
    }
}
