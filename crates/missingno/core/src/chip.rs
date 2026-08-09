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
