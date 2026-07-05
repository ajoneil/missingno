use super::{
    Console, ConsoleShadow, Model, ScreenBuffer,
    clock::{CpuGate, Edge, TcycleSchedule},
    cpu::mcycle::{BusAction, TCycle},
    cpu_bus::{BusAccess, BusAccessKind},
    interrupts::Interrupt,
    ppu,
};

/// Result of executing one instruction.
pub struct StepResult {
    /// Whether a new video frame was produced during this instruction.
    pub new_screen: bool,
    /// Whether battery-backed SRAM was written during this instruction.
    pub sram_dirty: bool,
    /// Number of T-cycles consumed by this instruction.
    pub tcycles: u32,
}

/// Result of executing one half-phase (rise or fall).
pub struct PhaseResult {
    /// Whether a new video frame was produced.
    pub new_screen: bool,
    /// Pixel pushed to the LCD during this phase, if any.
    pub pixel: Option<ppu::PixelOutput>,
}

/// The facts a rising master-clock edge settles in its pre-PPU-rise work,
/// carried from `rise_cpu_pre` to `rise_cpu_post`: whether the edge opened a
/// new M-cycle, and the CPU T-cycle the PPU rise is keyed to.
#[derive(Clone, Copy)]
struct RiseEdge {
    mcycle_boundary: bool,
    tcycle: TCycle,
}

/// The facts a falling master-clock edge settles before its PPU fall, carried
/// from `fall_cpu_pre` to `fall_cpu_post`: the CPU T-cycle, whether the edge
/// opened a new M-cycle, an LY value sampled pre-edge for a coincident FF44
/// read latch, and the PPU mode the HDMA trigger reads before the fall mutates
/// it.
#[derive(Clone, Copy)]
struct FallEdge {
    tcycle: TCycle,
    mcycle_boundary: bool,
    ly_at_latch: Option<u8>,
    pre_fall_mode: ppu::Mode,
}

impl<M: Model> Console<M> {
    pub fn step(&mut self) -> StepResult {
        self.step_traced(false).0
    }

    /// Step one instruction, optionally recording all bus accesses.
    /// Returns (result, trace). Trace is empty when `trace` is false.
    pub fn step_traced(&mut self, trace: bool) -> (StepResult, Vec<BusAccess>) {
        if trace {
            self.chassis.bus_trace.enable();
        }

        // If step_tcycle() left us mid-instruction, drain to the next
        // boundary first, then run one full instruction.
        let mut new_screen = false;
        let mut tcycles = 0u32;
        if !self.chassis.cpu.at_instruction_boundary() {
            let r = self.step_instruction();
            new_screen |= r.new_screen;
            tcycles += r.tcycles;
        }
        let r = self.step_instruction();
        new_screen |= r.new_screen;
        tcycles += r.tcycles;

        self.resolve_stop(tcycles);
        self.manage_dma_hold();

        let sram_dirty = self.chassis.external.cartridge.take_sram_dirty();
        (
            StepResult {
                new_screen,
                sram_dirty,
                tcycles,
            },
            self.chassis.bus_trace.take(),
        )
    }

    /// Run one complete instruction from start to finish.
    ///
    /// Runs phases until the CPU returns to the Fetch phase at a fresh
    /// M-cycle boundary (instruction boundary). At that point, EI delay
    /// is advanced and control returns to the caller.
    fn step_instruction(&mut self) -> StepResult {
        let mut new_screen = false;
        self.chassis.cpu.data_latch = 0;

        // Consume the current instruction boundary (we're starting
        // from a boundary — we want to run until the NEXT one).
        self.chassis.cpu.take_instruction_boundary();

        // Speed-switch blackout: the CPU clock is held while the dot clock
        // keeps running. Drive one CPU M-cycle of held master edges through the
        // same `execute_phase` loop (the gate is `Held`, the CPU frozen) and
        // return, draining the blackout across step()s until the count empties.
        if self.chassis.cpu.is_stopped() && self.model.speed_switch_in_progress() {
            return self.step_blackout_chunk();
        }

        const TCYCLE_BUDGET: u32 = 400;
        let mut tcycles_remaining = TCYCLE_BUDGET;
        let mut tcycles = 0u32;

        loop {
            assert!(
                tcycles_remaining > 0,
                "step() exceeded {TCYCLE_BUDGET} T-cycle budget — possible infinite loop in CPU"
            );
            tcycles_remaining -= 1;

            new_screen |= self.execute_tcycle();
            tcycles += 1;
            if self.chassis.cpu.at_instruction_boundary() {
                break;
            }
        }
        // Don't drain sram_dirty here — let the caller (step_traced) do it
        // so the flag accumulates across multiple step_instruction calls.
        let sram_dirty = self.chassis.external.cartridge.sram_dirty;
        StepResult {
            new_screen,
            sram_dirty,
            tcycles,
        }
    }

    /// Advance one CPU T-cycle — its two master edges, rise then fall — as one
    /// straight-line flow. The [`TcycleSchedule`] read up front names where the
    /// dot's rise and fall land: at ÷1 the rise carries the dot rise and the
    /// fall the dot fall; at ÷2 the T-cycle carries one dot edge, on its rise.
    /// The unit every step loop and the debugger advance by. Returns whether a
    /// new frame was produced across the pair. A T-cycle boundary is always
    /// rise-aligned, so the pair starts on the rise edge and ends back on a rise.
    pub fn execute_tcycle(&mut self) -> bool {
        let schedule = self.chassis.clock.tcycle_schedule();
        let rise = self.tcycle_rise(schedule);
        let fall = self.tcycle_fall(schedule);
        rise.new_screen || fall.new_screen
    }

    /// Advance one CPU T-cycle, observing the machine after each of its two
    /// master edges — the gbtrace capture hook. `after_phase` runs after the
    /// rise then after the fall with that edge's [`PhaseResult`], so a tracer
    /// keeps its exact between-edges sample points. A `Break` from the rise's
    /// observer leaves the fall for the next call: the double-speed per-edge
    /// capture defers the paired edge when the instruction retires on the rise,
    /// leaving the clock parked on the fall — so a resuming call runs only the
    /// fall (the pre-advance dot phase reproduces this T-cycle's schedule, since
    /// the ÷2 rise leaves the dot phase untouched).
    #[cfg(feature = "gbtrace")]
    pub fn execute_tcycle_observed(
        &mut self,
        mut after_phase: impl FnMut(&mut Self, &PhaseResult) -> std::ops::ControlFlow<()>,
    ) -> bool {
        let schedule = self.chassis.clock.tcycle_schedule();
        let mut new_screen = false;
        if self.chassis.clock.cpu_edge() == Edge::Rise {
            let rise = self.tcycle_rise(schedule);
            new_screen |= rise.new_screen;
            if after_phase(self, &rise).is_break() {
                return new_screen;
            }
        }
        let fall = self.tcycle_fall(schedule);
        new_screen |= fall.new_screen;
        // The fall is the pair's last edge; nothing to defer, so its control
        // signal is irrelevant.
        let _ = after_phase(self, &fall);
        new_screen
    }

    /// Advance to the next T-cycle boundary. Returns true if a new frame was
    /// produced. Consumes the boundary flag so a following `step`/`step_traced`
    /// sees mid-instruction state.
    pub fn step_tcycle(&mut self) -> bool {
        let new_screen = self.execute_tcycle();
        self.chassis.cpu.take_instruction_boundary();
        new_screen
    }

    /// The rising master edge of a T-cycle: advance the clock, then the CPU's
    /// pre-rise work, the dot edge the schedule places here, and the post-rise
    /// work. The rise always carries a dot edge — a dot rise at ÷1 and on the
    /// first ÷2 T-cycle, a dot fall on the second ÷2 T-cycle (a double-speed
    /// dot fall lands on a CPU rise, half a dot from the dot's own rise). The
    /// PPU rise is its own domain's edge, sequenced between the CPU's pre- and
    /// post-rise work rather than welded inside it.
    fn tcycle_rise(&mut self, schedule: TcycleSchedule) -> PhaseResult {
        // `dot_work` (the APU prescaler / CH3 sync / HDMA-trigger ride) belongs to
        // the dot rise; on the ÷2 T-cycle that carries the dot fall on this rise
        // it defers to the following CPU fall.
        let (ppu, dot_work) = match schedule {
            TcycleSchedule::FullDot | TcycleSchedule::DotRiseOnRise => (Edge::Rise, true),
            TcycleSchedule::DotFallOnRise => (Edge::Fall, false),
        };
        self.chassis.clock.advance(CpuGate::Running);

        let rise = self.rise_cpu_pre(ppu, dot_work);
        let (new_screen, pixel) = self.fire_dot_ppu(ppu, rise.mcycle_boundary, rise.tcycle);
        self.rise_cpu_post(rise);

        // Every rise carries a dot edge, so the ÷2 mode-2 onset settle rides it;
        // the interrupt set-settle rides every master edge. Both feed double-speed
        // read placement only — every consumer sits behind double_speed_active(),
        // so consoles without the ÷2 cell never read them.
        if M::DOUBLE_SPEED {
            self.chassis.ppu.tick_onset_settles();
            self.chassis.interrupts.tick_set_settles();
        }
        PhaseResult { new_screen, pixel }
    }

    /// The falling master edge of a T-cycle: advance the clock, then the CPU's
    /// pre-fall work, the dot fall the schedule places here (only at ÷1 — the ÷2
    /// fall is bare, the dot fall having ridden a CPU rise), and the post-fall
    /// work. The PPU fall is its own domain's edge, sequenced between the CPU's
    /// pre- and post-fall work rather than welded inside it.
    fn tcycle_fall(&mut self, schedule: TcycleSchedule) -> PhaseResult {
        // The ÷1 fall carries the dot fall; the ÷2 fall is bare. On the ÷2
        // T-cycle whose rise carried the dot fall, `dot_work` runs here — the dot
        // rise it belongs to fires next, so this fall is the dot's bare second
        // CPU edge that owes the deferred dot work.
        let (has_dot_fall, dot_work) = match schedule {
            TcycleSchedule::FullDot => (true, true),
            TcycleSchedule::DotRiseOnRise => (false, false),
            TcycleSchedule::DotFallOnRise => (false, true),
        };
        self.chassis.clock.advance(CpuGate::Running);

        let fall = self.fall_cpu_pre(dot_work);
        let video_result = if has_dot_fall {
            Some(self.ppu_fall_edge(fall.mcycle_boundary, fall.tcycle))
        } else {
            None
        };
        let (new_screen, pixel) = self.fall_cpu_post(fall, video_result, dot_work);

        // Only the ÷1 fall carries a dot edge for the onset settle to ride; the
        // set-settle rides every master edge.
        if M::DOUBLE_SPEED {
            if has_dot_fall {
                self.chassis.ppu.tick_onset_settles();
            }
            self.chassis.interrupts.tick_set_settles();
        }
        PhaseResult { new_screen, pixel }
    }

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

    /// Engage or release the CPU-clock hold a VRAM DMA asserts. While the DMA
    /// holds the bus the CPU spins and its bytes flow per M-cycle in
    /// `tick_mcycle_boundary_fall`; the PPU/timers keep running. Called at the
    /// instruction boundary (also by external phase-stepping drivers).
    pub fn manage_dma_hold(&mut self) {
        // An HBlank block owning the bus finishes before a GDMA hold engages
        // (the two cannot share the buses), and the dispatch tenure is
        // indivisible — the hold waits for it like the HDMA grant does.
        if self.model.console_state().bus_suspended() || self.chassis.cpu.in_dispatch() {
            return;
        }
        let holds = self.model.vram_dma_holds_cpu();
        let held = self.model.console_state().dma_cpu_hold();
        if holds && !held {
            self.model.console_state_mut().set_dma_cpu_hold(true);
            self.chassis.cpu.begin_bus_hold();
        } else if !holds && held {
            self.model.console_state_mut().set_dma_cpu_hold(false);
            self.chassis.cpu.end_bus_hold();
        }
    }

    /// Move one DMA byte: read the bus source, write the mapped destination
    /// (OAM or the VBK-selected VRAM bank), trace both, decay the source bus.
    /// The single byte-transfer OAM DMA and the CGB VRAM DMA share.
    fn dma_move(&mut self, source: u16, dest: u16) {
        let byte = self.read_dma_source(source);
        self.chassis.dma_commit(source, dest, byte);
    }

    /// CPU work for a rising master-clock edge, optionally carrying a PPU edge.
    /// The CPU's per-rise advance shared by both boundary paths: the T-cycle
    /// step, vector resolve at T3, dispatch logic, and the APU prescaler tick.
    /// Runs before the PPU rise off a boundary, after it on an M-boundary.
    fn rise_cpu_advance(&mut self, dot_work: bool) -> TCycle {
        let state = self.model.console_state();
        let grants = crate::cpu::mcycle::BusGrants {
            suspended: state.bus_suspended(),
            held: state.dma_cpu_hold(),
            claim: state.vram_dma_claim(),
        };
        self.chassis.cpu.next_tcycle(grants);
        // cpu_irq_ack1↑ at +2.993 dots into the dispatching M-cycle —
        // tcycle 3 rise in our half-phase resolution. Deferring to
        // tcycle 3 also lets M4's bus write commit (tcycle 2 fall)
        // before vector resolution reads IE (IE-push-bug semantics).
        if self.chassis.cpu.last_tcycle().as_u8() == 3 {
            self.apply_vector_resolve();
        }

        let tcycle = self.chassis.cpu.last_tcycle();
        self.step_dispatch_logic(tcycle);

        // APU prescaler tick (apuv ↑) on every master-clock rise.
        if dot_work {
            let double_speed = self.double_speed_active();
            self.chassis.audio.tcycle(
                self.chassis.timers.internal_counter(),
                tcycle.as_u8(),
                double_speed,
            );
        }
        tcycle
    }

    /// All CPU work on a rising edge before its PPU rise, plus the T-cycle the
    /// PPU edge is keyed to. The PPU rise fires after the T-cycle advance on
    /// every dot — one consistent CPU↔PPU phase (the spec pins a single fixed
    /// lattice; there is no per-dot CPU edge for it to vary against). The
    /// M-boundary additionally runs its boundary CPU work and the HDMA grant.
    fn rise_cpu_pre(&mut self, ppu: Edge, dot_work: bool) -> RiseEdge {
        let is_mcycle_boundary = self.chassis.cpu.consume_boundary_pending();

        // Pre-ALET-rise XYMU (mode-3) view: the mode 3→0 XYMU.q↑ fires inside
        // this dot's `ppu_rise_edge`. A double-speed FF41 read latching on the
        // same phase resolves its mode to this pre-transition view (the CGB
        // CPU↔ALET read placement). Only double speed consumes it.
        if ppu == Edge::Rise && self.double_speed_active() {
            self.model
                .note_pre_alet_rendering(self.chassis.ppu.is_rendering());
            if let Some(address) = self.chassis.cpu_bus.read_address() {
                self.model
                    .note_pre_alet_lock(self.chassis.ppu.read_lock(address));
            }
        }

        let tcycle = if is_mcycle_boundary {
            self.tick_mcycle_boundary_rise();
            self.chassis.audio.mcycle_boundary();
            // The HDMA grant is M-boundary-quantized: bus ownership asserts and
            // releases between M-cycles only. A dispatch sequence already in
            // flight when the transfer became ready holds the bus through its
            // M-cycles (the grant defers); a dispatch starting with the transfer
            // ready parks behind the block. Granted ownership is never revoked.
            // One-shot arbitration at the dispatch's M1 pick: a request
            // standing at the pick makes the dispatch yield its entire tenure
            // (chained blocks included); otherwise it holds the bus through
            // its M-cycles. The pick flips the phase after this point, so
            // entry is detected one boundary later against the request line's
            // one-boundary synchronizer stage.
            if self.chassis.cpu.in_dispatch() {
                self.chassis.cpu.bus_arbitration.sample_pick_if_entering();
            } else {
                self.chassis.cpu.bus_arbitration.clear_pick();
            }
            // Grant mode by the CPU's state at the commit: a halted-CPU
            // commit grants at the next M-boundary (the claim-standing
            // synchronizer still governs the halt-exit refetch); a running-CPU
            // commit waits for the in-flight instruction to retire.
            if self.chassis.cpu.bus_arbitration.take_at_boundary() {
                self.model.vram_dma_instruction_retired();
            }
            let suspended = self.model.vram_dma_seizes_bus()
                && (self.model.console_state().bus_suspended()
                    || if self.chassis.cpu.in_dispatch() {
                        self.chassis.cpu.bus_arbitration.parks_behind_dma()
                    } else {
                        !self.model.vram_dma_park_waits_for_fetch()
                    });
            self.model.console_state_mut().set_bus_suspended(suspended);
            self.chassis
                .cpu
                .bus_arbitration
                .shift_request(self.model.vram_dma_request_standing());
            let tcycle = self.rise_cpu_advance(dot_work);
            // The M-cycle pick inside `rise_cpu_advance` consumed the claim; clear
            // the stored claim so a fresh one can commit in the new M-cycle (the
            // relocated per-M reset).
            self.model.console_state_mut().clear_vram_dma_claim();
            self.stage_mcycle_bus_activity();
            tcycle
        } else {
            self.rise_cpu_advance(dot_work)
        };

        if M::HAS_OAM_BUG && tcycle.as_u8() == 0 {
            self.arm_oam_bugs();
        }
        if !is_mcycle_boundary {
            self.tick_non_boundary_rise(tcycle, ppu == Edge::Fall);
        }
        RiseEdge {
            mcycle_boundary: is_mcycle_boundary,
            tcycle,
        }
    }

    /// CPU work on a rising edge after its PPU rise: off a boundary the dispatch
    /// latch update; an armed OAM bug fires last on both paths.
    fn rise_cpu_post(&mut self, rise: RiseEdge) {
        if !rise.mcycle_boundary {
            self.chassis.cpu.dispatch.update_latch(
                self.chassis.interrupts.enabled,
                self.chassis.interrupts.requested,
            );
        }

        // MOPA-rising fires any armed OAM bug.
        if M::HAS_OAM_BUG && rise.tcycle.as_u8() == 2 {
            self.chassis.ppu.apply_pending_oam_bug();
        }
    }

    /// PPU rising-edge advance and its interrupt readback: pixel output,
    /// VBlank IF, the STAT edge, and the CPU's interrupt-state refresh.
    fn ppu_rise_edge(&mut self) -> (bool, Option<ppu::PixelOutput>) {
        let oam_bus = self.chassis.dma.oam_bus_owner();
        let ppu_result = self
            .chassis
            .ppu
            .on_master_clock_rise(&self.chassis.vram_bus.vram, oam_bus);
        if ppu_result.request_vblank {
            self.chassis
                .interrupts
                .request(Interrupt::VideoBetweenFrames);
        }
        let (new_screen, pixel) = self.apply_ppu_result(&ppu_result);
        if self.chassis.ppu.check_stat_edge() {
            self.chassis.interrupts.request(Interrupt::VideoStatus);
        }
        let triggered = self.chassis.interrupts.triggered();
        self.chassis.cpu.update_interrupt_state(triggered);
        (new_screen, pixel)
    }

    /// PPU falling-edge advance: divider chain, CATU, scanline boundaries,
    /// fetcher, DFF8/DFF9, LCD-off. The caller applies the returned result's
    /// IF requests and pixel output.
    fn ppu_fall_edge(
        &mut self,
        is_mcycle_boundary: bool,
        tcycle: TCycle,
    ) -> ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel> {
        let oam_bus = self.chassis.dma.oam_bus_owner();
        // The M-cycle's last PPU fall, where the WY/WX/LCDC.5/LCDC.2 crossing
        // captures — resolved by the divider cell from the ratio.
        let mcycle_last_fall = self
            .chassis
            .clock
            .divider()
            .mcycle_last_fall(is_mcycle_boundary, tcycle.as_u8());
        self.chassis
            .ppu
            .on_master_clock_fall(is_mcycle_boundary, mcycle_last_fall, oam_bus)
    }

    /// Apply a PPU fall's outputs: VBlank/STAT IF requests and the pixel/screen
    /// commit. The `cpu_irq_ack1` re-assert is the caller's (it runs on every
    /// CPU fall, not only the dot's PPU fall).
    fn apply_ppu_fall(
        &mut self,
        video_result: &ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>,
    ) -> (bool, Option<ppu::PixelOutput>) {
        let double_speed = self.double_speed_active();
        // VBlank IF: POPU transitions happen on the fall since the divider
        // chain runs there.
        if video_result.request_vblank {
            self.chassis
                .interrupts
                .request_ppu_fall(Interrupt::VideoBetweenFrames, double_speed);
        }
        // STAT IF: the SUKO check folds into request_stat; cpu_irq_ack1_pulse
        // (LALU.r_n=0) absorbs same-M-cycle SUKO rises.
        if video_result.request_stat && !self.chassis.cpu.irq.cpu_irq_ack1_pulse {
            self.chassis
                .interrupts
                .request_ppu_fall(Interrupt::VideoStatus, double_speed);
        }
        self.apply_ppu_result(video_result)
    }

    /// Run one CPU M-cycle of the speed-switch blackout: a loop of held master
    /// edges (the clock's `Held` arm freezes the CPU and free-runs the dot
    /// domain), never entering the fused running path. Returns when the divider
    /// M-cycle completes — so the blackout drains across `step()`s — or earlier
    /// when the count empties and the CPU re-engages. `tcycles` reports the
    /// CPU-time equivalent so the step harness's accounting matches the running
    /// path.
    fn step_blackout_chunk(&mut self) -> StepResult {
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
        let mcycle_boundary = elapsed % mcycle_edges == 0;

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

    /// Run the PPU edge a CPU edge carries (if any). The rise outputs pixel +
    /// VBlank/STAT-edge IF; the fall runs the divider chain and applies its
    /// outputs. Double speed places the master fall on the High arm's rise.
    fn fire_dot_ppu(
        &mut self,
        ppu: Edge,
        is_mcycle_boundary: bool,
        tcycle: TCycle,
    ) -> (bool, Option<ppu::PixelOutput>) {
        match ppu {
            Edge::Rise => self.ppu_rise_edge(),
            Edge::Fall => {
                // A dot fall on a CPU rise (double speed only): an LY tick on
                // the read's own T2 rise sits 3 half-edges before the latch,
                // inside the mux ripple — stash LY_old for the latch's AND. A
                // tick earlier in the M (T0) has settled by the latch.
                let ripple_old =
                    if self.chassis.cpu_bus.read_address() == Some(0xFF44) && tcycle.as_u8() == 2 {
                        Some(self.read(0xFF44))
                    } else {
                        None
                    };
                let video_result = self.ppu_fall_edge(is_mcycle_boundary, tcycle);
                if let Some(old) = ripple_old
                    && self.read(0xFF44) != old
                {
                    self.model.note_ff44_ripple_old(Some(old));
                }
                self.apply_ppu_fall(&video_result)
            }
        }
    }

    /// CPU work on a falling edge before its PPU fall: CH3 wave-latch sync, the
    /// T2 read drive-enable, the pre-edge LY sample, and the pre-fall mode the
    /// HDMA trigger reads. The PPU fall is the caller's, sequenced after this.
    fn fall_cpu_pre(&mut self, dot_work: bool) -> FallEdge {
        let tcycle = self.chassis.cpu.last_tcycle();
        let is_mcycle_boundary = self.chassis.cpu.at_mcycle_boundary();

        // CH3's BUSA / AZUS DFFs latch on apu_4mhz ↑ (= our fall);
        // settle before the T=2 drive-enable so wave-RAM reads see
        // the current wave_data_latch.
        if dot_work {
            self.chassis.audio.fall_sync();
        }

        if tcycle.as_u8() == 2 {
            self.apply_read_drive_enable();
        }

        // data_phase_n↑ precedes this fall's edge: sample LY pre-edge so an
        // FF44 latch coincident with the RUTU-clocked capture resolves the
        // mid-ripple flux.
        let ly_at_latch = match self.chassis.cpu.last_bus_action {
            BusAction::Read { address: 0xFF44 } => Some(self.read(0xFF44)),
            _ => None,
        };

        let pre_fall_mode = self.chassis.ppu.mode();

        FallEdge {
            tcycle,
            mcycle_boundary: is_mcycle_boundary,
            ly_at_latch,
            pre_fall_mode,
        }
    }

    /// CPU work on a falling edge after its PPU fall: STAT-sync capture, the
    /// read latch and write commit, the HDMA trigger, the fall path's IF
    /// requests, and the DMA/timer ticks. `video_result` is the PPU fall's
    /// output, `None` on the double-speed CPU T-cycle that carries no PPU fall.
    fn fall_cpu_post(
        &mut self,
        fall: FallEdge,
        video_result: Option<ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>>,
        dot_work: bool,
    ) -> (bool, Option<ppu::PixelOutput>) {
        let standalone_stat =
            self.capture_standalone_stat_sync(video_result.is_some(), fall.mcycle_boundary);

        if fall.tcycle.as_u8() == 2 {
            self.sample_mid_cupa_lock();
        }

        self.commit_read_latch(fall.ly_at_latch);
        self.commit_write();

        self.tick_vram_dma_trigger(dot_work, fall.pre_fall_mode);

        self.request_fall_path_interrupts(&video_result, standalone_stat);
        self.reclear_held_ack();

        let (new_screen, pixel) = self.apply_fall_ppu_result(video_result.as_ref());

        self.clock_oam_dma_gate(fall.tcycle);

        if fall.mcycle_boundary {
            self.tick_mcycle_boundary_fall();
        }

        self.recapture_interrupts();
        (new_screen, pixel)
    }

    /// Double-speed boundary fall sharing a dot with no PPU fall: the
    /// CPU-clocked STAT register synchroniser still captures; its request
    /// joins the fall path's gating.
    fn capture_standalone_stat_sync(
        &mut self,
        has_ppu_fall: bool,
        is_mcycle_boundary: bool,
    ) -> bool {
        !has_ppu_fall && is_mcycle_boundary && self.chassis.ppu.capture_register_sync_standalone()
    }

    /// HDMA trigger, evaluated each dot's fall with this fall's write
    /// commit visible: the pend forms on the post-rise mode view and
    /// commits to cancel-immunity one fall later (the pend pipeline
    /// lives in the model).
    fn tick_vram_dma_trigger(&mut self, dot_work: bool, pre_fall_mode: ppu::Mode) {
        if dot_work {
            // The engine thaws at the IF rise, ahead of the CPU's halt-exit
            // latency (a wake-coincident block is decided before the first
            // fetch and the dispatch pick); level re-evaluation and the
            // taken-clear wait for the CPU's own resume. The model owns the
            // trigger pipeline and hands back its committed bus claim.
            self.model.vram_dma_edge(&mut self.chassis, pre_fall_mode);
        }
    }

    /// Fall-path VBlank/STAT IF requests: POPU/SUKO transitions land on the
    /// fall since the divider chain runs there.
    fn request_fall_path_interrupts(
        &mut self,
        video_result: &Option<ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>>,
        standalone_stat: bool,
    ) {
        if let Some(video_result) = video_result {
            // VBlank IF: POPU transitions happen here since the divider
            // chain runs in fall().
            if video_result.request_vblank {
                self.chassis
                    .interrupts
                    .request(Interrupt::VideoBetweenFrames);
            }
            // STAT IF: PPU's two-phase SUKO check (post-advance + post-tick_scan_capture, with
            // TOLU lag modelled via the post-fast snapshot) folds into request_stat.
            // Gated by cpu_irq_ack1_pulse: LALU.r_n=0 absorbs same-M-cycle SUKO rises.
            if video_result.request_stat && !self.chassis.cpu.irq.cpu_irq_ack1_pulse {
                self.chassis.interrupts.request(Interrupt::VideoStatus);
            }
        }
        if standalone_stat && !self.chassis.cpu.irq.cpu_irq_ack1_pulse {
            self.chassis.interrupts.request(Interrupt::VideoStatus);
        }
    }

    /// cpu_irq_ack1 holds the serviced IF bit's r_n LOW across the whole
    /// dispatch-ack window — re-assert it after every same-M-cycle setter
    /// (the FF0F PC-push commit above and the source requests) so a source
    /// rise inside the window is captured-but-suppressed.
    fn reclear_held_ack(&mut self) {
        if let Some(interrupt) = self.chassis.cpu.irq.irq_ack_held {
            self.chassis.interrupts.clear(interrupt);
        }
    }

    /// Apply this fall's PPU result — pixel draw and VSYNC/LCD-off present.
    /// `None` on the double-speed CPU T-cycle that carries no PPU fall.
    fn apply_fall_ppu_result(
        &mut self,
        video_result: Option<&ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>>,
    ) -> (bool, Option<ppu::PixelOutput>) {
        match video_result {
            Some(video_result) => self.apply_ppu_result(video_result),
            None => (false, None),
        }
    }

    /// OAM DMA control gates clock on dma_phi = !data_phase; tick
    /// every master-clock edge so the engage (dma_phi rising) and arm
    /// (dma_phi_n rising) edges are both seen. data_phase is held LOW
    /// during halt-spin, freezing the engine (matu/counter get no edge).
    fn clock_oam_dma_gate(&mut self, tcycle: TCycle) {
        let data_phase = !self.chassis.cpu.halt_rs_latched() && matches!(tcycle.as_u8(), 2 | 3);
        self.drive_dma(data_phase);
    }

    /// M-cycle-boundary CPU work on the rising edge: irq_latched capture,
    /// dispatch update, IME promotion, bus clear, timer/serial mcycle. The
    /// boundary PPU rise follows in the caller via `ppu_rise_edge`.
    fn tick_mcycle_boundary_rise(&mut self) {
        // cpu_irq_ack1↓ at +3.992 dots — hardware releases LALU.r_n
        // ~8 ps before this CLK9↑. Clear at boundary entry so
        // check_stat_edge below sees r_n released.
        self.chassis.cpu.irq.cpu_irq_ack1_pulse = false;
        // On CGB the IF-bit reset trails the boundary's own timer/serial
        // set (tick_cpu_clock_mcycle below), so hold it past that set and
        // release after; DMG releases here, ahead of the set.
        if !M::IRQ_ACK_HOLDS_THROUGH_BOUNDARY_SET {
            self.chassis.cpu.irq.irq_ack_held = None;
        }

        // yoii captures dispatch.latched() before data_phase_n↑ refreshes
        // the per-bit irq_latch — preserves pre-release values held
        // through the prior M-cycle's data phase.
        self.chassis
            .cpu
            .tick_irq_latched(M::HALT_WAKE_SAMPLES_EARLY);

        // data_phase_n↑ reopens the per-bit irq_latch_inst<i> to
        // re-snapshot IF for this M-cycle's dispatch.
        self.chassis.cpu.dispatch.set_data_phase_n(true);
        self.chassis.cpu.dispatch.update_latch(
            self.chassis.interrupts.enabled,
            self.chassis.interrupts.requested,
        );
        self.chassis.cpu.dispatch.tick_zacw();

        // Promote ime_delay (EI's shadow) to ime — produces EI's
        // one-instruction delay.
        self.chassis
            .cpu
            .irq
            .ime
            .write_immediate(if self.chassis.cpu.irq.ime_delay {
                crate::cpu::InterruptMasterEnable::Enabled
            } else {
                crate::cpu::InterruptMasterEnable::Disabled
            });

        self.chassis.cpu_bus.clear_activity();

        self.chassis.ppu.tick_clock_domain_capture();

        self.tick_cpu_clock_mcycle();

        // CGB: the ack reset-hold extends past the boundary set above, so a
        // timer/serial IF assertion coincident with the dispatch boundary is
        // re-cleared before the hold releases.
        if M::IRQ_ACK_HOLDS_THROUGH_BOUNDARY_SET {
            if let Some(interrupt) = self.chassis.cpu.irq.irq_ack_held {
                self.chassis.interrupts.clear(interrupt);
            }
            self.chassis.cpu.irq.irq_ack_held = None;
        }
    }

    /// The CPU-clock peripherals (BOGA M-cycle pulse): the timer divider and
    /// serial shift clock. These are the SM83's own silicon, clocked by the
    /// CPU clock — not by the instruction sequencer. When the SM83 runs, this
    /// rides its M-cycle boundary; through the speed-switch blackout it keeps
    /// pulsing off the master clock while the SM83 is frozen.
    fn tick_cpu_clock_mcycle(&mut self) {
        self.chassis.timers.mcycle();
        if let Some(interrupt) = self.chassis.timers.take_pending_interrupt() {
            self.chassis.interrupts.request(interrupt);
        }

        // Serial bit-5 fall lands IF.serial in this M-cycle's
        // data-phase window for same-M-cycle dispatch.
        let counter = self.chassis.timers.internal_counter();
        if let Some(interrupt) = self
            .chassis
            .serial
            .mcycle(counter, self.model.has_serial_fast_clock())
        {
            self.chassis.interrupts.request(interrupt);
        }
    }

    /// Non-boundary T-cycle rise CPU work: pre-CUPA LCDC snapshot and the
    /// staged write apply at T-cycle 2. The PPU rise + STAT edge follow in
    /// the caller via `ppu_rise_edge`.
    fn tick_non_boundary_rise(&mut self, tcycle: TCycle, edge_carries_dot_fall: bool) {
        // Snapshot LCDC.1 BEFORE the staged write applies — the
        // alet-rising DFF capture (SOBU on TEKY → FEPO → XYLO) beats
        // CUPA-rising's transparent-latch propagation by ~14 ns. Other
        // consumers read post-CUPA `regs` directly.
        self.chassis.ppu.snapshot_pre_cupa_lcdc();

        // Apply staged write at CUPA-rising (T-cycle 2). PPU registers
        // latch combinationally during CUPA-high; memory commits at
        // CUPA-falling in fall().
        if tcycle.as_u8() == 2
            && let Some(address) = self.chassis.cpu_bus.pending_write()
        {
            let value = self
                .chassis
                .cpu
                .pending_bus_write()
                .map(|(_, v)| v)
                .expect("cpu_bus pending write requires cpu.pending_bus_write to be Some");
            self.chassis.cpu_bus.drive(value);
            if self.drive_ppu_bus(address, value, edge_carries_dot_fall) {
                self.chassis.interrupts.request(Interrupt::VideoStatus);
            }
            // Snapshot OAM/VRAM lock at CUPA-rising. AND'd with the
            // mid and commit samples — the write blocks only if locked
            // throughout the entire CUPA window.
            self.chassis
                .cpu_bus
                .record_snapshot_lock(self.chassis.ppu.write_lock(address));
        }
    }

    /// Vector resolve (ISR M3→M4): clear zkog/zloz + the dispatched IF
    /// bit, latch the vector into pc. Reads the priority chain
    /// output (post-latch), matching the IE-push-bug timing.
    fn apply_vector_resolve(&mut self) {
        if self.chassis.cpu.take_pending_vector_resolve() {
            if let Some(interrupt) = self.chassis.cpu.dispatch.vector() {
                self.chassis.interrupts.clear(interrupt);
                self.chassis.cpu.irq.irq_ack_held = Some(interrupt);
                self.chassis.cpu.pc = interrupt.vector();
            } else {
                self.chassis.cpu.pc = 0x0000;
            }
            self.chassis.cpu.dispatch.clear_dispatch();
            // cpu_irq_ack1↑: LALU.r_n driven LOW via lety/movu until next
            // M-cycle boundary. Absorbs same-M-cycle SUKO rises.
            self.chassis.cpu.irq.cpu_irq_ack1_pulse = true;
        }
    }

    /// data_phase_n↓ at T1→T2 and the zkog SR-latch update. Together
    /// they gate this M-cycle's interrupt dispatch visibility.
    fn step_dispatch_logic(&mut self, tcycle: TCycle) {
        // data_phase_n↓ closes the per-bit irq_latch at the T1→T2
        // boundary, freezing IF visibility for this M-cycle's dispatch.
        // The halt-state spin keeps data_phase LOW so the latch stays
        // transparent throughout.
        if tcycle.as_u8() == 2 && !self.chassis.cpu.halt_rs_latched() {
            self.chassis.cpu.dispatch.set_data_phase_n(false);
        }

        // T2 rise: the CGB halt-release chain's sample point.
        if tcycle.as_u8() == 2 && M::HALT_WAKE_SAMPLES_EARLY {
            self.chassis.cpu.presample_halt_wake();
        }

        // step_zkog: zaij = ime ∧ data_phase ∧ int_take ∧ xogs. HALT
        // body and halt-spin both feed into xogs so dispatch can fire
        // mid-HALT for the immediate-dispatch path.
        let halt_body = self.chassis.cpu.is_halted() && !self.chassis.cpu.halt_rs_latched();
        let halt_spin = self.chassis.cpu.halt_rs_latched();
        let data_phase = !halt_spin && (tcycle.as_u8() == 2 || tcycle.as_u8() == 3);
        let write_phase = !halt_spin && tcycle.as_u8() == 3;
        let ctl_fetch = self.chassis.cpu.is_fetch_phase() || halt_body;
        let xogs = (data_phase && ctl_fetch) || halt_spin;
        let ime_enabled =
            self.chassis.cpu.irq.ime.output() == crate::cpu::InterruptMasterEnable::Enabled;
        self.chassis.cpu.dispatch.update_latch(
            self.chassis.interrupts.enabled,
            self.chassis.interrupts.requested,
        );
        self.chassis
            .cpu
            .dispatch
            .step_zkog(ime_enabled, data_phase, write_phase, xogs);
    }

    /// Stage this M-cycle's bus activity. The CPU asserts at most one
    /// of cpu_rd / cpu_wr per M-cycle.
    fn stage_mcycle_bus_activity(&mut self) {
        if let Some((address, _value)) = self.chassis.cpu.pending_bus_write() {
            self.chassis.cpu_bus.stage_write(address);
        } else if let Some(address) = self.chassis.cpu.pending_bus_read() {
            self.chassis.cpu_bus.stage_read(address);
        }
    }

    /// BOWA: arm OAM corruption from any OAM-range address on the CPU
    /// bus this M-cycle. CUFE fires at MOPA (T-cycle 2 rise); arming
    /// must be visible at T-cycle 0 so the same M-cycle's MOPA edge
    /// picks it up.
    fn arm_oam_bugs(&mut self) {
        if let BusAction::InternalOamBug { address } = self.chassis.cpu.last_bus_action {
            self.chassis.ppu.arm_oam_bug_for_write(address);
        }
        if let Some(address) = self.chassis.cpu.pending_bus_read() {
            self.chassis.ppu.arm_oam_bug_for_read(address);
        }
        if let Some((address, _)) = self.chassis.cpu.pending_bus_write() {
            self.chassis.ppu.arm_oam_bug_for_write(address);
        }
    }

    /// Driver-enable edge (tobe↑ / wafu↑) at T-cycle 2: the addressed
    /// peripheral opens its tri-state driver. Mid-M-cycle flux
    /// propagates combinationally to the latch edge in `commit_read_latch`.
    fn apply_read_drive_enable(&mut self) {
        if let Some(address) = self.chassis.cpu_bus.pending_read() {
            let value = self.bus_value_at_drive_enable(address);
            // OAM read lock at the drive enable: the grant view tobe↑ samples
            // before this fall's PPU advance applies any lock onset.
            if let 0xFE00..=0xFEFF = address {
                self.model
                    .note_read_drive_phase(self.chassis.ppu.read_lock(address));
            }
            self.chassis.cpu_bus.drive(value);

            // A VRAM-source bus conflict on a read forces the DMA's OAM deposit
            // to $00, same as on a write.
            if self.chassis.dma.is_active_on_bus().is_some()
                && self
                    .model
                    .oam_dma_conflict_zeroes_oam(address, self.chassis.dma.source())
                && let Some((_, dst_offset)) = self.chassis.dma.peek_transfer()
            {
                self.model
                    .console_state_mut()
                    .set_dma_conflict_oam_zero(Some(dst_offset));
            }
        }
    }

    /// Mid-CUPA lock sample: catches the AJUJ-glitch window where AVAP
    /// ends mode-2 mid-strobe and the rendering deferral leaves
    /// `mode2=0 ∧ mode3=0` observable here.
    fn sample_mid_cupa_lock(&mut self) {
        if let Some(address) = self.chassis.cpu_bus.mid_sample_pending() {
            // The double-speed write-lock follows this mid sample; it counts only
            // the genuine mode lock, not the RUTU pre-onset that the single-speed
            // window's later samples already exclude.
            let locked = if self.double_speed_active() && matches!(address, 0xFE00..=0xFEFF) {
                Some(self.chassis.ppu.oam_mode_locked())
            } else {
                self.chassis.ppu.write_lock(address)
            };
            self.chassis.cpu_bus.record_mid_lock(locked);
        }
    }

    /// CPU data latch (data_phase_n↑ near the end of T-cycle 3).
    /// Resolves the drive-enable snapshot against mid-M-cycle flux
    /// before the SM83 captures cpu_port_d.
    fn commit_read_latch(&mut self, ly_at_latch: Option<u8>) {
        if let BusAction::Read { address } = &self.chassis.cpu.last_bus_action {
            let address = *address;
            // Double speed: the LY tick can land mid-M on the read's own dot
            // fall (no CPU fall carries it), so the ripple LY_old arrives from
            // the tick edge instead of the pre-fall sample.
            let ly_at_latch = if address == 0xFF44 {
                self.model.take_ff44_ripple_old().or(ly_at_latch)
            } else {
                ly_at_latch
            };
            // A lockable read is offered the unfloated accessible byte; the
            // model owns the float decision from its latch lock view. Other
            // addresses resolve through `bus_value_at_latch`.
            let latch_lock = self.chassis.ppu.read_lock(address);
            let accessible = if latch_lock.is_some() {
                self.chassis.cpu_bus.data
            } else {
                self.bus_value_at_latch(address, self.chassis.cpu_bus.data, ly_at_latch)
            };
            let value = if let Some(source) = self.model.vram_dma_conflict_source(address) {
                self.read_dma_source(source)
            } else {
                self.model
                    .resolve_read_latch(address, accessible, latch_lock)
            };
            // Mode-3 onset (XYMU↓ at AVAP↑) bus-settle, the symmetric counterpart to
            // the mode-2 not_if1 hold: a double-speed STAT read landing in the onset
            // contention window holds the XYMU bit at its pre-onset 0 (PRE mode 2).
            let value = if address == 0xFF41 && self.double_speed_active() {
                if self.chassis.ppu.in_mode3_onset_settle() {
                    (value & !0b11) | self.chassis.ppu.mode3_onset_pre_stat()
                } else if self.chassis.ppu.in_mode1_onset_settle() {
                    value & !0b01
                } else {
                    value
                }
            } else {
                value
            };
            // OAM read-lock onset hold (RUTU↑ before ACYL settles the gate closed): a
            // double-speed OAM read landing in the onset window reads accessible — the
            // OAM analogue of the not_if1 hold the bare OAM gate lacks.
            let value = if matches!(address, 0xFE00..=0xFEFF)
                && self.double_speed_active()
                && self.chassis.ppu.in_oam_onset_settle()
            {
                accessible
            } else {
                value
            };
            self.chassis.cpu.data_latch = value;
            // A next-opcode overlap prefetch that latched after a GDMA seized the
            // bus keeps its byte: it read the pre-transfer value (the transfer
            // suppresses the fetch, it does not re-drive the read). Retain it so
            // the post-hold re-fetch decodes it instead of the open-bus re-read.
            if self.model.console_state().dma_cpu_hold() && self.chassis.cpu.bus_hold_over_prefetch
            {
                self.chassis.cpu.held_overlap_opcode = Some(value);
                self.chassis.cpu.bus_hold_over_prefetch = false;
            }
            self.commit_bus_read(address, value);
        }
    }

    /// CPU writes commit at CUPA-falling (end of T-cycle 3). PPU
    /// registers were already written at CUPA-rising via
    /// `drive_ppu_bus` in rise(); this commits memory.
    fn commit_write(&mut self) {
        if let BusAction::Write { address, value: _ } = &self.chassis.cpu.last_bus_action {
            let address = *address;
            if self.chassis.dma.is_active_on_bus().is_some()
                && self
                    .model
                    .oam_dma_source_bank_write(address, self.chassis.dma.source())
            {
                self.chassis.dma_conflict.pending_bank_write = Some(crate::DmaBankWrite {
                    address,
                    value: self.chassis.cpu_bus.data,
                });
                return;
            }
            let (locked_at_snapshot, locked_at_mid) = self.chassis.cpu_bus.write_lock_samples();
            self.write_byte_with_cupa_lock(
                address,
                self.chassis.cpu_bus.data,
                locked_at_snapshot,
                locked_at_mid,
            );
        }
    }

    /// M-cycle-boundary work on the falling edge (data phase): commit the
    /// OAM DMA byte for this M-cycle, plus external-bus decay. A CPU write
    /// that collided with DMA on the source bus open-drains at the OAM
    /// slot DMA deposits. (Audio mcycle is at boundary rise.)
    fn tick_mcycle_boundary_fall(&mut self) {
        let oam = self.chassis.dma.peek_transfer();
        // The CGB VRAM DMA arbitrates the shared bus before the OAM byte moves:
        // it may take this M-cycle's OAM deposit (single-speed contention) or
        // stall the OAM engine (a double-speed switch-cancel escape byte). DMG:
        // never suppresses.
        let suppress_oam = self.model.vram_dma_arbitrate_oam(&mut self.chassis);
        if !suppress_oam {
            if let Some((src_addr, dst_offset)) = oam {
                self.dma_move(src_addr, 0xfe00 + dst_offset as u16);
            }
        }

        // A source-bank register write (VBK/SVBK) latches here at the boundary,
        // after the coincident byte's source read above reads the pre-write
        // bank. Reuses the CPU write-commit path (map_write); no-op on the DMG.
        if let Some(crate::DmaBankWrite { address, value }) =
            self.chassis.dma_conflict.pending_bank_write.take()
        {
            self.write_byte_with_cupa_lock(address, value, None, None);
        }

        // The CGB VRAM-DMA byte engine: moves this M-cycle's bytes while it holds
        // the bus and deposits the contended byte at OAM. No-op on the DMG.
        self.model.vram_dma_boundary(&mut self.chassis);

        if let Some(crate::DmaConflictWrite {
            oam_offset,
            src_byte,
            cpu_value,
        }) = self.chassis.dma_conflict.pending_write.take()
        {
            let dst_addr = 0xfe00 + oam_offset as u16;
            let oam_addr = match ppu::memory::MappedAddress::map(dst_addr) {
                ppu::memory::MappedAddress::Oam(addr) => addr,
                _ => unreachable!(),
            };
            let value = self.model.oam_dma_write_conflict_byte(
                src_byte,
                cpu_value,
                self.chassis.dma.source(),
            );
            self.chassis.ppu.write_oam(oam_addr, value);
            self.chassis.bus_trace.record(BusAccess {
                address: dst_addr,
                value,
                kind: BusAccessKind::Write,
            });
        }

        if let Some(dst_offset) = self.model.console_state_mut().take_dma_conflict_oam_zero() {
            let dst_addr = 0xfe00 + dst_offset as u16;
            if let ppu::memory::MappedAddress::Oam(oam_addr) =
                ppu::memory::MappedAddress::map(dst_addr)
            {
                self.chassis.ppu.write_oam(oam_addr, 0);
                self.chassis.bus_trace.record(BusAccess {
                    address: dst_addr,
                    value: 0,
                    kind: BusAccessKind::Write,
                });
            }
        }

        self.chassis.external.tick_decay();
        // The RTC crystal is speed-independent: 4 base dots per M-cycle at
        // single speed, 2 at double speed.
        self.chassis
            .external
            .tick_rtc(4 / self.model.cpu_steps_per_dot() as u32);
    }

    /// Advance the OAM-DMA control gates one master-clock edge (engage/
    /// release/counter). The byte transfer itself commits at the M-cycle
    /// data phase in `tick_mcycle_boundary_fall`.
    fn drive_dma(&mut self, data_phase: bool) {
        let master_edge = self.chassis.clock.master_edge();
        self.chassis.dma.tick(data_phase, master_edge);
    }

    /// Re-capture interrupt state after bus writes and M-cycle
    /// subsystems so IF mutations from CPU writes to 0xFF0F, STAT
    /// edges from PPU register writes, and serial completion are all
    /// visible by the time the next rise() ticks irq_latched.
    fn recapture_interrupts(&mut self) {
        let triggered = self.chassis.interrupts.triggered();
        self.chassis.cpu.update_interrupt_state(triggered);
        self.chassis.cpu.dispatch.update_latch(
            self.chassis.interrupts.enabled,
            self.chassis.interrupts.requested,
        );
    }

    /// Process a PPU tick: draw the pixel, present on VSYNC (only if
    /// MEDA has pulsed since LCD-on), blank on LCD-off. Returns
    /// `(new_screen, pixel)` — `new_screen` fires only on VSYNC, never
    /// on LCD-off blank.
    fn apply_ppu_result(
        &mut self,
        result: &ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>,
    ) -> (bool, Option<ppu::PixelOutput>) {
        let trace_pixel = result.pixel.map(|pixel| {
            if pixel.x < ppu::screen::PIXELS_PER_LINE && pixel.y < ppu::screen::NUM_SCANLINES {
                self.chassis
                    .screen
                    .draw_pixel(pixel.x, pixel.y, pixel.color);
            }
            ppu::PixelOutput {
                x: pixel.x,
                y: pixel.y,
                shade: <M::Ppu as ppu::PpuModel>::trace_shade(pixel.color),
            }
        });
        if result.new_frame {
            if self.chassis.ppu.control().video_enabled() && self.chassis.ppu.vsync_committed() {
                self.chassis.screen.present();
                self.model.on_present(&self.chassis.screen);
            }
            return (true, trace_pixel);
        }
        if result.lcd_disabled {
            self.chassis.screen.blank();
            self.model.on_present(&self.chassis.screen);
        }
        (false, trace_pixel)
    }
}
