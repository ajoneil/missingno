//! The DMG master clock: the 4.19 MHz signal that drives every
//! subsystem. One full master-clock cycle (Low → High → Low) is one
//! T-cycle; four T-cycles make one M-cycle (the CPU's machine cycle).
//!
//! The executor alternates `rise()` (Low→High edge) and `fall()`
//! (High→Low edge); together they advance one T-cycle.

/// Master clock signal level. Alternates High → Low → High each
/// half-T-cycle. `rise()` fires at the Low→High edge; `fall()` fires
/// at the High→Low edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockPhase {
    High,
    Low,
}

impl ClockPhase {
    pub fn next(self) -> ClockPhase {
        match self {
            ClockPhase::High => ClockPhase::Low,
            ClockPhase::Low => ClockPhase::High,
        }
    }
}

impl From<crate::clock::Edge> for ClockPhase {
    fn from(edge: crate::clock::Edge) -> ClockPhase {
        match edge {
            crate::clock::Edge::Rise => ClockPhase::Low,
            crate::clock::Edge::Fall => ClockPhase::High,
        }
    }
}
