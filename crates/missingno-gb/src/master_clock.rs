//! `ClockPhase` — the master-edge level type surfaced for the headless
//! debugger API. The live clock model lives in [`crate::clock`].

/// Master-clock signal level (the `clock::Edge` view exposed to the debugger).
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
