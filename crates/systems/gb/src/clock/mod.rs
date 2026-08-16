//! The master-clock phase primitive: the `÷1`-or-`÷2` KEY1 divider between the
//! `ck1_ck2` master clock and the CPU CLK9 family is read in one place here, so
//! `advance` is the sole producer of the CPU:dot dispatch schedule.

/// One alternating edge of the continuous master clock (`ck1_ck2`). `Rise` and
/// `Fall` are the two edges of one cycle — not an ordering. A DFF captures on
/// one of them; that is their only meaning. `Rise` is the master rise (the
/// even/`Low` level), `Fall` the master fall (the odd/`High` level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Rise,
    Fall,
}

impl Edge {
    pub fn flip(self) -> Edge {
        match self {
            Edge::Rise => Edge::Fall,
            Edge::Fall => Edge::Rise,
        }
    }
}

/// The `÷1`-or-`÷2` divider cell — the one timing circuit the CGB adds to the
/// DMG die. DMG is hard-wired `One`; KEY1 is the only thing that selects `Two`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuDivider {
    One,
    Two,
}

impl CpuDivider {
    /// CPU edges per dot edge — the CPU:dot ratio as a single `u8`.
    pub fn cpu_edges_per_dot(self) -> u8 {
        match self {
            CpuDivider::One => 1,
            CpuDivider::Two => 2,
        }
    }

    /// Resolve [`CaptureEdge::MCycleLastFall`] to a concrete fall under this
    /// ratio: is this PPU fall the last one of the writing M-cycle? At `÷1`
    /// every T-cycle carries a PPU fall, so the M-cycle's last fall is its T3
    /// boundary fall. At `÷2` PPU falls land on alternate T-cycles, so when the
    /// T3 boundary edge carries no PPU fall the M's last fall is T2's. The
    /// (ii) clock-domain phase the CGB crossing rides arrives entirely from
    /// *which* edge this resolves to — never folded into a `delayed_falls`
    /// count.
    ///
    /// [`CaptureEdge::MCycleLastFall`]: crate::ppu::CaptureEdge::MCycleLastFall
    pub fn mcycle_last_fall(self, is_mcycle_boundary: bool, tcycle: u8) -> bool {
        is_mcycle_boundary || (self == CpuDivider::Two && tcycle == 2)
    }
}

/// The CPU-clock gate handed to [`MasterClock::advance`]. `Running` clocks the
/// CPU normally; `Held` freezes the CPU CLK9 family while the dot domain keeps
/// free-running — the speed-switch blackout (and, in a later step, the HDMA
/// park). The gate is NOT a bool: `Held` records the dot edge the freeze landed
/// on, so the distinguishing DS-HDMA phase survives the unification (the dot
/// phase a bit-identical straddle differs by).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuGate {
    Running,
    /// CPU CLK9 frozen; the dot domain free-runs. `froze_on` is the dot edge the
    /// most recent held advance landed on — recorded from day one so the phase
    /// signal exists for the deferred HDMA fall-counter re-expression.
    Held {
        froze_on: Edge,
    },
}

/// What one master edge schedules. The step loop matches on this instead of
/// re-deriving the schedule from a speed flag. At `÷1`, `cpu` and `dot` are
/// always both `Some`/equal. `cpu` is `None` only on a `Held` edge (CPU frozen);
/// `dot` is `None` only on the bare second `÷2` running CPU edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    /// The CPU's edge this master edge, or `None` while the CPU clock is `Held`.
    pub cpu: Option<Edge>,
    /// The dot edge this master edge carries, or `None` on the bare second `÷2`
    /// running CPU edge (no dot edge). At `÷1` always `Some` — the dot domain
    /// advances every CPU edge — and on every `Held` edge the dot domain steps.
    pub dot: Option<Edge>,
}

/// The dot edges one CPU T-cycle carries. At `÷1` a T-cycle spans exactly one
/// dot (rise on the CPU rise, fall on the CPU fall). At `÷2` the CPU runs two
/// T-cycles per dot, so each T-cycle carries exactly one dot edge, on its
/// rise — the dot's rise and fall alternate across consecutive T-cycles.
/// No other combination is producible by the divider, so no other is
/// representable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcycleSchedule {
    /// `÷1`: dot rise on the CPU rise, dot fall on the CPU fall.
    FullDot,
    /// `÷2`, first T-cycle of the dot: dot rise on the CPU rise, bare fall.
    DotRiseOnRise,
    /// `÷2`, second T-cycle: dot fall on the CPU rise, bare fall.
    DotFallOnRise,
}

/// The master-clock phase layer: the free-running master-edge count, the divider
/// cell, and the CPU/dot phases it produces. Replaces the loose `clock_phase`
/// (CPU edge) and `ppu_phase` (dot edge) fields with one object that owns the
/// dispatch.
#[derive(Clone, Copy, Debug)]
pub struct MasterClock {
    /// Free-running master-edge counter. Monotone; one toggle per master
    /// half-cycle.
    master_edge: u64,
    /// The `÷1`-or-`÷2` cell. DMG: `One`.
    divider: CpuDivider,
    /// CPU CLK9-family phase — today's `clock_phase`.
    cpu_phase: Edge,
    /// Dot/ALET-family phase — today's `ppu_phase`. Free-running, untouched by
    /// the divider ratio.
    dot_phase: Edge,
}

impl MasterClock {
    /// A clock starting on the master rise (`Low`/even), so the ratio=1 parity
    /// identity `cpu_phase == dot_phase` holds from edge 0.
    pub fn new(divider: CpuDivider) -> MasterClock {
        MasterClock {
            master_edge: 0,
            divider,
            cpu_phase: Edge::Rise,
            dot_phase: Edge::Rise,
        }
    }

    /// The CPU's current edge.
    pub fn cpu_edge(&self) -> Edge {
        self.cpu_phase
    }

    /// The dot domain's own current edge (independent of whether this CPU edge
    /// carries it). The blackout reads this while the CPU is frozen.
    pub fn dot_phase(&self) -> Edge {
        self.dot_phase
    }

    pub fn divider(&self) -> CpuDivider {
        self.divider
    }

    pub fn master_edge(&self) -> u64 {
        self.master_edge
    }

    /// Switch the divider ratio. The CGB KEY1 path flips this; DMG never calls
    /// it.
    pub fn set_divider(&mut self, divider: CpuDivider) {
        self.divider = divider;
    }

    /// Force the CPU phase to the master rise — the blackout-resume re-engage,
    /// where the SM83's first fetch begins on a CPU rising edge. Every freeze
    /// exits through here, which is what keeps the `÷2` dot placement a pure
    /// function of `cpu_phase` (the dot fires on rises, flips after falls).
    pub fn engage_on_rise(&mut self) {
        self.cpu_phase = Edge::Rise;
    }

    /// The dot edges the NEXT full CPU T-cycle (rise + fall) will carry, and
    /// where. At `÷1` every T-cycle is [`TcycleSchedule::FullDot`]; at `÷2`
    /// the dot's rise and fall land on alternate T-cycles' rises. Reading it
    /// does not advance the clock — pair with two [`MasterClock::advance`]
    /// calls (or the fused stepping that replaces them).
    pub fn tcycle_schedule(&self) -> TcycleSchedule {
        match self.divider {
            CpuDivider::One => TcycleSchedule::FullDot,
            CpuDivider::Two => match self.dot_phase {
                Edge::Rise => TcycleSchedule::DotRiseOnRise,
                Edge::Fall => TcycleSchedule::DotFallOnRise,
            },
        }
    }

    /// Advance one master edge. THE single place the `÷2` ratio is read, the
    /// dispatch schedule is produced, and the CPU clock can be frozen. The
    /// running machine passes `Running`; the speed-switch blackout passes `Held`,
    /// which freezes the CPU phase and free-runs the dot domain. Returns which
    /// domain edges fire.
    pub fn advance(&mut self, gate: CpuGate) -> Tick {
        self.master_edge += 1;
        match gate {
            CpuGate::Running => {
                let cpu = self.cpu_phase;
                // The dot fires on every ÷1 edge; at ÷2 only on the CPU rises
                // (rise-alignment holds because every freeze exits via
                // `engage_on_rise`).
                let dot_fires = self.divider == CpuDivider::One || cpu == Edge::Rise;
                let dot = if dot_fires {
                    Some(self.dot_phase)
                } else {
                    None
                };
                self.cpu_phase = self.cpu_phase.flip();
                // The fired dot edge is spent once the dot's CPU edges are: on
                // every ÷1 edge, and after the ÷2 bare fall.
                if self.divider == CpuDivider::One || cpu == Edge::Fall {
                    self.dot_phase = self.dot_phase.flip();
                }
                Tick {
                    cpu: Some(cpu),
                    dot,
                }
            }
            CpuGate::Held { .. } => {
                // CPU CLK9 gated: `cpu_phase` / `cpu_phase_in_dot` frozen, the dot
                // domain free-runs (VID_RST releases the PPU dividers; they count
                // from zero). The dot edge fired is this edge's pre-flip phase.
                let dot = self.dot_phase;
                self.dot_phase = self.dot_phase.flip();
                Tick {
                    cpu: None,
                    dot: Some(dot),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
