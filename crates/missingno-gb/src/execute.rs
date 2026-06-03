use super::{
    ClockPhase, Console, Model, ScreenBuffer, StopAction,
    cpu::mcycle::{BusAction, TCycle},
    cpu_bus::{BusAccess, BusAccessKind},
    interrupts::Interrupt,
    memory::Bus,
    ppu,
};

/// CPU T-cycles the CGB holds the CPU `Stopped` during a double-speed switch
/// (the ~0x20000-T-cycle blackout). The divider and PPU run throughout; the CPU
/// re-engages at the new speed when this drains. Tuned against the age `spsw-*`
/// expected values.
const SPEED_SWITCH_BLACKOUT_TCYCLES: u32 = 0x2_0000;

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

        const PHASE_BUDGET: u32 = 400;
        let mut phases_remaining = PHASE_BUDGET;
        let mut tcycles = 0u32;

        // Single speed completes a T-cycle every two edges (at the fall back to
        // Low); double speed completes a full T-cycle on each edge.
        let double_speed = self.model.cpu_steps_per_dot() == 2;

        loop {
            assert!(
                phases_remaining > 0,
                "step() exceeded {PHASE_BUDGET} phase budget — possible infinite loop in CPU"
            );
            phases_remaining -= 1;

            let result = self.execute_phase();
            new_screen |= result.new_screen;

            // Check for instruction boundary after completing a T-cycle.
            if double_speed || self.clock_phase == ClockPhase::Low {
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
        self.execute_phase()
    }

    /// Advance to the next T-cycle boundary — the next Low state.
    /// Executes 1 phase if clock is High, 2 if Low. Returns true if
    /// a new frame was produced.
    pub fn step_tcycle(&mut self) -> bool {
        let mut new_screen = false;

        // Run phases until clock returns to Low (T-cycle complete)
        loop {
            let result = self.execute_phase();
            new_screen |= result.new_screen;
            if self.clock_phase == ClockPhase::Low {
                break;
            }
        }

        // Consume the boundary flag so step_traced sees mid-instruction
        // state after this returns.
        self.cpu.take_instruction_boundary();

        new_screen
    }

    /// Execute one master-clock edge. The PPU always advances one half-dot
    /// per edge; the CPU advances a half T-cycle per edge in lockstep (single
    /// speed) or a full T-cycle per edge (double speed — the CPU clock runs at
    /// 2× the dot clock). In double speed the rise edge runs the rise half of
    /// one T-cycle plus the fall half of that same T-cycle, and the fall edge
    /// runs the rise half of the next T-cycle plus its fall half — so the CPU's
    /// T-cycle-keyed bus events (drive at T2, commit at T3) land on consecutive
    /// edges while the PPU still advances one dot per two edges.
    fn execute_phase(&mut self) -> PhaseResult {
        let double_speed = self.model.cpu_steps_per_dot() == 2;
        match self.clock_phase {
            ClockPhase::Low => {
                let (mut new_screen, mut pixel) = self.rise_work(true);
                if double_speed {
                    let (ns, px) = self.fall_work(false);
                    new_screen |= ns;
                    if pixel.is_none() {
                        pixel = px;
                    }
                }
                self.clock_phase = ClockPhase::High;
                PhaseResult { new_screen, pixel }
            }
            ClockPhase::High => {
                let (mut new_screen, mut pixel) = (false, None);
                if double_speed {
                    let (ns, px) = self.rise_work(false);
                    new_screen |= ns;
                    pixel = px;
                }
                let (ns, px) = self.fall_work(true);
                new_screen |= ns;
                if pixel.is_none() {
                    pixel = px;
                }
                self.clock_phase = ClockPhase::Low;
                PhaseResult { new_screen, pixel }
            }
        }
    }

    /// Resolve a STOP the CPU has settled into (called at the M-cycle
    /// boundary). The model decides: a CGB armed speed switch starts the
    /// blackout (the CPU stays stopped while the divider/PPU run, then
    /// re-engages at the new speed); otherwise the CPU stays stopped.
    /// `elapsed_tcycles` is the CPU T-cycle count of the step that just ran.
    pub(crate) fn resolve_stop(&mut self, elapsed_tcycles: u32) {
        if !self.cpu.is_stopped() {
            return;
        }

        // Mid-blackout: drain the switch penalty, then re-engage. The divider
        // and PPU advanced through `elapsed_tcycles` while the CPU spun.
        if self.speed_switch_blackout > 0 {
            self.speed_switch_blackout = self.speed_switch_blackout.saturating_sub(elapsed_tcycles);
            if self.speed_switch_blackout == 0 {
                self.cpu.resume_from_stop();
            }
            return;
        }

        match self.model.resolve_stop() {
            StopAction::SpeedSwitch => {
                // Hardware resets DIV across the switch and holds the CPU for
                // the blackout (the model has already toggled its speed bit).
                let old_counter = self.timers.internal_counter();
                self.timers
                    .write_register(crate::timers::Register::Divider, 0);
                self.audio.on_div_write(old_counter);
                self.speed_switch_blackout = SPEED_SWITCH_BLACKOUT_TCYCLES;
            }
            StopAction::Remain => {}
        }
    }

    /// CPU + PPU work for a rising master-clock edge. `advance_ppu` gates this
    /// dot's PPU rise and the master-clock-domain APU tick: true on the edge
    /// that owns the PPU rise, false on the extra double-speed CPU T-cycle that
    /// shares the dot. Sets no clock phase — `execute_phase` owns that.
    fn rise_work(&mut self, advance_ppu: bool) -> (bool, Option<ppu::PixelOutput>) {
        let is_mcycle_boundary = self.cpu.consume_boundary_pending();
        let mut new_screen = false;
        let mut pixel = None;

        if is_mcycle_boundary {
            self.tick_mcycle_boundary_rise();
            if advance_ppu {
                let (ns, pix) = self.ppu_rise_edge();
                new_screen |= ns;
                pixel = pix;
            }
        }

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
        if advance_ppu {
            let double_speed = self.model.cpu_steps_per_dot() == 2;
            self.audio
                .tcycle(self.timers.internal_counter(), tcycle.as_u8(), double_speed);
        }

        if is_mcycle_boundary {
            self.stage_mcycle_bus_activity();
        }
        if M::HAS_OAM_BUG && tcycle.as_u8() == 0 {
            self.arm_oam_bugs();
        }
        if !is_mcycle_boundary {
            self.tick_non_boundary_rise(tcycle);
            if advance_ppu {
                let (ns, pix) = self.ppu_rise_edge();
                new_screen |= ns;
                if pixel.is_none() {
                    pixel = pix;
                }
            }
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);
        }

        // MOPA-rising fires any armed OAM bug.
        if M::HAS_OAM_BUG && tcycle.as_u8() == 2 {
            self.ppu.apply_pending_oam_bug();
        }

        (new_screen, pixel)
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

    /// CPU + PPU work for a falling master-clock edge. `advance_ppu` gates this
    /// dot's PPU fall (and its IF requests) and the master-clock-domain APU
    /// wave latch; false on the extra double-speed CPU T-cycle that shares the
    /// dot. Sets no clock phase — `execute_phase` owns that.
    fn fall_work(&mut self, advance_ppu: bool) -> (bool, Option<ppu::PixelOutput>) {
        let tcycle = self.cpu.last_tcycle();
        let is_mcycle_boundary = self.cpu.at_mcycle_boundary();
        let mut new_screen = false;
        let mut pixel = None;

        // CH3's BUSA / AZUS DFFs latch on apu_4mhz ↑ (= our fall);
        // settle before the T=2 drive-enable so wave-RAM reads see
        // the current wave_data_latch.
        if advance_ppu {
            self.audio.fall_sync();
        }

        if tcycle.as_u8() == 2 {
            self.apply_read_drive_enable();
        }

        // PPU master-clock fall: divider chain, CATU, scanline
        // boundaries, fetcher, DFF8/DFF9, LCD-off.
        let video_result = if advance_ppu {
            let oam_bus = self.dma.oam_bus_owner();
            Some(self.ppu.on_master_clock_fall(is_mcycle_boundary, oam_bus))
        } else {
            None
        };

        if tcycle.as_u8() == 2 {
            self.sample_mid_cupa_lock();
        }

        self.commit_read_latch();
        self.commit_write();

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
        self.cpu.tick_irq_latched();

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

        self.timers.mcycle();
        if let Some(interrupt) = self.timers.take_pending_interrupt() {
            self.interrupts.request(interrupt);
        }

        // Serial bit-5 fall lands IF.serial in this M-cycle's
        // data-phase window for same-M-cycle dispatch.
        let counter = self.timers.internal_counter();
        if let Some(interrupt) = self.serial.mcycle(counter) {
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
            self.cpu_bus.drive(value);
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
    fn commit_read_latch(&mut self) {
        if let BusAction::Read { address } = &self.cpu.last_bus_action {
            let address = *address;
            let value = self.bus_value_at_latch(address, self.cpu_bus.data);
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
            let byte = self.read_dma_source(src_addr);
            let dst_addr = 0xfe00 + dst_offset as u16;
            let oam_addr = match ppu::memory::MappedAddress::map(dst_addr) {
                ppu::memory::MappedAddress::Oam(addr) => addr,
                _ => unreachable!(),
            };
            self.ppu.write_oam(oam_addr, byte);
            self.bus_trace.record(BusAccess {
                address: src_addr,
                value: byte,
                kind: BusAccessKind::DmaRead,
            });
            self.bus_trace.record(BusAccess {
                address: dst_addr,
                value: byte,
                kind: BusAccessKind::DmaWrite,
            });
            match Bus::of(src_addr) {
                Some(Bus::External) => self.external.drive(byte),
                Some(Bus::Vram) => self.vram_bus.drive(byte),
                None => {}
            }
        }

        if let Some((dst_offset, src_byte, cpu_value)) = self.dma_conflict_write_pending.take() {
            let dst_addr = 0xfe00 + dst_offset as u16;
            let oam_addr = match ppu::memory::MappedAddress::map(dst_addr) {
                ppu::memory::MappedAddress::Oam(addr) => addr,
                _ => unreachable!(),
            };
            let value = if self.dma.source_drives_write_phase() {
                src_byte & cpu_value
            } else {
                cpu_value
            };
            self.ppu.write_oam(oam_addr, value);
            self.bus_trace.record(BusAccess {
                address: dst_addr,
                value,
                kind: BusAccessKind::Write,
            });
        }

        self.external.tick_decay();
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
