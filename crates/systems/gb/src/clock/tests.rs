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

/// The resolver matches the rule `is_mcycle_boundary || (cpu_edges_per_dot() ==
/// 2 && tcycle == 2)` across the full `(is_mcycle_boundary, tcycle)` domain at
/// BOTH ratios — `ppu_fall_edge` reads it for the (ii) crossing placement.
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

// Oracle model of the per-edge dispatch; the tests below assert `advance` matches it exactly.

/// The oracle's master-clock phase.
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

/// One dispatch decision — the values `tcycle_rise`/`tcycle_fall` match on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct OldDispatch {
    cpu: Edge,
    dot: Option<Edge>,
    fall_arm_dot_work_extra: bool,
}

/// The per-edge dispatch derived independently from `clock_phase`, `ppu_phase`
/// and `double_speed`, with no machine state attached.
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

/// The clock's per-edge decision, in the same shape, including the fall-arm
/// extra `dot_work` term `tcycle_fall` computes from the pre-advance dot phase.
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

/// DMG (ratio=1): the clock's dispatch is byte-identical to the oracle over 10k
/// master edges — `cpu_phase == dot_phase` for all time, so every edge's
/// `(cpu, dot, dot_work-extra)` matches.
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

/// CGB double speed (ratio=2): the clock's dispatch also matches the oracle
/// byte-for-byte over 10k edges, covering the double-speed dot-on-rise
/// schedule.
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
