//! The KEY1 double-speed switch: the STOP that performs it, the blackout the
//! CPU spends held, and the wake that ends it.

use missingno_gb::clock::CpuDivider;
use missingno_gb::{Chassis, ConsoleShadow, Model};

use crate::Cgb;
use crate::vram_dma::{TransferMode, VramDma};

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
        let hold = SPEED_SWITCH_BLACKOUT_TCYCLES * 2 / self.steps_per_dot() as u32;
        hold + self.relock_edges()
    }

    pub(crate) fn steps_per_dot(&self) -> u8 {
        if self.double_speed { 2 } else { 1 }
    }

    /// An interrupt pending with IME set at the speed-switch STOP skips the
    /// post-STOP oscillation-stabilization HALT (Pan Docs STOP decision table):
    /// only the clock-mux relock tail remains, during which the divider is
    /// frozen — so DIV stays 0 until the CPU re-engages and services it.
    pub(crate) fn preempt_speed_switch_halt(&mut self) {
        self.speed_switch_blackout = self.relock_edges();
    }

    pub(crate) fn blackout_active(&self) -> bool {
        self.speed_switch_blackout > 0
    }

    pub(crate) fn drain_blackout(&mut self, elapsed: u32) -> bool {
        self.speed_switch_blackout = self.speed_switch_blackout.saturating_sub(elapsed);
        if self.speed_switch_blackout == 0 {
            self.speed_switch_wake_latency = None;
        }
        self.speed_switch_blackout == 0
    }

    pub(crate) fn blackout_divider_active(&self) -> bool {
        // The divider runs through the hold but freezes during the relock tail:
        // the CPU clock is still settling there, so it gains no edges (this keeps
        // the re-phase from disturbing DIV). The tail is the final `relock`
        // master edges of the count. Placing the quiet edges at the tail vs the
        // head is observationally identical (no test in the corpus latches a
        // divider-driven event in that window), so this picks the resume-side
        // offset that SameBoy/gambatte also model.
        self.speed_switch_blackout > self.relock_edges()
    }

    pub(crate) fn wake_intake_ready(&mut self, mcycle_boundary: bool) -> bool {
        let latency = match self.speed_switch_wake_latency {
            None => 1, // First IF-set edge: arm the WakeIntake M-cycle.
            Some(n) if mcycle_boundary && n > 0 => n - 1,
            Some(n) => n,
        };
        self.speed_switch_wake_latency = Some(latency);
        latency == 0
    }

    pub(crate) fn attempt_speed_switch(&mut self, chassis: &mut Chassis<Self>) -> bool {
        // The settle is bus-coupled: a bus master holding the CPU defers it.
        if self.console_state().dma_cpu_hold() {
            return false;
        }
        // Mid-blackout: the held-edge stepping owns the countdown and re-engage.
        if self.blackout_active() {
            return false;
        }
        if !self.key1_armed {
            return false;
        }
        // The mux settle is placed against the dot phase, which a span defers.
        chassis.ppu.sync_span();
        let entry_dot_phase = chassis.ppu.dot_in_mcycle_phase();

        // The clock-mux settle is bus-coupled, and only the upward swap
        // disturbs the mux and resets the trigger's request/commit
        // chain (the CPU-written arming/length registers persist).
        if self.double_speed {
            // Downward: the chain survives, so a granted burst keeps
            // the bus and the settle waits for its release.
            if self.vram_dma.block.remaining > 0 && !self.vram_dma.block.ready_in.active() {
                return false;
            }
        } else {
            // Upward: the reset grades the committed block, which is
            // cancel-immune and ignores the arming flag. Not yet
            // bus-eligible: discarded whole. Bus-eligible: the dropped
            // grant latches the stop condition — the in-flight byte
            // completes outside the latched length.
            if self.vram_dma.block.remaining > 0 {
                // Either grading leg revokes the committed block's FF55
                // count (the dropped grant re-arms the status).
                self.vram_dma.arb.granted_ahead = self.vram_dma.arb.granted_ahead.saturating_sub(1);
                self.vram_dma.arb.grant_counted = false;
                if !self.vram_dma.block.ready_in.active() {
                    self.vram_dma.cursor.mode = TransferMode::Idle;
                    self.vram_dma.block.remaining = 1;
                    self.vram_dma.cursor.escape_byte = true;
                } else {
                    self.vram_dma.block.remaining = 0;
                    self.vram_dma.block.ready_in.clear();
                    self.vram_dma.block.setup_cells.clear();
                }
            }
            self.vram_dma.arb.pend = false;
        }
        self.double_speed = !self.double_speed;
        self.key1_armed = false;
        self.speed_switch_blackout = self.speed_switch_blackout_master_edges();
        if self.double_speed {
            self.vram_dma
                .arb
                .wake_pend_blind
                .arm(VramDma::WAKE_PEND_BLIND_TICKS);
        }
        // A 1×→2× relock entered at dot-in-M phase p3 lands the mux
        // displaced (cost-free); a displaced 2×→1× completes a dot early
        // and stays displaced; the following 1×→2× spends the dot back
        // re-syncing — and a re-sync suppresses a fresh displacement.
        if self.double_speed {
            if self.switch_relock_debit {
                self.speed_switch_blackout += 2;
                self.switch_relock_debit = false;
            } else if entry_dot_phase == Some(3) {
                self.switch_relock_debit = true;
            }
        } else if self.switch_relock_debit {
            self.speed_switch_blackout -= 2;
        }

        // Hardware resets DIV across the switch (the speed bit is now toggled).
        // The CPU clock is held while the dot clock runs the blackout out; the
        // held-edge stepping advances the master clock every edge and re-engages
        // at the phase the count expires on.
        let old_counter = chassis.timers.internal_counter();
        let to_double = self.double_speed;
        chassis.timers.reset_for_speed_switch();
        chassis
            .audio
            .on_div_write(old_counter.wrapping_sub(1), !to_double);
        chassis.audio.on_speed_switch(to_double);
        if let Some(interrupt) = chassis
            .serial
            .on_div_write(old_counter, self.has_serial_fast_clock())
        {
            chassis.interrupts.request(interrupt);
        }
        // KEY1 has flipped the speed bit; align the clock's ÷1/÷2 cell to the
        // new ratio so the clock stays the sole ratio owner.
        chassis.clock.set_divider(if to_double {
            CpuDivider::Two
        } else {
            CpuDivider::One
        });
        // An interrupt pending with IME set at the STOP preempts the post-STOP
        // HALT: the switch happens but the CPU services the interrupt at once
        // (DIV ≈ 0), not after the long wait.
        if chassis.cpu.interrupts_enabled() && chassis.interrupts.triggered().is_some() {
            self.preempt_speed_switch_halt();
        }
        // Anchor the held-edge count at the current master edge; the blackout's
        // elapsed count is `master_edge - blackout_anchor`.
        let anchor = chassis.clock.master_edge();
        self.console_state_mut().set_blackout_anchor(anchor);
        true
    }
}
