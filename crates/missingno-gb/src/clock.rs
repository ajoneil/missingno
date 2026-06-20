//! The master-clock phase primitive.
//!
//! On hardware the CPU's CLK9 family and the PPU's ALET (dot) family are both
//! derived from the one continuous `ck1_ck2` master clock. A single `÷1`-or-`÷2`
//! divider cell sits between the master oscillator and the CPU clock: at `÷1`
//! (DMG, and CGB single speed) every CPU edge carries a dot edge, so the two
//! domains coincide; the CGB KEY1 switch flips it to `÷2`, splitting the CPU
//! edge stream from the dot edge stream.
//!
//! `MasterClock::advance` is the one place the divider ratio is read and the one
//! place the dispatch schedule is produced. At `÷1` it emits a `cpu` edge and a
//! coincident `dot` edge every master edge — exactly the `clock_phase ==
//! ppu_phase` lockstep the rest of the machine was built on.

/// One alternating edge of the continuous master clock (`ck1_ck2`). `Rise` and
/// `Fall` are the two edges of one cycle — not an ordering. A DFF captures on
/// one of them; that is their only meaning. This is `ClockPhase` renamed for the
/// phase layer: `Rise` ≡ `ClockPhase::Low` (master rise), `Fall` ≡
/// `ClockPhase::High` (master fall).
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
}

/// The CPU-clock gate handed to [`MasterClock::advance`]. `Running` clocks the
/// CPU normally. (The `Held` arm — the speed-switch blackout and the HDMA park —
/// is a later spike; this primitive only fields `Running`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuGate {
    Running,
}

/// What one master edge schedules. The step loop matches on this instead of
/// re-deriving the schedule from a speed flag. At `÷1`, `cpu` and `dot` are
/// always both `Some`/equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    /// The CPU's edge this master edge.
    pub cpu: Edge,
    /// The dot edge this CPU edge carries, or `None` on the bare second `÷2` CPU
    /// edge (no dot edge). At `÷1` always `Some` — the dot domain advances every
    /// CPU edge.
    pub dot: Option<Edge>,
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
    /// Within-dot CPU sub-edge: `0..ratio`. At `÷1` always `0` (cpu ≡ dot). At
    /// `÷2` it is `0` on the dot-carrying CPU edge, `1` on the bare second CPU
    /// edge — the explicit phase carry that replaces
    /// `ppu_advances = !double_speed || rising`.
    cpu_phase_in_dot: u8,
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
            cpu_phase_in_dot: 0,
        }
    }

    /// The CPU's current edge.
    pub fn cpu_edge(&self) -> Edge {
        self.cpu_phase
    }

    /// The dot edge this CPU edge carries, or `None` when this `÷2` CPU edge is
    /// the bare second T-cycle. At `÷1` always `Some`.
    pub fn dot_edge(&self) -> Option<Edge> {
        if self.cpu_phase_in_dot == 0 {
            Some(self.dot_phase)
        } else {
            None
        }
    }

    /// Did the dot domain advance this master edge? (`ppu_advances` today.)
    pub fn dot_step(&self) -> bool {
        self.cpu_phase_in_dot == 0
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
    /// where the SM83's first fetch begins on a CPU rising edge. Re-anchors the
    /// within-dot sub-edge to 0 so `cpu_phase_in_dot == 0 ⟺ cpu_phase == Rise`
    /// holds on the first CPU edge after the freeze (the dot fires on the resume
    /// rise, as `ppu_advances = rising` did).
    pub fn engage_on_rise(&mut self) {
        self.cpu_phase = Edge::Rise;
        self.cpu_phase_in_dot = 0;
    }

    /// Advance the dot domain one master edge without clocking the CPU — the
    /// frozen-CPU blackout edge. The CPU phase is untouched; the dot phase
    /// free-runs.
    pub fn advance_dot_only(&mut self) {
        self.dot_phase = self.dot_phase.flip();
    }

    /// Advance one master edge. THE single place the `÷2` ratio is read and the
    /// dispatch schedule is produced. Returns which domain edges fire.
    pub fn advance(&mut self, gate: CpuGate) -> Tick {
        match gate {
            CpuGate::Running => {
                self.master_edge += 1;
                let cpu = self.cpu_phase;
                let dot = self.dot_edge();
                self.cpu_phase = self.cpu_phase.flip();
                // The dot advances every CPU edge at ÷1, every other at ÷2.
                self.cpu_phase_in_dot =
                    (self.cpu_phase_in_dot + 1) % self.divider.cpu_edges_per_dot();
                if self.cpu_phase_in_dot == 0 {
                    self.dot_phase = self.dot_phase.flip();
                }
                Tick { cpu, dot }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `advance()` truth table at `÷1`: every master edge carries a
    /// coincident CPU and dot edge, alternating Rise/Fall — the DMG
    /// `rise()`/`fall()` lockstep pair.
    #[test]
    fn advance_truth_table_at_one() {
        let mut clock = MasterClock::new(CpuDivider::One);
        let expected = [
            Tick {
                cpu: Edge::Rise,
                dot: Some(Edge::Rise),
            },
            Tick {
                cpu: Edge::Fall,
                dot: Some(Edge::Fall),
            },
            Tick {
                cpu: Edge::Rise,
                dot: Some(Edge::Rise),
            },
            Tick {
                cpu: Edge::Fall,
                dot: Some(Edge::Fall),
            },
        ];
        for (i, want) in expected.iter().enumerate() {
            // The ratio=1 substitution identity: cpu_phase == dot_phase before
            // every edge.
            assert!(
                clock.dot_step(),
                "dot advances every edge at ÷1 (before edge {i})"
            );
            assert_eq!(clock.advance(CpuGate::Running), *want, "edge {i}");
        }
    }

    /// The `advance()` truth table at `÷2`: the dot edge lands on the first CPU
    /// edge of each dot and is absent on the bare second CPU edge, reproducing
    /// `ppu_advances = rising`.
    #[test]
    fn advance_truth_table_at_two() {
        let mut clock = MasterClock::new(CpuDivider::Two);
        let expected = [
            // dot rise on the dot's first CPU edge (a CPU rise)
            Tick {
                cpu: Edge::Rise,
                dot: Some(Edge::Rise),
            },
            // bare second CPU edge of the dot — no dot edge
            Tick {
                cpu: Edge::Fall,
                dot: None,
            },
            // next dot's first CPU edge carries the dot fall
            Tick {
                cpu: Edge::Rise,
                dot: Some(Edge::Fall),
            },
            Tick {
                cpu: Edge::Fall,
                dot: None,
            },
            Tick {
                cpu: Edge::Rise,
                dot: Some(Edge::Rise),
            },
            Tick {
                cpu: Edge::Fall,
                dot: None,
            },
        ];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(clock.advance(CpuGate::Running), *want, "edge {i}");
        }
    }

    /// At `÷2` the dot domain advances only on the dot-carrying CPU edge — half
    /// the master-edge rate — matching `ppu_advances = !double_speed || rising`.
    #[test]
    fn dot_step_halves_at_two() {
        let mut clock = MasterClock::new(CpuDivider::Two);
        let mut dot_steps = 0;
        for _ in 0..100 {
            if clock.dot_step() {
                dot_steps += 1;
            }
            clock.advance(CpuGate::Running);
        }
        assert_eq!(dot_steps, 50);
    }

    // ----------------------------------------------------------------------
    // Golden edge-trace: prove `advance` reproduces the pre-rewrite per-edge
    // dispatch byte-for-byte. The pre-rewrite `execute_phase` derived its
    // schedule inline from two `ClockPhase` fields (`clock_phase` = CPU edge,
    // `ppu_phase` = dot edge) and a `double_speed` flag; the model below is that
    // logic copied verbatim from the original source. The dispatch a master edge
    // produces is fully determined by `(clock_phase, ppu_phase, double_speed)`,
    // so a free-running comparison over thousands of edges is the complete
    // substitution proof (the per-edge `mcycle_boundary` comes from untouched CPU
    // state, so it is invariant under this change by construction).
    // ----------------------------------------------------------------------

    /// `ClockPhase` renamed locally so the oracle is a verbatim transcription.
    #[derive(Clone, Copy, PartialEq)]
    enum Old {
        Low,  // master rise
        High, // master fall
    }
    impl Old {
        fn next(self) -> Old {
            match self {
                Old::Low => Old::High,
                Old::High => Old::Low,
            }
        }
    }

    /// One pre-rewrite dispatch decision, the values `execute_phase` matched on.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct OldDispatch {
        cpu: Edge,
        dot: Option<Edge>,
        fall_arm_dot_work_extra: bool,
    }

    /// The pre-rewrite per-edge logic, transcribed from the original
    /// `execute_phase` body (the `clock_phase`/`ppu_phase`/`double_speed`
    /// derivation and the tail toggles), with no machine state attached.
    struct OldClock {
        clock_phase: Old,
        ppu_phase: Old,
    }

    impl OldClock {
        fn step(&mut self, double_speed: bool) -> OldDispatch {
            let rising = self.clock_phase == Old::Low;
            let ppu_advances = !double_speed || rising;
            let ppu = if ppu_advances {
                match self.ppu_phase {
                    Old::Low => Some(Edge::Rise),
                    Old::High => Some(Edge::Fall),
                }
            } else {
                None
            };
            let cpu = if rising { Edge::Rise } else { Edge::Fall };
            // The fall arm's extra dot_work term: `double_speed && ppu_phase==Low`,
            // read pre-toggle. Only meaningful on a fall edge (the fall arm).
            let fall_arm_dot_work_extra = !rising && double_speed && self.ppu_phase == Old::Low;

            self.clock_phase = self.clock_phase.next();
            if ppu_advances {
                self.ppu_phase = self.ppu_phase.next();
            }
            OldDispatch {
                cpu,
                dot: ppu,
                fall_arm_dot_work_extra,
            }
        }
    }

    /// The new clock's per-edge decision, in the same shape, including the
    /// fall-arm extra `dot_work` term the rewired `execute_phase` computes from
    /// the pre-advance dot phase.
    fn new_dispatch(clock: &mut MasterClock, double_speed: bool) -> OldDispatch {
        let dot_phase_before = clock.dot_phase();
        let tick = clock.advance(CpuGate::Running);
        OldDispatch {
            cpu: tick.cpu,
            dot: tick.dot,
            // The dot phase toggles lazily here (after the dot's second CPU edge),
            // inverting the eager `ppu_phase == Low` the old code read — a pending
            // dot rise reads as the held phase being `Fall`. Only meaningful on a
            // fall edge (the fall arm).
            fall_arm_dot_work_extra: tick.cpu == Edge::Fall
                && double_speed
                && dot_phase_before == Edge::Fall,
        }
    }

    /// DMG (ratio=1): the new clock's dispatch is byte-identical to the
    /// pre-rewrite logic over 10k master edges. This is the headline
    /// substitution proof — `cpu_phase == dot_phase` for all time, so every
    /// edge's `(cpu, dot, dot_work-extra)` matches.
    #[test]
    fn golden_edge_trace_dmg_ratio1_byte_identical() {
        let mut new = MasterClock::new(CpuDivider::One);
        let mut old = OldClock {
            clock_phase: Old::Low,
            ppu_phase: Old::Low,
        };
        for edge in 0..10_000 {
            let got = new_dispatch(&mut new, false);
            let want = old.step(false);
            assert_eq!(got, want, "ratio=1 edge {edge}");
            // The substitution identity itself: at ÷1 the CPU and dot edges
            // coincide on every edge.
            assert_eq!(got.cpu, got.dot.expect("dot fires every ÷1 edge"));
        }
    }

    /// CGB double speed (ratio=2): the new clock's dispatch also matches the
    /// pre-rewrite logic byte-for-byte over 10k edges, so the rewire did not
    /// disturb the double-speed dot-on-rise schedule either.
    #[test]
    fn golden_edge_trace_ratio2_byte_identical() {
        let mut new = MasterClock::new(CpuDivider::Two);
        let mut old = OldClock {
            clock_phase: Old::Low,
            ppu_phase: Old::Low,
        };
        for edge in 0..10_000 {
            let got = new_dispatch(&mut new, true);
            let want = old.step(true);
            assert_eq!(got, want, "ratio=2 edge {edge}");
        }
    }
}
