//! The sprite pre-processing scanner's lattice: when the counter steps
//! within a line, and what the status field presents around each step.

use std::ops::RangeInclusive;

use crate::Vdp;
use crate::port::{CYCLES_PER_LINE, run_cycle};
use crate::standard::ACTIVE_LINES;

/// The counter steps on the run schedule, this many memory cycles behind
/// each run start (steal-cadence pins it).
const SCAN_ALIGNMENT_CYCLES: usize = 6;
/// Entry 0 lands with the reset at the length-4 run; entries 1-7 burst one
/// per cycle behind it; entries 8-31 step three per run across the eight
/// regular runs that follow.
const SCAN_RESET_RUN: usize = 7;
const SCAN_BURST_OFFSETS: RangeInclusive<usize> = 2..=8;
const SCAN_STEP_RUNS: [usize; 8] = [8, 9, 10, 0, 1, 2, 3, 4];
const SCAN_STEP_OFFSET_CYCLES: [usize; 3] = [0, 4, 8];
const SCAN_RESET_CYCLE: usize = run_cycle(SCAN_RESET_RUN, SCAN_ALIGNMENT_CYCLES);
/// Burst steps land one XTAL later in their cycle than steady steps and
/// present the counter immediately; boundary texture starts at 7-to-8.
const SCAN_BURST_XTAL: u32 = 3;
/// Which memory cycles advance the scanner in the steady regime.
const SCAN_STEP_CYCLES: [bool; CYCLES_PER_LINE] = {
    let mut map = [false; CYCLES_PER_LINE];
    let mut run = 0;
    while run < SCAN_STEP_RUNS.len() {
        let mut offset = 0;
        while offset < SCAN_STEP_OFFSET_CYCLES.len() {
            map[run_cycle(
                SCAN_STEP_RUNS[run],
                SCAN_ALIGNMENT_CYCLES + SCAN_STEP_OFFSET_CYCLES[offset],
            )] = true;
            offset += 1;
        }
        run += 1;
    }
    map
};
/// The lattice instant within its memory cycle; silicon pins it only to a
/// 5-XTAL window.
const SCAN_STEP_XTAL: u32 = 2;
/// After an increment the field spends this long not presenting the
/// counter: bits 4/3 read 0 throughout; bits 2..0 hold the old value's low
/// bits at the first instant, all-ones through the middle, and the new
/// value's low bits at the last.
const SCAN_WINDOW_XTALS: u64 = 5;

/// Where a line's pre-processing ramp ends and why: the full 32-entry
/// walk, a terminator's own index, or the fifth match's.
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
}

impl Scanner {
    pub(crate) const POWER_ON: Self = Scanner {
        counter: 31,
        stop: ScanStop::FullWalk,
        stepped_at: 0,
        step_from: 31,
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
        // A counter line's lattice is the scan whose effects latch at the
        // boundary into the next line; the scanner serves display lines plus
        // the phantom pass, so the counter holds its stop through the border.
        let scanned_line =
            self.line < ACTIVE_LINES || self.line == self.standard.lines_per_frame() - 1;
        if !scanned_line {
            return;
        }
        if sub == SCAN_BURST_XTAL {
            if cycle >= SCAN_RESET_CYCLE
                && SCAN_BURST_OFFSETS.contains(&(cycle - SCAN_RESET_CYCLE))
                && self.scanner.counter < self.scanner.stop.index()
            {
                self.scanner.step_from = self.scanner.counter;
                self.scanner.counter += 1;
            }
        } else if cycle == SCAN_RESET_CYCLE {
            self.scanner.step_from = self.scanner.counter;
            self.scanner.counter = 0;
            self.scanner.stepped_at = self.xtal_total;
        } else if SCAN_STEP_CYCLES[cycle] && self.scanner.counter < self.scanner.stop.index() {
            self.scanner.step_from = self.scanner.counter;
            self.scanner.counter += 1;
            self.scanner.stepped_at = self.xtal_total;
        }
    }

    /// Status low five bits: the latched fifth-sprite index while 5S is
    /// set, otherwise the scanner's counter — live, except inside the
    /// boundary window around each step. The 7-to-8 step reads inverted
    /// mid-window; cause open.
    pub(crate) fn scanned_field(&self) -> u8 {
        if self.status.fifth_sprite {
            return self.status.sprite_field & 0x1F;
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
