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
                let dot = if dot_fires { Some(self.dot_phase) } else { None };
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
mod tests {
    use super::*;

    /// `tcycle_schedule()` predicts exactly the dot edges the next two
    /// `advance(Running)` calls dispatch, from every reachable phase state at
    /// both ratios — the fused T-cycle stepping and the per-edge stepping are
    /// the same schedule.
    #[test]
    fn tcycle_schedule_matches_two_running_advances() {
        for divider in [CpuDivider::One, CpuDivider::Two] {
            let mut clock = MasterClock::new(divider);
            for _ in 0..64 {
                assert_eq!(clock.cpu_edge(), Edge::Rise, "T-cycles start on a rise");
                let schedule = clock.tcycle_schedule();
                let rise = clock.advance(CpuGate::Running);
                let fall = clock.advance(CpuGate::Running);
                assert_eq!(rise.cpu, Some(Edge::Rise));
                assert_eq!(fall.cpu, Some(Edge::Fall));
                let (expect_rise_dot, expect_fall_dot) = match schedule {
                    TcycleSchedule::FullDot => (Some(Edge::Rise), Some(Edge::Fall)),
                    TcycleSchedule::DotRiseOnRise => (Some(Edge::Rise), None),
                    TcycleSchedule::DotFallOnRise => (Some(Edge::Fall), None),
                };
                assert_eq!(rise.dot, expect_rise_dot, "{divider:?} {schedule:?}");
                assert_eq!(fall.dot, expect_fall_dot, "{divider:?} {schedule:?}");
            }
        }
    }

    /// At `÷2` the schedule alternates rise-carrying and fall-carrying
    /// T-cycles; `÷1` never leaves `FullDot`.
    #[test]
    fn tcycle_schedule_alternation() {
        let mut clock = MasterClock::new(CpuDivider::Two);
        for i in 0..64 {
            let expect = if i % 2 == 0 {
                TcycleSchedule::DotRiseOnRise
            } else {
                TcycleSchedule::DotFallOnRise
            };
            assert_eq!(clock.tcycle_schedule(), expect, "tcycle {i}");
            clock.advance(CpuGate::Running);
            clock.advance(CpuGate::Running);
        }
        let mut clock = MasterClock::new(CpuDivider::One);
        for _ in 0..64 {
            assert_eq!(clock.tcycle_schedule(), TcycleSchedule::FullDot);
            clock.advance(CpuGate::Running);
            clock.advance(CpuGate::Running);
        }
    }

    /// The resolver reproduces the inline `execute_phase` rule
    /// `is_mcycle_boundary || (cpu_steps_per_dot()==2 && tcycle==2)` across the
    /// full `(is_mcycle_boundary, tcycle)` domain at BOTH ratios — the (ii)
    /// phase placement comes from this and nothing else.
    #[test]
    fn mcycle_last_fall_matches_inline_rule_at_both_ratios() {
        for (divider, steps_per_dot) in [(CpuDivider::One, 1u8), (CpuDivider::Two, 2u8)] {
            for is_boundary in [false, true] {
                for tcycle in 0u8..=3 {
                    let inline = is_boundary || (steps_per_dot == 2 && tcycle == 2);
                    assert_eq!(
                        divider.mcycle_last_fall(is_boundary, tcycle),
                        inline,
                        "ratio {steps_per_dot}, boundary {is_boundary}, tcycle {tcycle}"
                    );
                }
            }
        }
    }

    /// The `advance()` truth table at `÷1`: every master edge carries a
    /// coincident CPU and dot edge, alternating Rise/Fall — the DMG
    /// `rise()`/`fall()` lockstep pair.
    #[test]
    fn advance_truth_table_at_one() {
        let mut clock = MasterClock::new(CpuDivider::One);
        let expected = [
            Tick {
                cpu: Some(Edge::Rise),
                dot: Some(Edge::Rise),
            },
            Tick {
                cpu: Some(Edge::Fall),
                dot: Some(Edge::Fall),
            },
            Tick {
                cpu: Some(Edge::Rise),
                dot: Some(Edge::Rise),
            },
            Tick {
                cpu: Some(Edge::Fall),
                dot: Some(Edge::Fall),
            },
        ];
        for (i, want) in expected.iter().enumerate() {
            // The ratio=1 substitution identity: every edge carries its dot.
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
                cpu: Some(Edge::Rise),
                dot: Some(Edge::Rise),
            },
            // bare second CPU edge of the dot — no dot edge
            Tick {
                cpu: Some(Edge::Fall),
                dot: None,
            },
            // next dot's first CPU edge carries the dot fall
            Tick {
                cpu: Some(Edge::Rise),
                dot: Some(Edge::Fall),
            },
            Tick {
                cpu: Some(Edge::Fall),
                dot: None,
            },
            Tick {
                cpu: Some(Edge::Rise),
                dot: Some(Edge::Rise),
            },
            Tick {
                cpu: Some(Edge::Fall),
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
    fn dot_edges_halve_at_two() {
        let mut clock = MasterClock::new(CpuDivider::Two);
        let dot_edges = (0..100)
            .filter(|_| clock.advance(CpuGate::Running).dot.is_some())
            .count();
        assert_eq!(dot_edges, 50);
    }

    /// A `Held` advance freezes the CPU phase and free-runs the dot domain: the
    /// CPU edge is `None` and unchanged across the hold, the dot edge fires every
    /// held edge, and `master_edge` increments so an anchor difference counts the
    /// held edges.
    #[test]
    fn held_advance_freezes_cpu_and_free_runs_dot() {
        let mut clock = MasterClock::new(CpuDivider::Two);
        // Advance one running edge so the CPU lands on a Fall — the phase the
        // freeze should preserve.
        clock.advance(CpuGate::Running);
        let frozen_cpu = clock.cpu_edge();
        assert_eq!(frozen_cpu, Edge::Fall);

        let anchor = clock.master_edge();
        let mut dots = Vec::new();
        for _ in 0..6 {
            let froze_on = clock.dot_phase();
            let tick = clock.advance(CpuGate::Held { froze_on });
            assert_eq!(tick.cpu, None, "CPU is frozen across the hold");
            dots.push(tick.dot.expect("a held edge always carries a dot edge"));
            // The CPU phase never moves during the hold.
            assert_eq!(clock.cpu_edge(), frozen_cpu);
        }
        // The dot domain alternated every held edge from its current phase.
        assert_eq!(
            dots,
            [
                Edge::Rise,
                Edge::Fall,
                Edge::Rise,
                Edge::Fall,
                Edge::Rise,
                Edge::Fall,
            ]
        );
        // master_edge - anchor counts the held edges exactly.
        assert_eq!(clock.master_edge() - anchor, 6);
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
        let cpu = tick.cpu.expect("running edge carries a CPU edge");
        OldDispatch {
            cpu,
            dot: tick.dot,
            // The dot phase toggles lazily here (after the dot's second CPU edge),
            // inverting the eager `ppu_phase == Low` the old code read — a pending
            // dot rise reads as the held phase being `Fall`. Only meaningful on a
            // fall edge (the fall arm).
            fall_arm_dot_work_extra: cpu == Edge::Fall
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
