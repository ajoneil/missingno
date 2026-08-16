//! The sprite pre-processing scanner's lattice: when the counter steps
//! within a line, and what the status field presents around each step.

use crate::Vdp;
use crate::port::CYCLES_PER_LINE;
use crate::standard::ACTIVE_LINES;

/// The lattice is locked to the run schedule: entry 0 lands with the
/// counter reset at the length-4 run's start, entries 1-7 burst one per
/// memory cycle behind it, and entries 8-31 step three per 16-cycle run
/// period across the eight regular length-1 runs — 9 entries per 48
/// cycles exactly.
const SCAN_RESET_CYCLE: usize = 110;
const SCAN_BURST_CYCLES: std::ops::RangeInclusive<usize> = 112..=118;
/// Burst steps land one XTAL later in their cycle than steady steps and
/// present the counter immediately; boundary texture starts at 7-to-8.
const SCAN_BURST_XTAL: u32 = 3;
const SCAN_STEP_RUNS: [usize; 8] = [123, 139, 155, 0, 16, 32, 48, 64];
const SCAN_STEP_OFFSET_CYCLES: [usize; 3] = [0, 4, 8];
/// Which memory cycles advance the scanner in the steady regime.
const SCAN_STEP_CYCLES: [bool; CYCLES_PER_LINE] = {
    let mut map = [false; CYCLES_PER_LINE];
    let mut run = 0;
    while run < SCAN_STEP_RUNS.len() {
        let mut offset = 0;
        while offset < SCAN_STEP_OFFSET_CYCLES.len() {
            map[SCAN_STEP_RUNS[run] + SCAN_STEP_OFFSET_CYCLES[offset]] = true;
            offset += 1;
        }
        run += 1;
    }
    map
};
/// The lattice instant within its memory cycle; silicon pins it only to a
/// 5-XTAL window.
const SCAN_STEP_XTAL: u32 = 2;
/// The fifth-match hold releases here — between the counter's 13th and
/// 14th steps; the sub-cycle instant is free.
const SCAN_HOLD_RELEASE_CYCLE: usize = 153;
/// After an increment the field spends this long not presenting the
/// counter: bits 4/3 read 0 throughout; bits 2..0 hold the old value's low
/// bits at the first instant, all-ones through the middle, and the new
/// value's low bits at the last.
const SCAN_WINDOW_XTALS: u64 = 5;

/// Where a line's pre-processing ramp ends and why: the full 32-entry
/// walk, a terminator's own index, or the fifth match's — only the
/// fifth-match halt arms the field hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanStop {
    FullWalk,
    Terminator(u8),
    FifthMatch(u8),
}

impl ScanStop {
    fn index(self) -> u8 {
        match self {
            ScanStop::FullWalk => 31,
            ScanStop::Terminator(index) | ScanStop::FifthMatch(index) => index,
        }
    }
}

/// The scanner's live progress: last SAT entry handled, where this line's
/// ramp ends, and the latest step (instant + the value it replaced) for
/// the boundary window.
pub(crate) struct Scanner {
    pub(crate) counter: u8,
    pub(crate) stop: ScanStop,
    pub(crate) stepped_at: u64,
    pub(crate) step_from: u8,
    /// The fifth-match event arms a hold on the presented field; it
    /// survives the next reset and drops at the release cycle of the
    /// first scan with no event.
    pub(crate) field_hold: Option<u8>,
    pub(crate) fifth_match_this_scan: bool,
}

impl Scanner {
    pub(crate) const POWER_ON: Self = Scanner {
        counter: 31,
        stop: ScanStop::FullWalk,
        stepped_at: 0,
        step_from: 31,
        field_hold: None,
        fifth_match_this_scan: false,
    };
}

impl Vdp {
    /// Advance the pre-processing scanner at its lattice instants. M1 gates
    /// rendering, never the scanner, so only blanking stops it.
    pub(crate) fn scan_lattice(&mut self) {
        if !self.display_enabled() {
            return;
        }
        let sub = self.xtal_in_line % 4;
        if sub != SCAN_STEP_XTAL && sub != SCAN_BURST_XTAL {
            return;
        }
        let cycle = (self.xtal_in_line / 4) as usize;
        // From the reset on, a line's lattice tail belongs to the NEXT
        // line's scan; the scanner serves display lines plus the phantom
        // pass, so the counter holds its stop through the border.
        let scanned_line = if cycle >= SCAN_RESET_CYCLE {
            self.line < ACTIVE_LINES || self.line == self.standard.lines_per_frame() - 1
        } else {
            self.line <= ACTIVE_LINES
        };
        if !scanned_line {
            return;
        }
        if sub == SCAN_BURST_XTAL {
            if SCAN_BURST_CYCLES.contains(&cycle)
                && self.scanner.counter < self.scanner.stop.index()
            {
                self.scanner.step_from = self.scanner.counter;
                self.scanner.counter += 1;
                self.arm_hold_at_stop();
            }
        } else if cycle == SCAN_RESET_CYCLE {
            self.scanner.step_from = self.scanner.counter;
            self.scanner.counter = 0;
            self.scanner.stepped_at = self.xtal_total;
            self.scanner.fifth_match_this_scan = false;
        } else if cycle == SCAN_HOLD_RELEASE_CYCLE {
            if !self.scanner.fifth_match_this_scan {
                self.scanner.field_hold = None;
            }
        } else if SCAN_STEP_CYCLES[cycle] && self.scanner.counter < self.scanner.stop.index() {
            self.scanner.step_from = self.scanner.counter;
            self.scanner.counter += 1;
            self.scanner.stepped_at = self.xtal_total;
            self.arm_hold_at_stop();
        }
    }

    fn arm_hold_at_stop(&mut self) {
        if let ScanStop::FifthMatch(index) = self.scanner.stop
            && self.scanner.counter == index
        {
            self.scanner.field_hold = Some(index);
            self.scanner.fifth_match_this_scan = true;
        }
    }

    /// Status low five bits: the latched fifth-sprite index while 5S is
    /// set, the armed fifth-match hold next, otherwise the scanner's
    /// counter — live, except inside the boundary window around each step.
    /// The 7-to-8 step reads inverted mid-window; cause open.
    pub(crate) fn scanned_field(&self) -> u8 {
        if self.status.fifth_sprite {
            return self.status.sprite_field & 0x1F;
        }
        if let Some(held) = self.scanner.field_hold {
            return held;
        }
        let elapsed = self.xtal_total - self.scanner.stepped_at;
        if elapsed >= SCAN_WINDOW_XTALS {
            return self.scanner.counter;
        }
        let carry_step = self.scanner.step_from == 7 && self.scanner.counter == 8;
        if elapsed == 0 {
            self.scanner.step_from & 7
        } else if elapsed == SCAN_WINDOW_XTALS - 1 {
            if carry_step {
                self.scanner.counter
            } else {
                self.scanner.counter & 7
            }
        } else if carry_step {
            0b11000
        } else {
            7
        }
    }
}
