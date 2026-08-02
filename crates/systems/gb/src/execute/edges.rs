use super::{FallEdge, PhaseResult, RiseEdge};
use crate::{
    Console, ConsoleShadow, Model,
    clock::{CpuGate, Edge, TcycleSchedule},
    cpu::mcycle::{BusAction, TCycle},
    interrupts::Interrupt,
    ppu,
};

impl<M: Model> Console<M> {
    /// The rising master edge of a T-cycle: advance the clock, then the CPU's
    /// pre-rise work, the dot edge the schedule places here, and the post-rise
    /// work. The rise always carries a dot edge — a dot rise at ÷1 and on the
    /// first ÷2 T-cycle, a dot fall on the second ÷2 T-cycle (a double-speed
    /// dot fall lands on a CPU rise, half a dot from the dot's own rise). The
    /// PPU rise is its own domain's edge, sequenced between the CPU's pre- and
    /// post-rise work rather than welded inside it.
    pub(super) fn tcycle_rise(&mut self, schedule: TcycleSchedule) -> PhaseResult {
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
    pub(super) fn tcycle_fall(&mut self, schedule: TcycleSchedule) -> PhaseResult {
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
        let ly_at_latch = match self.chassis.cpu.bus.last_bus_action {
            BusAction::Read { address: 0xFF44 } => Some(self.read(0xFF44)),
            _ => None,
        };

        let pre_fall_mode = self.chassis.ppu.pre_fall_mode();

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
    pub(super) fn tick_cpu_clock_mcycle(&mut self) {
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
        if let BusAction::InternalOamBug { address } = self.chassis.cpu.bus.last_bus_action {
            self.chassis.ppu.arm_oam_bug_for_write(address);
        }
        if let Some(address) = self.chassis.cpu.pending_bus_read() {
            self.chassis.ppu.arm_oam_bug_for_read(address);
        }
        if let Some((address, _)) = self.chassis.cpu.pending_bus_write() {
            self.chassis.ppu.arm_oam_bug_for_write(address);
        }
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
}
