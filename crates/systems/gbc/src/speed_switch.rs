use missingno_gb::Model;

use crate::Cgb;

/// CPU T-cycles the CPU stays `Stopped` during a double-speed switch (the
/// ~0x20000-T-cycle blackout). The divider and PPU run throughout; the CPU
/// re-engages at the new speed when this drains. Tuned against the age `spsw-*`
/// expected values.
const SPEED_SWITCH_BLACKOUT_TCYCLES: u32 = 0x2_0000;

/// Master edges of clock-mux relock tail after the 1×→2× hold: the dot clock
/// keeps stepping the PPU while the CPU clock is still settling, so the divider
/// stays quiet here (DIV is set by the hold alone) but the PPU advances — that
/// is the post-switch CPU↔dot re-phase.
const SWITCH_TO_DOUBLE_RELOCK_EDGES: u32 = 5;

/// Relock tail for the 2×→1× swap. The downward mux also settles to a phase;
/// it sets the CPU↔dot alignment the NEXT 1×→2× switch enters from, so over
/// repeated switches it determines whether the post-switch reads converge to
/// the single-switch alignment.
const SWITCH_TO_SINGLE_RELOCK_EDGES: u32 = 2;

impl Cgb {
    /// Master edges of the clock-mux relock tail at the end of the blackout.
    /// `double_speed` holds the NEW speed: the 1×→2× swap settles one way, the
    /// 2×→1× swap another (the latter sets the entry phase of the next swap).
    pub(crate) fn relock_edges(&self) -> u32 {
        if self.double_speed {
            SWITCH_TO_DOUBLE_RELOCK_EDGES
        } else {
            SWITCH_TO_SINGLE_RELOCK_EDGES
        }
    }

    /// Master edges (dot-clock half-cycles) the CPU stays held across a
    /// double-speed switch — a fixed real-time hold the dot clock runs through
    /// while the SM83 is frozen. The count's residue past a whole CPU M-cycle
    /// re-phases the SM83 against the dot clock at re-engage. `double_speed`
    /// already holds the new speed, so convert the T-cycle figure by the
    /// post-switch ratio (2 master edges per CPU T-cycle at single speed, 1 at
    /// double). The relock tail rides on the end (PPU only, divider quiet).
    pub(crate) fn speed_switch_blackout_master_edges(&self) -> u32 {
        let hold = SPEED_SWITCH_BLACKOUT_TCYCLES * 2 / self.cpu_steps_per_dot() as u32;
        hold + self.relock_edges()
    }

    /// An interrupt pending with IME set at the speed-switch STOP skips the
    /// post-STOP oscillation-stabilization HALT (Pan Docs STOP decision table):
    /// only the clock-mux relock tail remains, during which the divider is
    /// frozen — so DIV stays 0 until the CPU re-engages and services it.
    pub(crate) fn preempt_speed_switch_halt(&mut self) {
        self.speed_switch_blackout = self.relock_edges();
    }
}
