//! The chip-crate stepping contract: a CPU driven by its board one chip
//! T-state at a time.
//!
//! Every CPU in the workspace advances at its own silicon's T — the 6502's
//! bus cycle, the Z80's T-state, the SM83's T-cycle — and the board owns
//! time: it interleaves the other chips between ticks, so a bus access
//! issued inside a tick lands against a world that has advanced to that
//! instant. Cycle counts are call counts; the only boundary a chip reports
//! is the instruction boundary the debugger and save states step to.
//!
//! The bus itself stays a per-chip trait (a 6502 has `read`/`write`, a Z80
//! adds ports) — this vocabulary shares the *stepping* contract, not the
//! pinout. Execution decode likewise stays per-crate, per `isa`'s charter.

/// A CPU stepped by its board, one T of its own clock per call, generic
/// over the board's bus `B`.
pub trait ClockedCpu<B> {
    /// Advance one chip T-state, issuing any bus access that T carries.
    fn tick(&mut self, bus: &mut B);

    /// At the boundary between instructions — the debugger's stepping
    /// unit and the only place save states restore to.
    fn at_instruction_boundary(&self) -> bool;

    /// Fetch-stopped for good (a 6502 JAM); boards use this to end
    /// run-to-boundary loops that would otherwise never return.
    fn jammed(&self) -> bool {
        false
    }
}

/// Carried-fraction division of one clock grid into another: a client
/// running at `numerator/denominator` of the master is owed `advance()`
/// ticks as the master moves, the sub-tick residue carried exactly.
#[derive(Clone)]
pub struct ClockRatio {
    numerator: u64,
    denominator: u64,
    remainder: u64,
}

impl ClockRatio {
    pub fn new(numerator: u64, denominator: u64) -> Self {
        assert!(numerator > 0 && denominator > 0);
        ClockRatio {
            numerator,
            denominator,
            remainder: 0,
        }
    }

    /// Client ticks due after the master advances `master_ticks`.
    pub fn advance(&mut self, master_ticks: u64) -> u64 {
        let due = self.remainder + master_ticks * self.numerator;
        self.remainder = due % self.denominator;
        due / self.denominator
    }

    /// Master ticks until the `n`th client tick from now, for callers that
    /// advance a whole span at once and need to know where inside it the
    /// client ticks land.
    pub fn ticks_until(&self, n: u64) -> u64 {
        assert!(n > 0);
        (n * self.denominator - self.remainder).div_ceil(self.numerator)
    }
}

#[cfg(test)]
mod tests {
    use super::ClockRatio;

    #[test]
    fn three_dots_per_two_tstates() {
        let mut ratio = ClockRatio::new(3, 2);
        assert_eq!(ratio.advance(1), 1);
        assert_eq!(ratio.advance(1), 2);
        let more: u64 = (0..100).map(|_| ratio.advance(7)).sum();
        assert_eq!(3 + more, 702 * 3 / 2);
    }

    #[test]
    fn lookahead_matches_stepping() {
        let mut ratio = ClockRatio::new(44_100, 4_194_304);
        for _ in 0..2_000 {
            let ahead = ratio.ticks_until(1);
            for _ in 1..ahead {
                assert_eq!(ratio.advance(1), 0);
            }
            assert_eq!(ratio.advance(1), 1);
        }
    }

    #[test]
    fn lookahead_spans_many_client_ticks() {
        let ratio = ClockRatio::new(44_100, 4_194_304);
        for n in 1..1_000u64 {
            let mut stepped = ratio.clone();
            let at = ratio.ticks_until(n);
            assert_eq!(stepped.advance(at), n);
            let mut short = ratio.clone();
            assert_eq!(short.advance(at - 1), n - 1);
        }
    }

    #[test]
    fn residue_never_lost() {
        let mut ratio = ClockRatio::new(3, 2);
        let mut due = 0;
        for _ in 0..1000 {
            due += ratio.advance(1);
        }
        assert_eq!(due, 1500);
    }
}
