//! The horizontal sync counter — the TIA's line-timing spine.
//!
//! A position counter clocked by a two-phase clock (the colour clock divided
//! by four) whose decode matrix produces every horizontal control line. One
//! grid drives both the playfield serialiser and — gated by the HBlank latch
//! as the object motion clock — the movable objects, so the two cannot drift.
//!
//! The counter runs 0..=56 (a 57-count period at ¼ CLK, 57×4 = 228 CLK). We
//! hold the position at full colour-clock resolution (`position`, 0..228 =
//! HCount×4 + sub-phase); the polynomial LFSR the silicon actually shifts is
//! not software-observable, only the decode *timing* is. Named decodes, by CLK:
//!
//! ```text
//!   CLK   HCount  control line
//!    16     4     SHS   set HSYNC
//!    32     8     RHS   reset HSYNC
//!    48    12     RCB   reset colour burst
//!    64    16     RHB   reset HBlank (enable output), latched [HB] +4 CLK
//!    72    18     LRHB  late RHB, used instead of RHB when HMOVE holds blank
//!   144    36     CNT   centre: restart / reflect the playfield (+4 → CNTD)
//!   224    56     SHB   start HBlank, self-reset HCount → 0 (+4 → wrap at 228)
//! ```
//!
//! The HB latch is set at SHB and released 4 CLK after RHB (first visible pixel
//! at CLK 68), or 4 CLK after LRHB (CLK 76) when an HMOVE has extended the
//! blank — the 8-pixel HMOVE comb.

use super::{CLOCKS_PER_LINE, HBLANK_CLOCKS, LATE_HBLANK_CLOCKS, VISIBLE_CLOCKS};

/// The RHB/LRHB decode: sampled here, the HMOVE latch chooses which reset
/// releases the HB latch this line.
const RESET_HBLANK_DECODE: u16 = 64;

/// What the beam is doing at the current colour clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Beam {
    /// Horizontal blank: the HB latch is set, nothing reaches the screen and
    /// the object motion clock (MOTCK) is gated off.
    Blank,
    /// A visible column hidden by the HMOVE comb (extended blank): output
    /// forced to background, MOTCK still gated so the objects resume late.
    Comb(u8),
    /// A visible column: compose and render it, and MOTCK reaches the objects.
    Pixel(u8),
}

pub(crate) struct HSyncCounter {
    /// Colour clocks since line start (0..228 = HCount×4 + two-phase sub-phase).
    position: u16,
    /// Where the HB latch releases this line: 68 normally, or 76 (LRHB) when an
    /// HMOVE extended the blank. Chosen at the RHB decode.
    hblank_release: u16,
}

impl HSyncCounter {
    pub(crate) fn new() -> Self {
        HSyncCounter {
            position: 0,
            hblank_release: HBLANK_CLOCKS,
        }
    }

    pub(crate) fn position(&self) -> u16 {
        self.position
    }

    /// The two-phase sub-phase within the 4-CLK cycle (0..4).
    pub(crate) fn phase(&self) -> u16 {
        self.position % 4
    }

    /// The beam state at the current position: blank, comb-hidden, or a
    /// visible pixel. Object MOTCK ticks and playfield output run on `Pixel`.
    pub(crate) fn beam(&self) -> Beam {
        if self.position < HBLANK_CLOCKS {
            return Beam::Blank;
        }
        let x = (self.position - HBLANK_CLOCKS) as u8;
        if self.position < self.hblank_release {
            Beam::Comb(x)
        } else {
            Beam::Pixel(x)
        }
    }

    /// Advance one colour clock. `hmove_extends` is the HMOVE latch as sampled
    /// at the RHB decode. Returns `true` on the SHB wrap (line complete).
    pub(crate) fn advance(&mut self, hmove_extends: bool) -> bool {
        if self.position == RESET_HBLANK_DECODE {
            self.hblank_release = if hmove_extends {
                LATE_HBLANK_CLOCKS
            } else {
                HBLANK_CLOCKS
            };
        }
        self.position += 1;
        if self.position == CLOCKS_PER_LINE {
            self.reset_line();
            return true;
        }
        false
    }

    /// Whether MOTCK reaches the objects this clock: the HB gate opens one
    /// colour clock ahead of the pixel window (N90's measured per-line edges
    /// run x=−1..158 — one rise on the last blank clock, none on the last
    /// visible one).
    pub(crate) fn motck_fires(&self) -> bool {
        (self.position + 1) % CLOCKS_PER_LINE >= self.hblank_release
    }

    /// Whether this colour clock is the line's last stuff slot — the final
    /// H@1 of the line-fixed grid (stuff slots sit at position ≡ 1 mod 4;
    /// die-measured drift cadence), with no committing MOTCK edge remaining
    /// before the wrap.
    pub(crate) fn final_stuff_slot(&self) -> bool {
        self.position % 4 == 1 && self.position + 4 >= CLOCKS_PER_LINE
    }

    /// Force the line back to its start (the SHB wrap, or an RSYNC strobe).
    pub(crate) fn reset_line(&mut self) {
        self.position = 0;
        self.hblank_release = HBLANK_CLOCKS;
    }

    /// Visible columns already drawn this line — the ones RSYNC keeps before
    /// blanking the truncated remainder.
    pub(crate) fn columns_drawn(&self) -> usize {
        (self.position.saturating_sub(HBLANK_CLOCKS) as usize).min(VISIBLE_CLOCKS)
    }

    /// The line-timing spine's boundary state: the colour-clock position and
    /// the HB-latch release the RHB decode chose this line.
    pub(crate) fn capture(&self) -> (u16, u16) {
        (self.position, self.hblank_release)
    }

    pub(crate) fn restore(&mut self, position: u16, hblank_release: u16) {
        self.position = position % CLOCKS_PER_LINE;
        self.hblank_release = hblank_release;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Step a fresh counter to an absolute position with no HMOVE extension.
    fn at(position: u16) -> HSyncCounter {
        let mut c = HSyncCounter::new();
        while c.position() != position {
            c.advance(false);
        }
        c
    }

    #[test]
    fn normal_line_blanks_then_renders_at_rhb_plus_four() {
        assert_eq!(at(63).beam(), Beam::Blank);
        assert_eq!(at(67).beam(), Beam::Blank);
        // RHB decodes at 64; the HB latch releases 4 CLK later at 68.
        assert_eq!(at(68).beam(), Beam::Pixel(0));
        assert_eq!(at(227).beam(), Beam::Pixel(159));
    }

    #[test]
    fn hmove_extends_the_blank_by_eight_as_a_comb() {
        // Arm the extension so the RHB decode at 64 chooses LRHB.
        let mut c = HSyncCounter::new();
        while c.position() != 68 {
            c.advance(true);
        }
        assert_eq!(c.beam(), Beam::Comb(0));
        while c.position() != 75 {
            c.advance(true);
        }
        assert_eq!(c.beam(), Beam::Comb(7));
        c.advance(true);
        // LRHB releases the latch 4 CLK after its decode at 72 → pixel at 76.
        assert_eq!(c.beam(), Beam::Pixel(8));
    }

    #[test]
    fn wraps_after_228_clocks() {
        let mut c = at(227);
        assert!(c.advance(false));
        assert_eq!(c.position(), 0);
    }
}
