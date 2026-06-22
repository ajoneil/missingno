//! STAT register state and LALU edge-detection.
//! LYC pipeline: PALY (LY==LYC) → ROPO (TALU-rising DFF) → RUPO (transparent NOR latch).

use bitflags::bitflags;

bitflags! {
    #[derive(Copy, Clone, PartialEq, Eq)]
    pub struct InterruptFlags: u8 {
        const DUMMY                = 0b10000000;
        const CURRENT_LINE_COMPARE = 0b01000000;
        const OAM_SCAN             = 0b00100000;
        const VERTICAL_BLANK       = 0b00010000;
        const HORIZONTAL_BLANK     = 0b00001000;
    }
}

/// The synchroniser between the FF41/FF45 register cells and the STAT-IRQ
/// block: DFF copies of the enables and LYC cells, captured on the CPU-clock
/// M-cycle boundary. The cells stay CPU-visible at write time; only the IRQ
/// block reads these copies. Lives on the model — the CGB owns the real cells
/// ([`SyncedStatCells`]), the DMG a ZST `()`, since the DMG feeds the legs and
/// comparator combinationally off the cells and never crosses the domain.
pub trait StatShadow {
    fn synced_enables(&self) -> InterruptFlags;
    fn set_synced_enables(&mut self, value: InterruptFlags);
    /// PALY's LYC input. DMG reads the cell directly (no synchroniser), so its
    /// ZST forwards the `cell` argument; the CGB returns its captured copy.
    fn synced_lyc(&self, cell: u8) -> u8;
    fn set_synced_lyc(&mut self, value: u8);
}

/// The CGB FF41/FF45 synchroniser DFFs.
#[derive(Default)]
pub struct SyncedStatCells {
    enables: InterruptFlags,
    lyc: u8,
}

impl StatShadow for SyncedStatCells {
    fn synced_enables(&self) -> InterruptFlags {
        self.enables
    }
    fn set_synced_enables(&mut self, value: InterruptFlags) {
        self.enables = value;
    }
    fn synced_lyc(&self, _cell: u8) -> u8 {
        self.lyc
    }
    fn set_synced_lyc(&mut self, value: u8) {
        self.lyc = value;
    }
}

impl StatShadow for () {
    fn synced_enables(&self) -> InterruptFlags {
        InterruptFlags::empty()
    }
    fn set_synced_enables(&mut self, _value: InterruptFlags) {}
    fn synced_lyc(&self, cell: u8) -> u8 {
        cell
    }
    fn set_synced_lyc(&mut self, _value: u8) {}
}

impl Default for InterruptFlags {
    fn default() -> Self {
        Self::empty()
    }
}

pub struct StatInterrupt {
    /// LYC register ($FF45).
    pub(in crate::ppu) lyc: u8,
    /// PALY combinational comparator; recomputed at TALU fall and on LYC writes.
    pub(in crate::ppu) comparison_pending: bool,
    /// ROPO DFF — latched LY==LYC. Reset only by SYS_RST (not VID_RST). Drives STAT bit 2 via transparent RUPO.
    pub(in crate::ppu) comparison_latched: bool,
    /// FF41 bits 3-6 enables + DUMMY pull-up on bit 7.
    pub(in crate::ppu) enables: InterruptFlags,
    /// LALU per-leg previous values — one bit per SUKO source AND-term.
    pub(in crate::ppu) legs_was_high: InterruptFlags,
    /// Condition-input values at the previous evaluation (per-source,
    /// pre-enable) — the waveform scan's transition baseline on both cores.
    pub(in crate::ppu) conditions_was: InterruptFlags,
}

impl StatInterrupt {
    /// SYS_RST state (LCD off, everything cleared; ROPO resets high).
    pub(in crate::ppu) fn power_on() -> Self {
        Self {
            lyc: 0,
            comparison_pending: false,
            comparison_latched: true,
            enables: InterruptFlags::empty(),
            legs_was_high: InterruptFlags::empty(),
            conditions_was: InterruptFlags::empty(),
        }
    }

    pub(in crate::ppu) fn post_boot() -> Self {
        Self {
            lyc: 0,
            comparison_pending: true,
            comparison_latched: true,
            enables: InterruptFlags::DUMMY,
            legs_was_high: InterruptFlags::empty(),
            conditions_was: InterruptFlags::empty(),
        }
    }

    /// Post-boot comparator settled against the live LY (LYC=0).
    pub(in crate::ppu) fn post_boot_at_line(ly: u8) -> Self {
        let matches = ly == 0;
        Self {
            comparison_pending: matches,
            comparison_latched: matches,
            ..Self::post_boot()
        }
    }

    /// PALY recompute: `pending = (ly == lyc)`. PALY's LYC input is the IRQ
    /// domain's view — the cell on DMG, the synchroniser copy on CGB.
    pub(in crate::ppu) fn update_comparison(&mut self, ly: u8, shadow: &impl StatShadow) {
        self.comparison_pending = ly == shadow.synced_lyc(self.lyc);
    }

    /// ROPO captures comparison_pending on TALU rising.
    pub(in crate::ppu) fn latch_comparison(&mut self) {
        self.comparison_latched = self.comparison_pending;
    }

    /// ROPO.Q — drives STAT bit 2 via the transparent RUPO latch, and also feeds the
    /// LYC-match arm of the STAT-interrupt edge detector.
    pub(in crate::ppu) fn ly_eq_lyc(&self) -> bool {
        self.comparison_latched
    }

    pub(in crate::ppu) fn lyc(&self) -> u8 {
        self.lyc
    }

    pub(in crate::ppu) fn enables(&self) -> InterruptFlags {
        self.enables
    }

    pub(in crate::ppu) fn legs_was_high(&self) -> InterruptFlags {
        self.legs_was_high
    }

    /// Used by the STAT write glitch path to install the transient all-bits-high state.
    pub(in crate::ppu) fn set_enables(&mut self, flags: InterruptFlags) {
        self.enables = flags;
    }

    pub(in crate::ppu) fn write_lyc(&mut self, value: u8, ly: u8, shadow: &mut impl StatShadow) {
        self.lyc = value;
        shadow.set_synced_lyc(value);
        self.update_comparison(ly, shadow);
    }

    pub(in crate::ppu) fn write_stat_bits(&mut self, value: u8, shadow: &mut impl StatShadow) {
        self.enables = InterruptFlags::from_bits_truncate(value);
        shadow.set_synced_enables(self.enables);
    }

    /// CGB FF45 write: the cell updates now (readback is write-time); the IRQ
    /// domain sees it at the LYC crossing's next resolved capture edge.
    pub(in crate::ppu) fn write_lyc_cell(&mut self, value: u8) {
        self.lyc = value;
    }

    /// The FF45→IRQ-block crossing: copy the LYC cell into the synchroniser DFF
    /// and recompute PALY against it. Fired on the LYC crossing's resolved
    /// capture edge; the synced value feeds the comparator for the next TALU↑.
    pub(in crate::ppu) fn capture_synced_lyc(&mut self, ly: u8, shadow: &mut impl StatShadow) {
        shadow.set_synced_lyc(self.lyc);
        self.update_comparison(ly, shadow);
    }

    /// CGB FF41 write: cell-only, as `write_lyc_cell`.
    pub(in crate::ppu) fn write_stat_bits_cell(&mut self, value: u8) {
        self.enables = InterruptFlags::from_bits_truncate(value);
    }

    /// DMG SUKO evaluation against the live enables (no synchroniser; the
    /// cells feed the legs combinationally, so no register edge can occur
    /// inside a TALU evaluation).
    pub(in crate::ppu) fn eval_conditions(
        &mut self,
        conditions: InterruptFlags,
        talu_rising: bool,
    ) -> bool {
        let enables = self.enables;
        self.eval_core(
            conditions,
            talu_rising,
            enables,
            enables,
            InterruptFlags::empty(),
        )
    }

    /// CGB SUKO evaluation against the synchronised register file. At an
    /// M-boundary fall (`boundary_capture`) the FF41 synchroniser DFF captures
    /// the enables cell first; the resulting register-path edges race this
    /// fall's condition edges within the SUKO waveform. The FF45 crossing is
    /// captured separately on its own resolved edge (`capture_synced_lyc`); its
    /// synced value feeds PALY for the next TALU↑.
    pub(in crate::ppu) fn eval_synced(
        &mut self,
        conditions: InterruptFlags,
        talu_rising: bool,
        boundary_capture: bool,
        shadow: &mut impl StatShadow,
    ) -> bool {
        let enables_before = shadow.synced_enables();
        let register_edges = if boundary_capture {
            let delta = enables_before ^ self.enables;
            shadow.set_synced_enables(self.enables);
            delta & !InterruptFlags::DUMMY
        } else {
            InterruptFlags::empty()
        };
        let enables_after = shadow.synced_enables();
        self.eval_core(
            conditions,
            talu_rising,
            enables_before,
            enables_after,
            register_edges,
        )
    }

    fn eval_core(
        &mut self,
        conditions: InterruptFlags,
        talu_rising: bool,
        enables_before: InterruptFlags,
        enables_after: InterruptFlags,
        register_edges: InterruptFlags,
    ) -> bool {
        let conditions_before = self.conditions_was;
        self.conditions_was = conditions;

        // Off-TALU evaluations use the boolean rising-edge rule; so does any
        // evaluation with no input transition — a flat SUKO waveform cannot
        // produce a capturable 0→1.
        if register_edges.is_empty() && (!talu_rising || conditions == conditions_before) {
            let legs = conditions & enables_after;
            return self.detect_suko_edge(legs);
        }
        self.detect_suko_waveform(
            conditions_before,
            conditions,
            enables_before,
            enables_after,
            register_edges,
        )
    }

    /// SUKO waveform scan for a TALU↑ or register-capture evaluation. Each
    /// leg is the AND of its enable (transitioning at the shared
    /// register-path arrival, CGB only) and its condition (transitioning at
    /// the per-leg constant); SUKO is the OR of the legs. The LALU dffsr
    /// captures a 0→1 only when the preceding low interval and the following
    /// high interval both meet the capture threshold.
    fn detect_suko_waveform(
        &mut self,
        conditions_before: InterruptFlags,
        conditions_after: InterruptFlags,
        enables_before: InterruptFlags,
        enables_after: InterruptFlags,
        register_edges: InterruptFlags,
    ) -> bool {
        const STEADY_PS: i32 = i32::MAX / 2;
        self.legs_was_high = conditions_after & enables_after;
        let rising_conditions = (conditions_before ^ conditions_after) & conditions_after;

        // Per-leg breakpoints: at most one enable edge and one condition edge.
        let mut times = [0i32; 9];
        let mut time_count = 1; // t = 0 (initial state)
        for leg in (conditions_before ^ conditions_after).iter() {
            let arrival = arrival(leg);
            times[time_count] = if conditions_after.contains(leg) {
                arrival.rising_ps as i32
            } else {
                arrival.falling_ps as i32
            };
            time_count += 1;
        }
        for leg in register_edges.iter() {
            times[time_count] = register_arrival(leg, rising_conditions) as i32;
            time_count += 1;
        }
        let times = &mut times[..time_count];
        times.sort_unstable();

        let level_at = |t: i32| -> bool {
            for leg in InterruptFlags::all().iter() {
                if leg == InterruptFlags::DUMMY {
                    continue;
                }
                let cond_changed = (conditions_before ^ conditions_after).contains(leg);
                let cond_arrival = if conditions_after.contains(leg) {
                    arrival(leg).rising_ps as i32
                } else {
                    arrival(leg).falling_ps as i32
                };
                let cond = if cond_changed && t >= cond_arrival {
                    conditions_after.contains(leg)
                } else {
                    conditions_before.contains(leg)
                };
                let enable = if register_edges.contains(leg)
                    && t >= register_arrival(leg, rising_conditions) as i32
                {
                    enables_after.contains(leg)
                } else {
                    enables_before.contains(leg)
                };
                if cond && enable {
                    return true;
                }
            }
            false
        };

        // Scan segments for a captured 0→1: low for >= threshold, then high
        // for >= threshold (or steady to the end of the evaluation).
        let mut low_since = if level_at(0) {
            None
        } else {
            Some(i32::MIN / 2)
        };
        for (i, &t) in times.iter().enumerate() {
            let level = level_at(t);
            match (low_since, level) {
                (Some(since), true) => {
                    if t - since >= SUKO_CAPTURE_THRESHOLD_PS {
                        let high_until = times[i + 1..]
                            .iter()
                            .copied()
                            .find(|&u| !level_at(u))
                            .unwrap_or(STEADY_PS);
                        if high_until - t >= SUKO_CAPTURE_THRESHOLD_PS {
                            return true;
                        }
                    }
                    low_since = None;
                }
                (None, false) => low_since = Some(t),
                _ => {}
            }
        }
        false
    }

    /// LALU dffsr SUKO 0→1 capture, off-TALU boolean rule (write-time glitch
    /// evaluations and rise-edge evaluations — no input arrives mid-eval).
    pub(in crate::ppu) fn detect_suko_edge(&mut self, legs: InterruptFlags) -> bool {
        let prev = self.legs_was_high;
        self.legs_was_high = legs;

        let rising = legs - prev;
        let surviving = prev & legs;
        !rising.is_empty() && surviving.is_empty()
    }

    /// Prime the evaluation baselines at LCD-enable so the first evaluation
    /// sees no synthetic transition.
    pub(in crate::ppu) fn prime_baselines(
        &mut self,
        legs: InterruptFlags,
        conditions: InterruptFlags,
    ) {
        self.legs_was_high = legs;
        self.conditions_was = conditions;
    }
}

/// Gate-prop arrival time of each SUKO source leg at the AO2222 inputs, in ps from
/// the triggering TALU↑.
#[derive(Copy, Clone)]
struct LegArrival {
    rising_ps: u16,
    falling_ps: u16,
}

/// LYC arm via ROPO.dff17 (TALU-clocked, 1 stage).
const LYC_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 874,
    falling_ps: 874,
};
/// Mode 1 arm via NYPE + POPU.dffr + PARU.not_x1 (rising slower than falling — PMOS skew).
const MODE_1_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 2_822,
    falling_ps: 2_300,
};
/// Mode 0 arm via NYPE + POPU + PARU + TOLU.not_x1 + TARU.AND2.
const MODE_0_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 4_038,
    falling_ps: 4_038,
};
/// Mode 2 arm via NYPE + POPU + PARU + TOLU.not_x1 + TAPA.AND2.
const MODE_2_ARRIVAL: LegArrival = LegArrival {
    rising_ps: 3_970,
    falling_ps: 3_970,
};

/// LALU dffsr minimum captured SUKO low-pulse width. Cases 1 (1,802 ps) and 4
/// (1,524 ps) bracket the empirical threshold.
const SUKO_CAPTURE_THRESHOLD_PS: i32 = 1_700;

/// CGB register-path propagation from the FF41 synchroniser DFF output to a
/// leg's enable AND-term. The default holds the LYC (`E−874 ≥ threshold`) and
/// mode-1-fall (`E−2_300 < threshold`) corpus bounds. The deep mode-0 (WODU)
/// arm is slow enough that a same-fall enable-clear can't suppress a mode-0
/// condition rising on the same fall before its SUKO pulse is captured
/// (`E ≥ MODE_0_ARRIVAL.rising + SUKO_CAPTURE_THRESHOLD_PS`); it applies only to
/// that coincident-rising race, not to a steady mode-0 leg. No CGB netlist;
/// gambatte cgb04c pins these.
const REGISTER_ARRIVAL_DEFAULT: u16 = 3_300;
const REGISTER_ARRIVAL_MODE_0: u16 = 6_000;

fn register_arrival(leg: InterruptFlags, rising_conditions: InterruptFlags) -> u16 {
    if leg == InterruptFlags::HORIZONTAL_BLANK && rising_conditions.contains(leg) {
        REGISTER_ARRIVAL_MODE_0
    } else {
        REGISTER_ARRIVAL_DEFAULT
    }
}

fn arrival(leg: InterruptFlags) -> LegArrival {
    if leg == InterruptFlags::CURRENT_LINE_COMPARE {
        LYC_ARRIVAL
    } else if leg == InterruptFlags::VERTICAL_BLANK {
        MODE_1_ARRIVAL
    } else if leg == InterruptFlags::HORIZONTAL_BLANK {
        MODE_0_ARRIVAL
    } else if leg == InterruptFlags::OAM_SCAN {
        MODE_2_ARRIVAL
    } else {
        unreachable!("arrival(): non-single-leg flags 0x{:02X}", leg.bits());
    }
}
