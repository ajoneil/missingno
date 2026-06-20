use super::{
    Console, ConsoleShadow, Model, ScreenBuffer, StopAction,
    clock::{CpuDivider, CpuGate, Edge},
    cpu::mcycle::{BusAction, TCycle},
    cpu_bus::{BusAccess, BusAccessKind},
    interrupts::Interrupt,
    memory::Bus,
    ppu::{self, memory::Vram},
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

/// Which PPU master-clock edge a CPU edge carries. The PPU clock is
/// speed-independent: one rise and one fall per dot. At single speed they sit
/// on the CPU's own rise and fall; at double speed the dot's master rise lands
/// on the first CPU edge and the master fall half a dot later — the second CPU
/// T-cycle's rise — not the dot's final edge.
#[derive(Clone, Copy, PartialEq)]
enum PpuEdge {
    None,
    Rise,
    Fall,
}

impl<M: Model> Console<M> {
    pub fn step(&mut self) -> StepResult {
        self.step_traced(false).0
    }

    /// Step one instruction, optionally recording all bus accesses.
    /// Returns (result, trace). Trace is empty when `trace` is false.
    pub fn step_traced(&mut self, trace: bool) -> (StepResult, Vec<BusAccess>) {
        if trace {
            self.bus_trace.enable();
        }

        // If step_tcycle() left us mid-instruction, drain to the next
        // boundary first, then run one full instruction.
        let mut new_screen = false;
        let mut tcycles = 0u32;
        if !self.cpu.at_instruction_boundary() {
            let r = self.step_instruction();
            new_screen |= r.new_screen;
            tcycles += r.tcycles;
        }
        let r = self.step_instruction();
        new_screen |= r.new_screen;
        tcycles += r.tcycles;

        self.resolve_stop(tcycles);
        self.manage_dma_hold();

        let sram_dirty = self.external.cartridge.take_sram_dirty();
        (
            StepResult {
                new_screen,
                sram_dirty,
                tcycles,
            },
            self.bus_trace.take(),
        )
    }

    /// Run one complete instruction from start to finish.
    ///
    /// Runs phases until the CPU returns to the Fetch phase at a fresh
    /// M-cycle boundary (instruction boundary). At that point, EI delay
    /// is advanced and control returns to the caller.
    fn step_instruction(&mut self) -> StepResult {
        let mut new_screen = false;
        self.cpu.data_latch = 0;

        // Consume the current instruction boundary (we're starting
        // from a boundary — we want to run until the NEXT one).
        self.cpu.take_instruction_boundary();

        // Speed-switch blackout: the CPU clock is held while the dot clock
        // keeps running. Drive one CPU M-cycle of held master edges through the
        // same `execute_phase` loop (the gate is `Held`, the CPU frozen) and
        // return, draining the blackout across step()s until the count empties.
        if self.cpu.is_stopped() && self.model.speed_switch_in_progress() {
            return self.step_blackout_chunk();
        }

        const PHASE_BUDGET: u32 = 800;
        let mut phases_remaining = PHASE_BUDGET;
        let mut tcycles = 0u32;

        loop {
            assert!(
                phases_remaining > 0,
                "step() exceeded {PHASE_BUDGET} phase budget — possible infinite loop in CPU"
            );
            phases_remaining -= 1;

            let result = self.execute_phase(CpuGate::Running);
            new_screen |= result.new_screen;

            // A T-cycle completes every two CPU edges, at the return to a rise.
            if self.clock.cpu_edge() == Edge::Rise {
                tcycles += 1;
                if self.cpu.at_instruction_boundary() {
                    break;
                }
            }
        }
        // Don't drain sram_dirty here — let the caller (step_traced) do it
        // so the flag accumulates across multiple step_instruction calls.
        let sram_dirty = self.external.cartridge.sram_dirty;
        StepResult {
            new_screen,
            sram_dirty,
            tcycles,
        }
    }

    /// Advance exactly one half-phase — execute rise() or fall()
    /// depending on current clock level.
    pub fn step_phase(&mut self) -> PhaseResult {
        self.execute_phase(CpuGate::Running)
    }

    /// Advance to the next T-cycle boundary — the next Low state.
    /// Executes 1 phase if clock is High, 2 if Low. Returns true if
    /// a new frame was produced.
    pub fn step_tcycle(&mut self) -> bool {
        let mut new_screen = false;

        // Run phases until the clock returns to a rise (T-cycle complete)
        loop {
            let result = self.execute_phase(CpuGate::Running);
            new_screen |= result.new_screen;
            if self.clock.cpu_edge() == Edge::Rise {
                break;
            }
        }

        // Consume the boundary flag so step_traced sees mid-instruction
        // state after this returns.
        self.cpu.take_instruction_boundary();

        new_screen
    }

    /// Advance the machine by one master edge under a CPU-clock gate. The CPU and
    /// PPU are separate state machines on the one master clock; `MasterClock`
    /// owns both phases. At ÷1 (single speed) the CPU and dot edges coincide; at
    /// ÷2 (double speed) the dot edge lands only on the CPU rise edges. The
    /// `Held` gate (the speed-switch blackout) freezes the CPU phase while the
    /// dot domain free-runs, so the post-switch alignment emerges from the held
    /// count rather than a fixed map.
    fn execute_phase(&mut self, gate: CpuGate) -> PhaseResult {
        let double_speed = self.model.cpu_steps_per_dot() == 2;
        // The ÷1/÷2 divider is the model's ratio; KEY1 mutates it through the
        // speed switch, which drains the blackout and re-engages on a CPU rise,
        // so the cpu_phase_in_dot==0 ⟺ cpu_phase==Rise invariant the dispatch
        // relies on is restored before the next running edge.
        self.clock.set_divider(if double_speed {
            CpuDivider::Two
        } else {
            CpuDivider::One
        });
        // The pre-advance dot phase — the fall arm's standalone dot_work read of
        // the dot domain's current edge.
        let dot_phase_before = self.clock.dot_phase();
        let master_edge_before = self.clock.master_edge();
        let tick = self.clock.advance(gate);
        // A held edge: the CPU is frozen and the dot domain alone advanced.
        if tick.cpu.is_none() {
            let dot = tick.dot.expect("a held edge always carries a dot edge");
            // Correctness relies on no Running edge falling between arming the
            // blackout anchor and draining it: the elapsed count is the
            // pre-advance anchor difference, the held edges already completed.
            let held_elapsed =
                master_edge_before.wrapping_sub(self.model.console_state().blackout_anchor());
            return self.held_dot_advance(dot, held_elapsed);
        }
        let ppu = match tick.dot {
            Some(Edge::Rise) => PpuEdge::Rise,
            Some(Edge::Fall) => PpuEdge::Fall,
            None => PpuEdge::None,
        };
        // Per-dot master-clock work rides the PPU edges: the APU tick on the PPU
        // rise, the CH3 fall-sync / HDMA trigger on the PPU fall (which at double
        // speed lands on a CPU rise, so its work runs on the following CPU fall).
        let (new_screen, pixel) = match tick.cpu {
            Some(Edge::Rise) => {
                let dot_work = ppu == PpuEdge::Rise;
                // The PPU rise is its own domain's edge, sequenced here between the
                // CPU's pre- and post-rise work rather than welded inside it.
                let (is_mcycle_boundary, ppu_tcycle) = self.rise_cpu_pre(ppu, dot_work);
                let edge = self.fire_dot_ppu(ppu, is_mcycle_boundary, ppu_tcycle);
                self.rise_cpu_post(is_mcycle_boundary, ppu_tcycle);
                edge
            }
            Some(Edge::Fall) => {
                // The double-speed fall that shares a dot with no PPU fall still
                // does dot_work when this is the dot's bare second CPU edge — i.e.
                // the next dot edge to fire is the dot's rise. The dot phase
                // toggles lazily (after the second sub-edge), so a pending dot
                // rise reads as the held phase being `Fall`.
                let dot_work =
                    ppu == PpuEdge::Fall || (double_speed && dot_phase_before == Edge::Fall);
                // The PPU fall is its own domain's edge, sequenced here between the
                // CPU's pre- and post-fall work rather than welded inside it.
                let (tcycle, is_mcycle_boundary, ly_at_latch, pre_fall_mode) =
                    self.fall_cpu_pre(dot_work);
                let video_result = if ppu == PpuEdge::Fall {
                    Some(self.ppu_fall_edge(is_mcycle_boundary, tcycle))
                } else {
                    None
                };
                self.fall_cpu_post(
                    tcycle,
                    is_mcycle_boundary,
                    ly_at_latch,
                    pre_fall_mode,
                    video_result,
                    dot_work,
                )
            }
            // The held edge was dispatched above; a running edge always carries a
            // CPU edge.
            None => unreachable!("running edge carries a CPU edge"),
        };
        // The dot domain advanced this edge iff a dot edge fired — the divider's
        // `cpu_phase_in_dot==0`. `advance` already toggled
        // both phases; only the mode-2 settle ride stays here.
        if tick.dot.is_some() {
            self.ppu.tick_stat_mode2_settle();
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
        if !self.cpu.is_stopped() {
            return;
        }

        // The settle is bus-coupled: a bus master holding the CPU defers it.
        if self.cpu.bus_held {
            return;
        }

        // Mid-blackout: `step_blackout_chunk` owns the countdown and the
        // re-engage. Nothing to arm again until it expires.
        if self.model.speed_switch_in_progress() {
            return;
        }

        match self.model.resolve_stop() {
            StopAction::SpeedSwitch => {
                // Hardware resets DIV across the switch (the model has already
                // toggled its speed bit and armed the blackout count). The CPU
                // clock is then held while the dot clock runs the blackout out;
                // `step_blackout_chunk` advances the master clock every edge and
                // re-engages at the phase the count expires on.
                let old_counter = self.timers.internal_counter();
                self.timers.reset_for_speed_switch();
                self.audio.on_div_write(old_counter, false);
                if let Some(interrupt) = self
                    .serial
                    .on_div_write(old_counter, self.model.has_serial_fast_clock())
                {
                    self.interrupts.request(interrupt);
                }
                // Anchor the held-edge count at the current master edge; the
                // blackout's elapsed count is `master_edge - blackout_anchor`.
                let anchor = self.clock.master_edge();
                self.model.console_state_mut().set_blackout_anchor(anchor);
            }
            StopAction::Remain => {}
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
        if self.cpu.bus_suspended || self.cpu.in_dispatch() {
            return;
        }
        let holds = self.model.vram_dma_holds_cpu();
        let held = self.model.console_state().dma_cpu_hold();
        if holds && !held {
            self.model.console_state_mut().set_dma_cpu_hold(true);
            self.cpu.begin_bus_hold();
        } else if !holds && held {
            self.model.console_state_mut().set_dma_cpu_hold(false);
            self.cpu.end_bus_hold();
        }
    }

    /// Move one DMA byte: read the bus source, write the mapped destination
    /// (OAM or the VBK-selected VRAM bank), trace both, decay the source bus.
    /// The single byte-transfer OAM DMA and the CGB VRAM DMA share.
    fn dma_move(&mut self, source: u16, dest: u16) {
        let byte = self.read_dma_source(source);
        match ppu::memory::MappedAddress::map(dest) {
            ppu::memory::MappedAddress::Oam(address) => self.ppu.write_oam(address, byte),
            ppu::memory::MappedAddress::Vram(address) => {
                self.vram_bus.vram.cpu_write(address, byte)
            }
        }
        self.bus_trace.record(BusAccess {
            address: source,
            value: byte,
            kind: BusAccessKind::DmaRead,
        });
        self.bus_trace.record(BusAccess {
            address: dest,
            value: byte,
            kind: BusAccessKind::DmaWrite,
        });
        match Bus::of(source) {
            Some(Bus::External) => self.external.drive(byte),
            Some(Bus::Vram) => self.vram_bus.drive(byte),
            None => {}
        }
    }

    /// CPU work for a rising master-clock edge, optionally carrying a PPU edge.
    /// The CPU's per-rise advance shared by both boundary paths: the T-cycle
    /// step, vector resolve at T3, dispatch logic, and the APU prescaler tick.
    /// Runs before the PPU rise off a boundary, after it on an M-boundary.
    fn rise_cpu_advance(&mut self, dot_work: bool) -> TCycle {
        self.cpu.next_tcycle();
        // cpu_irq_ack1↑ at +2.993 dots into the dispatching M-cycle —
        // tcycle 3 rise in our half-phase resolution. Deferring to
        // tcycle 3 also lets M4's bus write commit (tcycle 2 fall)
        // before vector resolution reads IE (IE-push-bug semantics).
        if self.cpu.last_tcycle().as_u8() == 3 {
            self.apply_vector_resolve();
        }

        let tcycle = self.cpu.last_tcycle();
        self.step_dispatch_logic(tcycle);

        // APU prescaler tick (apuv ↑) on every master-clock rise.
        if dot_work {
            let double_speed = self.model.cpu_steps_per_dot() == 2;
            self.audio.tcycle(
                self.timers.internal_counter(),
                tcycle.as_u8(),
                double_speed,
                M::WAVE_RAM_COUPLING,
            );
        }
        tcycle
    }

    /// All CPU work on a rising edge before its PPU rise, plus the T-cycle the
    /// PPU edge is keyed to. The PPU rise fires after the T-cycle advance on
    /// every dot — one consistent CPU↔PPU phase (the spec pins a single fixed
    /// lattice; there is no per-dot CPU edge for it to vary against). The
    /// M-boundary additionally runs its boundary CPU work and the HDMA grant.
    fn rise_cpu_pre(&mut self, ppu: PpuEdge, dot_work: bool) -> (bool, TCycle) {
        let is_mcycle_boundary = self.cpu.consume_boundary_pending();

        // Pre-ALET-rise XYMU (mode-3) view: the mode 3→0 XYMU.q↑ fires inside
        // this dot's `ppu_rise_edge`. A double-speed FF41 read latching on the
        // same phase resolves its mode to this pre-transition view (the CGB
        // CPU↔ALET read placement). Only double speed consumes it.
        if ppu == PpuEdge::Rise && self.model.cpu_steps_per_dot() == 2 {
            self.model.note_pre_alet_rendering(self.ppu.is_rendering());
        }

        let tcycle = if is_mcycle_boundary {
            self.tick_mcycle_boundary_rise();
            self.audio.mcycle_boundary();
            // The HDMA grant is M-boundary-quantized: bus ownership asserts and
            // releases between M-cycles only. A dispatch sequence already in
            // flight when the transfer became ready holds the bus through its
            // M-cycles (the grant defers); a dispatch starting with the transfer
            // ready parks behind the block. Granted ownership is never revoked.
            self.cpu.bus_suspended = self.model.vram_dma_seizes_bus()
                && (self.cpu.bus_suspended || !self.cpu.in_dispatch());
            let tcycle = self.rise_cpu_advance(dot_work);
            self.stage_mcycle_bus_activity();
            tcycle
        } else {
            self.rise_cpu_advance(dot_work)
        };

        if M::HAS_OAM_BUG && tcycle.as_u8() == 0 {
            self.arm_oam_bugs();
        }
        if !is_mcycle_boundary {
            self.tick_non_boundary_rise(tcycle);
        }
        (is_mcycle_boundary, tcycle)
    }

    /// CPU work on a rising edge after its PPU rise: off a boundary the dispatch
    /// latch update; an armed OAM bug fires last on both paths.
    fn rise_cpu_post(&mut self, is_mcycle_boundary: bool, ppu_tcycle: TCycle) {
        if !is_mcycle_boundary {
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);
        }

        // MOPA-rising fires any armed OAM bug.
        if M::HAS_OAM_BUG && ppu_tcycle.as_u8() == 2 {
            self.ppu.apply_pending_oam_bug();
        }
    }

    /// PPU rising-edge advance and its interrupt readback: pixel output,
    /// VBlank IF, the STAT edge, and the CPU's interrupt-state refresh.
    fn ppu_rise_edge(&mut self) -> (bool, Option<ppu::PixelOutput>) {
        let oam_bus = self.dma.oam_bus_owner();
        let ppu_result = self.ppu.on_master_clock_rise(&self.vram_bus.vram, oam_bus);
        if ppu_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }
        let (new_screen, pixel) = self.apply_ppu_result(&ppu_result);
        if self.ppu.check_stat_edge() {
            self.interrupts.request(Interrupt::VideoStatus);
        }
        let triggered = self.interrupts.triggered();
        self.cpu.update_interrupt_state(triggered);
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
        let oam_bus = self.dma.oam_bus_owner();
        // The M-cycle's last PPU fall, where the WY/WX/LCDC.5/LCDC.2 crossing
        // captures — resolved by the divider cell from the ratio.
        let mcycle_last_fall = self
            .clock
            .divider()
            .mcycle_last_fall(is_mcycle_boundary, tcycle.as_u8());
        self.ppu
            .on_master_clock_fall(is_mcycle_boundary, mcycle_last_fall, oam_bus)
    }

    /// Apply a PPU fall's outputs: VBlank/STAT IF requests and the pixel/screen
    /// commit. The `cpu_irq_ack1` re-assert is the caller's (it runs on every
    /// CPU fall, not only the dot's PPU fall).
    fn apply_ppu_fall(
        &mut self,
        video_result: &ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>,
    ) -> (bool, Option<ppu::PixelOutput>) {
        // VBlank IF: POPU transitions happen on the fall since the divider
        // chain runs there.
        if video_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }
        // STAT IF: the SUKO check folds into request_stat; cpu_irq_ack1_pulse
        // (LALU.r_n=0) absorbs same-M-cycle SUKO rises.
        if video_result.request_stat && !self.cpu.irq.cpu_irq_ack1_pulse {
            self.interrupts.request(Interrupt::VideoStatus);
        }
        self.apply_ppu_result(video_result)
    }

    /// Run one CPU M-cycle of the speed-switch blackout through the main
    /// `execute_phase` loop with the CPU clock `Held`. Returns when the divider
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
            let froze_on = self.clock.dot_phase();
            new_screen |= self.execute_phase(CpuGate::Held { froze_on }).new_screen;
            edges += 1;
            if !self.cpu.is_stopped() {
                // The count emptied this edge and the CPU re-engaged; its first
                // fetch runs on the next step()'s normal loop.
                break;
            }
        }

        self.cpu.mark_instruction_boundary();
        StepResult {
            new_screen,
            sram_dirty: self.external.cartridge.sram_dirty,
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
        let double_speed = self.model.cpu_steps_per_dot() == 2;
        let steps_per_dot = self.model.cpu_steps_per_dot() as u64;
        let mcycle_edges = (8 / steps_per_dot).max(1);

        // The M-cycle phase, derived from elapsed master edges so it pulses
        // at the CPU rate independent of the frozen SM83.
        let mcycle_boundary = elapsed % mcycle_edges == 0;

        // The divider/STAT crossing run at the CPU rate through the hold but
        // freeze during the clock-mux relock tail (the CPU clock is settling),
        // so the re-phase tail advances the PPU without disturbing DIV.
        if mcycle_boundary && self.model.speed_switch_divider_active() {
            self.ppu.tick_clock_domain_capture(double_speed);
            self.tick_cpu_clock_mcycle();
        }

        let (new_screen, pixel) = match dot {
            Edge::Rise => {
                let r = self.ppu_rise_edge();
                self.audio.tcycle(
                    self.timers.internal_counter(),
                    0,
                    double_speed,
                    M::WAVE_RAM_COUPLING,
                );
                r
            }
            Edge::Fall => {
                let video_result = self.ppu_fall_edge(mcycle_boundary, TCycle::ZERO);
                self.audio.fall_sync();
                self.apply_ppu_fall(&video_result)
            }
        };

        // One master edge of the blackout spent; re-engage the moment it empties.
        if self.model.drain_speed_switch_blackout(1) {
            self.cpu.resume_from_stop();
            // The fetch begins on a CPU rising edge.
            self.clock.engage_on_rise();
        }

        PhaseResult { new_screen, pixel }
    }

    /// Run the PPU edge a CPU edge carries (if any). The rise outputs pixel +
    /// VBlank/STAT-edge IF; the fall runs the divider chain and applies its
    /// outputs. Double speed places the master fall on the High arm's rise.
    fn fire_dot_ppu(
        &mut self,
        ppu: PpuEdge,
        is_mcycle_boundary: bool,
        tcycle: TCycle,
    ) -> (bool, Option<ppu::PixelOutput>) {
        match ppu {
            PpuEdge::None => (false, None),
            PpuEdge::Rise => self.ppu_rise_edge(),
            PpuEdge::Fall => {
                let video_result = self.ppu_fall_edge(is_mcycle_boundary, tcycle);
                self.apply_ppu_fall(&video_result)
            }
        }
    }

    /// CPU work on a falling edge before its PPU fall: CH3 wave-latch sync, the
    /// T2 read drive-enable, the pre-edge LY sample, and the pre-fall mode the
    /// HDMA trigger reads. The PPU fall is the caller's, sequenced after this.
    fn fall_cpu_pre(&mut self, dot_work: bool) -> (TCycle, bool, Option<u8>, ppu::Mode) {
        let tcycle = self.cpu.last_tcycle();
        let is_mcycle_boundary = self.cpu.at_mcycle_boundary();

        // CH3's BUSA / AZUS DFFs latch on apu_4mhz ↑ (= our fall);
        // settle before the T=2 drive-enable so wave-RAM reads see
        // the current wave_data_latch.
        if dot_work {
            self.audio.fall_sync();
        }

        if tcycle.as_u8() == 2 {
            self.apply_read_drive_enable();
        }

        // data_phase_n↑ precedes this fall's edge: sample LY pre-edge so an
        // FF44 latch coincident with the RUTU-clocked capture resolves the
        // mid-ripple flux.
        let ly_at_latch = match self.cpu.last_bus_action {
            BusAction::Read { address: 0xFF44 } => Some(self.read(0xFF44)),
            _ => None,
        };

        let pre_fall_mode = self.ppu.mode();

        (tcycle, is_mcycle_boundary, ly_at_latch, pre_fall_mode)
    }

    /// CPU work on a falling edge after its PPU fall: STAT-sync capture, the
    /// read latch and write commit, the HDMA trigger, the fall path's IF
    /// requests, and the DMA/timer ticks. `video_result` is the PPU fall's
    /// output, `None` on the double-speed CPU T-cycle that carries no PPU fall.
    fn fall_cpu_post(
        &mut self,
        tcycle: TCycle,
        is_mcycle_boundary: bool,
        ly_at_latch: Option<u8>,
        pre_fall_mode: ppu::Mode,
        video_result: Option<ppu::PpuTickResult<<M::Ppu as ppu::PpuModel>::Pixel>>,
        dot_work: bool,
    ) -> (bool, Option<ppu::PixelOutput>) {
        let mut new_screen = false;
        let mut pixel = None;
        // Double-speed boundary fall sharing a dot with no PPU fall: the
        // CPU-clocked STAT register synchroniser still captures; its request
        // joins the fall path's gating below.
        let standalone_stat = video_result.is_none()
            && is_mcycle_boundary
            && self.ppu.capture_register_sync_standalone();

        if tcycle.as_u8() == 2 {
            self.sample_mid_cupa_lock();
        }

        self.commit_read_latch(ly_at_latch);
        self.commit_write();

        // HDMA trigger, evaluated each dot's fall with this fall's write
        // commit visible: the pend forms on the post-rise mode view and
        // commits to cancel-immunity one fall later (the pend pipeline
        // lives in the model).
        if dot_work {
            // The engine thaws at the IF rise, ahead of the CPU's halt-exit
            // latency (a wake-coincident block is decided before the first
            // fetch and the dispatch pick); level re-evaluation and the
            // taken-clear wait for the CPU's own resume.
            let cpu_halted = self.cpu.is_halted();
            let engine_gated = (cpu_halted && !self.cpu.irq_latched()) || self.cpu.is_stopped();
            let claim = self
                .model
                .vram_dma_tick(pre_fall_mode, engine_gated, cpu_halted);
            if claim.committed {
                // An active OAM DMA already owns a bus, blocking the
                // handover that would take the halt-release fetch's tail.
                let bus_free = self.dma.is_active_on_bus().is_none();
                self.cpu.vram_dma_claim = crate::VramDmaClaim {
                    committed: true,
                    standing: claim.standing && bus_free,
                };
            }
        }

        if let Some(video_result) = &video_result {
            // VBlank IF: POPU transitions happen here since the divider
            // chain runs in fall().
            if video_result.request_vblank {
                self.interrupts.request(Interrupt::VideoBetweenFrames);
            }
            // STAT IF: PPU's two-phase SUKO check (post-advance + post-tick_scan_capture, with
            // TOLU lag modelled via the post-fast snapshot) folds into request_stat.
            // Gated by cpu_irq_ack1_pulse: LALU.r_n=0 absorbs same-M-cycle SUKO rises.
            if video_result.request_stat && !self.cpu.irq.cpu_irq_ack1_pulse {
                self.interrupts.request(Interrupt::VideoStatus);
            }
        }
        if standalone_stat && !self.cpu.irq.cpu_irq_ack1_pulse {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        // cpu_irq_ack1 holds the serviced IF bit's r_n LOW across the whole
        // dispatch-ack window — re-assert it after every same-M-cycle setter
        // (the FF0F PC-push commit above and the source requests) so a source
        // rise inside the window is captured-but-suppressed.
        if let Some(interrupt) = self.cpu.irq.irq_ack_held {
            self.interrupts.clear(interrupt);
        }

        if let Some(video_result) = &video_result {
            let (ns, px) = self.apply_ppu_result(video_result);
            new_screen |= ns;
            pixel = px;
        }

        // OAM DMA control gates clock on dma_phi = !data_phase; tick
        // every master-clock edge so the engage (dma_phi rising) and arm
        // (dma_phi_n rising) edges are both seen. data_phase is held LOW
        // during halt-spin, freezing the engine (matu/counter get no edge).
        let data_phase = !self.cpu.halt_rs_latched() && matches!(tcycle.as_u8(), 2 | 3);
        self.drive_dma(data_phase);

        if is_mcycle_boundary {
            self.tick_mcycle_boundary_fall();
        }

        self.recapture_interrupts();
        (new_screen, pixel)
    }

    /// M-cycle-boundary CPU work on the rising edge: irq_latched capture,
    /// dispatch update, IME promotion, bus clear, timer/serial mcycle. The
    /// boundary PPU rise follows in the caller via `ppu_rise_edge`.
    fn tick_mcycle_boundary_rise(&mut self) {
        // cpu_irq_ack1↓ at +3.992 dots — hardware releases LALU.r_n
        // ~8 ps before this CLK9↑. Clear at boundary entry so
        // check_stat_edge below sees r_n released.
        self.cpu.irq.cpu_irq_ack1_pulse = false;
        self.cpu.irq.irq_ack_held = None;

        // yoii captures dispatch.latched() before data_phase_n↑ refreshes
        // the per-bit irq_latch — preserves pre-release values held
        // through the prior M-cycle's data phase.
        self.cpu
            .tick_irq_latched(self.model.halt_wake_samples_early());

        // data_phase_n↑ reopens the per-bit irq_latch_inst<i> to
        // re-snapshot IF for this M-cycle's dispatch.
        self.cpu.dispatch.set_data_phase_n(true);
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
        self.cpu.dispatch.tick_zacw();

        // Promote ime_delay (EI's shadow) to ime — produces EI's
        // one-instruction delay.
        self.cpu.irq.ime.write_immediate(if self.cpu.irq.ime_delay {
            crate::cpu::InterruptMasterEnable::Enabled
        } else {
            crate::cpu::InterruptMasterEnable::Disabled
        });

        self.cpu_bus.clear_activity();

        self.ppu
            .tick_clock_domain_capture(self.model.cpu_steps_per_dot() == 2);

        self.tick_cpu_clock_mcycle();
    }

    /// The CPU-clock peripherals (BOGA M-cycle pulse): the timer divider and
    /// serial shift clock. These are the SM83's own silicon, clocked by the
    /// CPU clock — not by the instruction sequencer. When the SM83 runs, this
    /// rides its M-cycle boundary; through the speed-switch blackout it keeps
    /// pulsing off the master clock while the SM83 is frozen.
    fn tick_cpu_clock_mcycle(&mut self) {
        self.timers.mcycle();
        if let Some(interrupt) = self.timers.take_pending_interrupt() {
            self.interrupts.request(interrupt);
        }

        // Serial bit-5 fall lands IF.serial in this M-cycle's
        // data-phase window for same-M-cycle dispatch.
        let counter = self.timers.internal_counter();
        if let Some(interrupt) = self
            .serial
            .mcycle(counter, self.model.has_serial_fast_clock())
        {
            self.interrupts.request(interrupt);
        }
    }

    /// Non-boundary T-cycle rise CPU work: pre-CUPA LCDC snapshot and the
    /// staged write apply at T-cycle 2. The PPU rise + STAT edge follow in
    /// the caller via `ppu_rise_edge`.
    fn tick_non_boundary_rise(&mut self, tcycle: TCycle) {
        // Snapshot LCDC.1 BEFORE the staged write applies — the
        // alet-rising DFF capture (SOBU on TEKY → FEPO → XYLO) beats
        // CUPA-rising's transparent-latch propagation by ~14 ns. Other
        // consumers read post-CUPA `regs` directly.
        self.ppu.snapshot_pre_cupa_lcdc();

        // Apply staged write at CUPA-rising (T-cycle 2). PPU registers
        // latch combinationally during CUPA-high; memory commits at
        // CUPA-falling in fall().
        if tcycle.as_u8() == 2
            && let Some(address) = self.cpu_bus.pending_write()
        {
            let value = self
                .cpu
                .pending_bus_write()
                .map(|(_, v)| v)
                .expect("cpu_bus pending write requires cpu.pending_bus_write to be Some");
            self.cpu_bus.drive(value);
            if self.drive_ppu_bus(address, value) {
                self.interrupts.request(Interrupt::VideoStatus);
            }
            // Snapshot OAM/VRAM lock at CUPA-rising. AND'd with the
            // mid and commit samples — the write blocks only if locked
            // throughout the entire CUPA window.
            self.cpu_bus
                .record_snapshot_lock(self.ppu.write_lock(address));
        }
    }

    /// Vector resolve (ISR M3→M4): clear zkog/zloz + the dispatched IF
    /// bit, latch the vector into pc. Reads the priority chain
    /// output (post-latch), matching the IE-push-bug timing.
    fn apply_vector_resolve(&mut self) {
        if self.cpu.take_pending_vector_resolve() {
            if let Some(interrupt) = self.cpu.dispatch.vector() {
                self.interrupts.clear(interrupt);
                self.cpu.irq.irq_ack_held = Some(interrupt);
                self.cpu.pc = interrupt.vector();
            } else {
                self.cpu.pc = 0x0000;
            }
            self.cpu.dispatch.clear_dispatch();
            // cpu_irq_ack1↑: LALU.r_n driven LOW via lety/movu until next
            // M-cycle boundary. Absorbs same-M-cycle SUKO rises.
            self.cpu.irq.cpu_irq_ack1_pulse = true;
        }
    }

    /// data_phase_n↓ at T1→T2 and the zkog SR-latch update. Together
    /// they gate this M-cycle's interrupt dispatch visibility.
    fn step_dispatch_logic(&mut self, tcycle: TCycle) {
        // data_phase_n↓ closes the per-bit irq_latch at the T1→T2
        // boundary, freezing IF visibility for this M-cycle's dispatch.
        // The halt-state spin keeps data_phase LOW so the latch stays
        // transparent throughout.
        if tcycle.as_u8() == 2 && !self.cpu.halt_rs_latched() {
            self.cpu.dispatch.set_data_phase_n(false);
        }

        // T2 rise: the CGB halt-release chain's sample point.
        if tcycle.as_u8() == 2 && self.model.halt_wake_samples_early() {
            self.cpu.presample_halt_wake();
        }

        // step_zkog: zaij = ime ∧ data_phase ∧ int_take ∧ xogs. HALT
        // body and halt-spin both feed into xogs so dispatch can fire
        // mid-HALT for the immediate-dispatch path.
        let halt_body = self.cpu.is_halted() && !self.cpu.halt_rs_latched();
        let halt_spin = self.cpu.halt_rs_latched();
        let data_phase = !halt_spin && (tcycle.as_u8() == 2 || tcycle.as_u8() == 3);
        let write_phase = !halt_spin && tcycle.as_u8() == 3;
        let ctl_fetch = self.cpu.is_fetch_phase() || halt_body;
        let xogs = (data_phase && ctl_fetch) || halt_spin;
        let ime_enabled = self.cpu.irq.ime.output() == crate::cpu::InterruptMasterEnable::Enabled;
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
        self.cpu
            .dispatch
            .step_zkog(ime_enabled, data_phase, write_phase, xogs);
    }

    /// Stage this M-cycle's bus activity. The CPU asserts at most one
    /// of cpu_rd / cpu_wr per M-cycle.
    fn stage_mcycle_bus_activity(&mut self) {
        if let Some((address, _value)) = self.cpu.pending_bus_write() {
            self.cpu_bus.stage_write(address);
        } else if let Some(address) = self.cpu.pending_bus_read() {
            self.cpu_bus.stage_read(address);
        }
    }

    /// BOWA: arm OAM corruption from any OAM-range address on the CPU
    /// bus this M-cycle. CUFE fires at MOPA (T-cycle 2 rise); arming
    /// must be visible at T-cycle 0 so the same M-cycle's MOPA edge
    /// picks it up.
    fn arm_oam_bugs(&mut self) {
        if let BusAction::InternalOamBug { address } = self.cpu.last_bus_action {
            self.ppu.arm_oam_bug_for_write(address);
        }
        if let Some(address) = self.cpu.pending_bus_read() {
            self.ppu.arm_oam_bug_for_read(address);
        }
        if let Some((address, _)) = self.cpu.pending_bus_write() {
            self.ppu.arm_oam_bug_for_write(address);
        }
    }

    /// Driver-enable edge (tobe↑ / wafu↑) at T-cycle 2: the addressed
    /// peripheral opens its tri-state driver. Mid-M-cycle flux
    /// propagates combinationally to the latch edge in `commit_read_latch`.
    fn apply_read_drive_enable(&mut self) {
        if let Some(address) = self.cpu_bus.pending_read() {
            let value = self.bus_value_at_drive_enable(address);
            // OAM read lock at the drive enable: the grant view tobe↑ samples
            // before this fall's PPU advance applies any lock onset.
            if let 0xFE00..=0xFEFF = address {
                self.model
                    .note_read_drive_phase(self.ppu.read_lock(address));
            }
            self.cpu_bus.drive(value);

            // A VRAM-source bus conflict on a read forces the DMA's OAM deposit
            // to $00, same as on a write.
            if self.dma.is_active_on_bus().is_some()
                && self
                    .model
                    .oam_dma_conflict_zeroes_oam(address, self.dma.source())
                && let Some((_, dst_offset)) = self.dma.peek_transfer()
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
        if let Some(address) = self.cpu_bus.mid_sample_pending() {
            self.cpu_bus.record_mid_lock(self.ppu.write_lock(address));
        }
    }

    /// CPU data latch (data_phase_n↑ near the end of T-cycle 3).
    /// Resolves the drive-enable snapshot against mid-M-cycle flux
    /// before the SM83 captures cpu_port_d.
    fn commit_read_latch(&mut self, ly_at_latch: Option<u8>) {
        if let BusAction::Read { address } = &self.cpu.last_bus_action {
            let address = *address;
            // A lockable read is offered the unfloated accessible byte; the
            // model owns the float decision from its latch lock view. Other
            // addresses resolve through `bus_value_at_latch`.
            let latch_lock = self.ppu.read_lock(address);
            let accessible = if latch_lock.is_some() {
                self.cpu_bus.data
            } else {
                self.bus_value_at_latch(address, self.cpu_bus.data, ly_at_latch)
            };
            let value = if let Some(source) = self.model.vram_dma_conflict_source(address) {
                self.read_dma_source(source)
            } else {
                self.model
                    .resolve_read_latch(address, accessible, latch_lock)
            };
            self.cpu.data_latch = value;
            self.commit_bus_read(address, value);
        }
    }

    /// CPU writes commit at CUPA-falling (end of T-cycle 3). PPU
    /// registers were already written at CUPA-rising via
    /// `drive_ppu_bus` in rise(); this commits memory.
    fn commit_write(&mut self) {
        if let BusAction::Write { address, value: _ } = &self.cpu.last_bus_action {
            let address = *address;
            let (locked_at_snapshot, locked_at_mid) = self.cpu_bus.write_lock_samples();
            self.write_byte_with_cupa_lock(
                address,
                self.cpu_bus.data,
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
        if let Some((src_addr, dst_offset)) = self.dma.peek_transfer() {
            self.dma_move(src_addr, 0xfe00 + dst_offset as u16);
        }

        // CGB VRAM DMA: commit the bytes it moves while it actually holds the
        // bus — gating on the hold keeps the transfer from overlapping the
        // arming instruction. (The trigger/quota tick ran before this edge's
        // write commit.) Idle (no-op) on the DMG.
        if self.model.console_state().dma_cpu_hold() || self.cpu.bus_suspended {
            if !self.model.vram_dma_take_setup_cell() {
                while let Some((src, dst)) = self.model.vram_dma_next_byte() {
                    self.dma_move(src, dst);
                }
            }
        }

        if let Some((dst_offset, src_byte, cpu_value)) = self.dma_conflict_write_pending.take() {
            let dst_addr = 0xfe00 + dst_offset as u16;
            let oam_addr = match ppu::memory::MappedAddress::map(dst_addr) {
                ppu::memory::MappedAddress::Oam(addr) => addr,
                _ => unreachable!(),
            };
            let value =
                self.model
                    .oam_dma_write_conflict_byte(src_byte, cpu_value, self.dma.source());
            self.ppu.write_oam(oam_addr, value);
            self.bus_trace.record(BusAccess {
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
                self.ppu.write_oam(oam_addr, 0);
                self.bus_trace.record(BusAccess {
                    address: dst_addr,
                    value: 0,
                    kind: BusAccessKind::Write,
                });
            }
        }

        self.external.tick_decay();
        // The RTC crystal is speed-independent: 4 base dots per M-cycle at
        // single speed, 2 at double speed.
        self.external
            .tick_rtc(4 / self.model.cpu_steps_per_dot() as u32);
    }

    /// Advance the OAM-DMA control gates one master-clock edge (engage/
    /// release/counter). The byte transfer itself commits at the M-cycle
    /// data phase in `tick_mcycle_boundary_fall`.
    fn drive_dma(&mut self, data_phase: bool) {
        self.dma.tick(data_phase);
    }

    /// Re-capture interrupt state after bus writes and M-cycle
    /// subsystems so IF mutations from CPU writes to 0xFF0F, STAT
    /// edges from PPU register writes, and serial completion are all
    /// visible by the time the next rise() ticks irq_latched.
    fn recapture_interrupts(&mut self) {
        let triggered = self.interrupts.triggered();
        self.cpu.update_interrupt_state(triggered);
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
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
                self.screen.draw_pixel(pixel.x, pixel.y, pixel.color);
            }
            ppu::PixelOutput {
                x: pixel.x,
                y: pixel.y,
                shade: <M::Ppu as ppu::PpuModel>::trace_shade(pixel.color),
            }
        });
        if result.new_frame {
            if self.ppu.control().video_enabled() && self.ppu.vsync_committed() {
                self.screen.present();
                self.model.on_present(&self.screen);
            }
            return (true, trace_pixel);
        }
        if result.lcd_disabled {
            self.screen.blank();
            self.model.on_present(&self.screen);
        }
        (false, trace_pixel)
    }
}
