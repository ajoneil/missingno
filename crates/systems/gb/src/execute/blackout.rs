use super::{PhaseResult, StepResult};
use crate::{
    Console, ConsoleShadow, Model,
    clock::{CpuGate, Edge},
    cpu::mcycle::TCycle,
};

impl<M: Model> Console<M> {
    /// Resolve a STOP the CPU has settled into (called at the M-cycle
    /// boundary). The model decides: a CGB armed speed switch starts the
    /// blackout (the CPU stays stopped while the divider/PPU run, then
    /// re-engages at the new speed); otherwise the CPU stays stopped.
    /// `elapsed_tcycles` is the CPU T-cycle count of the step that just ran.
    /// Public for external phase-stepping drivers (tracing), which must call
    /// this at each instruction boundary like `step` does.
    pub fn resolve_stop(&mut self, _elapsed_tcycles: u32) {
        if !self.chassis.cpu.is_stopped() {
            return;
        }
        // The model owns the STOP outcome: a CGB armed KEY1 switch resets the
        // divider, retaps the APU/serial, aligns the clock ÷1/÷2 cell, and arms
        // the blackout (returning true); otherwise the CPU stays stopped. When
        // a switch happened the upward-grading escape byte completes here inside
        // the hold — the CPU is held and the bus free, so its tenure never parks
        // the resumed stream.
        if self.model.resolve_stop(&mut self.chassis) {
            while let Some((src, dst)) = self.model.vram_dma_drain_escape() {
                self.dma_move(src, dst);
            }
        }
    }

    /// Run one CPU M-cycle of the speed-switch blackout: a loop of held master
    /// edges (the clock's `Held` arm freezes the CPU and free-runs the dot
    /// domain), never entering the fused running path. Returns when the divider
    /// M-cycle completes — so the blackout drains across `step()`s — or earlier
    /// when the count empties and the CPU re-engages. `tcycles` reports the
    /// CPU-time equivalent so the step harness's accounting matches the running
    /// path.
    pub(super) fn step_blackout_chunk(&mut self) -> StepResult {
        let steps_per_dot = self.model.cpu_steps_per_dot() as u32;
        // Master edges per CPU M-cycle (4 T-cycles) and per CPU T-cycle: at
        // double speed a T-cycle is one master edge, at single speed it is two.
        let mcycle_edges = (8 / steps_per_dot).max(1);
        let edges_per_tcycle = (2 / steps_per_dot).max(1);

        let mut new_screen = false;
        let mut edges = 0u32;
        for _ in 0..mcycle_edges {
            // The gate records the dot phase the freeze lands on (the phase
            // signal a DS-HDMA straddle is distinguished by); the spike does not
            // yet consume it.
            let froze_on = self.chassis.clock.dot_phase();
            // Correctness relies on no Running edge falling between arming the
            // blackout anchor and draining it: the elapsed count is the
            // pre-advance anchor difference, the held edges already completed.
            let master_edge_before = self.chassis.clock.master_edge();
            let tick = self.chassis.clock.advance(CpuGate::Held { froze_on });
            let dot = tick.dot.expect("a held edge always carries a dot edge");
            let held_elapsed =
                master_edge_before.wrapping_sub(self.model.console_state().blackout_anchor());
            new_screen |= self.held_dot_advance(dot, held_elapsed).new_screen;
            edges += 1;
            if !self.chassis.cpu.is_stopped() {
                // The count emptied this edge and the CPU re-engaged; its first
                // fetch runs on the next step()'s normal loop.
                break;
            }
        }

        self.chassis.cpu.mark_instruction_boundary();
        StepResult {
            new_screen,
            sram_dirty: self.chassis.external.cartridge.sram_dirty,
            tcycles: edges / edges_per_tcycle,
        }
    }

    /// One held master edge of the speed-switch blackout: the CPU clock is frozen
    /// (`execute_phase` already advanced the dot domain) and the dot clock alone
    /// ran. Step the PPU one edge with the per-dot APU tick riding it, and pulse
    /// the CPU-clock divider (timer/serial + the CGB STAT crossing) at the CPU
    /// rate off the master count. The CPU phase is untouched, so when the count
    /// empties the SM83 re-engages at whatever dot-clock phase this edge is, and
    /// the post-switch re-phase emerges from the count alone. `dot` is the edge
    /// this held advance fired; `elapsed` is the master edges already drained
    /// (an anchor difference).
    fn held_dot_advance(&mut self, dot: Edge, elapsed: u64) -> PhaseResult {
        let double_speed = self.double_speed_active();
        let steps_per_dot = self.model.cpu_steps_per_dot() as u64;
        let mcycle_edges = (8 / steps_per_dot).max(1);

        // The M-cycle phase, derived from elapsed master edges so it pulses
        // at the CPU rate independent of the frozen SM83.
        let mcycle_boundary = elapsed.is_multiple_of(mcycle_edges);

        // The divider/STAT crossing run at the CPU rate through the hold but
        // freeze during the clock-mux relock tail (the CPU clock is settling),
        // so the re-phase tail advances the PPU without disturbing DIV.
        if mcycle_boundary && self.model.speed_switch_divider_active() {
            self.chassis.ppu.tick_clock_domain_capture();
            self.tick_cpu_clock_mcycle();
        }

        let (new_screen, pixel) = match dot {
            Edge::Rise => {
                let r = self.ppu_rise_edge();
                // Held edges pin `t_index` and skip the M-boundary, so the APU's
                // span model does not describe them.
                self.chassis.audio.suspend_span();
                self.chassis
                    .audio
                    .tcycle(self.chassis.timers.internal_counter(), 0, double_speed);
                r
            }
            Edge::Fall => {
                let video_result = self.ppu_fall_edge(mcycle_boundary, TCycle::ZERO);
                self.chassis.audio.fall_sync();
                self.apply_ppu_fall(&video_result)
            }
        };

        // One master edge of the blackout spent; re-engage the moment it empties —
        // or early, the moment an enabled interrupt is pending. The post-STOP HALT
        // wakes on IE&IF like an ordinary HALT (a timer overflowing mid-wait, or an
        // interrupt already pending at the STOP). Only past the relock tail.
        let woken_by_interrupt = self.model.speed_switch_divider_active()
            && self.chassis.interrupts.triggered().is_some();
        // The mid-HALT timer wake spends the HALT-wake's WakeIntake M-cycle (the
        // divider ticking through it) before re-engaging; the pending-at-STOP
        // preempt path drains the bare relock tail with no such wake.
        let woken_ready = woken_by_interrupt && self.model.speed_switch_wake_ready(mcycle_boundary);
        let drain = if woken_ready { u32::MAX } else { 1 };
        if self.model.drain_speed_switch_blackout(drain) {
            // An enabled interrupt is serviced at re-engage, dispatching from the
            // post-STOP boundary. The mid-HALT timer wake is a HALT wake (M1, the
            // byte after STOP its return target); the pending-at-STOP 1-byte STOP
            // resumes mid-dispatch at M2 (its M1 was the pre-reset operand fetch).
            let dispatch_step = (self.chassis.cpu.interrupts_enabled()
                && self.chassis.interrupts.triggered().is_some())
            .then_some(if woken_by_interrupt { 0 } else { 1 });
            self.chassis.cpu.resume_from_stop(dispatch_step);
            // The fetch begins on a CPU rising edge.
            self.chassis.clock.engage_on_rise();
            // Reinstate the DIV-APU tap-retune slip now the divider is live again.
            self.chassis.audio.on_speed_resume();
        }

        PhaseResult { new_screen, pixel }
    }
}
