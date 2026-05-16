use super::{
    BusAccess, BusAccessKind, ClockPhase, GameBoy, StagedBusRead, StagedBusWrite,
    cpu::mcycle::DotAction,
    interrupts::Interrupt,
    memory::Bus,
    ppu::{self, PpuTickResult, types::palette::PaletteIndex},
};

/// Whether the OAM bug corruption uses the read or write formula.
/// Determined by the CPU operation type, not by the OAM control
/// signals at the moment of the spurious SRAM clock.
pub(super) enum OamBugKind {
    Read,
    Write,
}

/// Result of executing one instruction.
pub struct StepResult {
    /// Whether a new video frame was produced during this instruction.
    pub new_screen: bool,
    /// Whether battery-backed SRAM was written during this instruction.
    pub sram_dirty: bool,
    /// Number of T-cycles (dots) consumed by this instruction.
    pub dots: u32,
}

/// Result of executing one half-phase (rise or fall).
pub struct PhaseResult {
    /// Whether a new video frame was produced.
    pub new_screen: bool,
    /// Pixel pushed to the LCD during this phase, if any.
    pub pixel: Option<ppu::PixelOutput>,
}

impl GameBoy {
    pub fn step(&mut self) -> StepResult {
        self.step_traced(false).0
    }

    /// Step one instruction, optionally recording all bus accesses.
    /// Returns (result, trace). Trace is empty when `trace` is false.
    pub fn step_traced(&mut self, trace: bool) -> (StepResult, Vec<BusAccess>) {
        if trace {
            self.bus_trace = Some(Vec::new());
        }

        // If step_dot() left us mid-instruction, drain to the next
        // boundary first, then run one full instruction.
        let mut new_screen = false;
        let mut dots = 0u32;
        if !self.cpu.at_instruction_boundary() {
            let r = self.step_instruction();
            new_screen |= r.new_screen;
            dots += r.dots;
        }
        let r = self.step_instruction();
        new_screen |= r.new_screen;
        dots += r.dots;

        let sram_dirty = self.external.cartridge.take_sram_dirty();
        let trace = self.bus_trace.take().unwrap_or_default();
        (
            StepResult {
                new_screen,
                sram_dirty,
                dots,
            },
            trace,
        )
    }

    /// Run one complete instruction from start to finish.
    ///
    /// Runs phases until the CPU returns to the Fetch phase at a fresh
    /// M-cycle boundary (instruction boundary). At that point, EI delay
    /// is advanced and control returns to the caller.
    fn step_instruction(&mut self) -> StepResult {
        let mut new_screen = false;
        self.last_read_value = 0;

        // Consume the current instruction boundary (we're starting
        // from a boundary — we want to run until the NEXT one).
        self.cpu.take_instruction_boundary();

        const PHASE_BUDGET: u32 = 400;
        let mut phases_remaining = PHASE_BUDGET;
        let mut dots = 0u32;

        loop {
            assert!(
                phases_remaining > 0,
                "step() exceeded {PHASE_BUDGET} phase budget — possible infinite loop in CPU"
            );
            phases_remaining -= 1;

            let result = self.execute_phase();
            new_screen |= result.new_screen;

            // Check for instruction boundary after completing a dot
            // (clock is Low = just finished fall() = dot complete)
            if self.clock_phase == ClockPhase::Low {
                dots += 1;
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
            dots,
        }
    }

    /// Advance exactly one half-phase — execute rise() or fall()
    /// depending on current clock level.
    pub fn step_phase(&mut self) -> PhaseResult {
        self.execute_phase()
    }

    /// Advance to the next dot (T-cycle) boundary — the next Low
    /// state. Executes 1 phase if clock is High, 2 if Low.
    /// Returns true if a new frame was produced.
    pub fn step_dot(&mut self) -> bool {
        let mut new_screen = false;

        // Run phases until clock returns to Low (dot complete)
        loop {
            let result = self.execute_phase();
            new_screen |= result.new_screen;
            if self.clock_phase == ClockPhase::Low {
                break;
            }
        }

        // Consume instruction boundary flag (used by step_traced to detect
        // mid-instruction state). HALT bug and EI delay are handled
        // internally by the CPU state machine.
        self.cpu.take_instruction_boundary();

        new_screen
    }

    /// Execute one phase (half-dot) of hardware. When the clock is
    /// Low, execute rise() (Low→High edge). When High, execute
    /// fall() (High→Low edge).
    fn execute_phase(&mut self) -> PhaseResult {
        match self.clock_phase {
            ClockPhase::Low => self.rise(),
            ClockPhase::High => self.fall(),
        }
    }

    /// Rising edge: advance CPU state machine, capture bus reads,
    /// tick timer and PPU, fire OAM bugs.
    ///
    /// At each M-cycle boundary, `irq_latched` (yoii) captures
    /// `irq_pending`, driving the HALT-release chain (yoii → ykua →
    /// halt↓). The dispatching CLK9↑ — where zacw and the write-back
    /// DFFs share the same capture edge — happens inside `retire_edge()`,
    /// called via `next_mcycle` → `mcycle_*`; the sequencer combinationally
    /// selects fetch vs dispatch from `dispatch_active.q` on that same
    /// edge. On the HALT-wake path, `tick_dispatch_active` captures
    /// dispatch_active on the closing CLK9↑ of `Halted(WakeIntake)` so
    /// the next M-cycle's selector resolves dispatch vs fetch from the
    /// freshly-captured `dispatch_active.q`.
    fn rise(&mut self) -> PhaseResult {
        let mut new_screen = false;
        let mut pixel = None;
        let is_mcycle_boundary = self.cpu.consume_boundary_pending();

        // ── M-cycle boundary: irq_latched capture, then PPU + interrupt updates ──
        if is_mcycle_boundary {
            // yoii (irq_latched) captures `dispatch.latched()` first —
            // BEFORE this boundary's data_phase_n↑ + update_latch
            // refreshes the per-bit irq_latch. Hardware: yoii's setup
            // window precedes the data_phase_n↑ at -1,144 ps; for an
            // IF source held by the per-bit latch through the prior
            // M-cycle's data-phase, the held value's release lands
            // AFTER yoii's capture window, so yoii captures the
            // pre-release value.
            self.cpu.tick_irq_latched();

            // data_phase_n↑ at the new M-cycle's CLK9↑: the per-bit
            // irq_latch_inst<i> enabled D-latch reopens and re-snapshots
            // IF — visible to dispatch from this M-cycle onwards.
            self.cpu.dispatch.set_data_phase_n(true);
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);

            // zacw captures zfex on this CLK9↑.
            self.cpu.dispatch.tick_zacw();

            // ime ← ime_delay: copies the EI shadow stage onto the IME
            // DFF at the M-cycle boundary. EI's commit only set
            // ime_delay; this copy is what makes the new IME visible to
            // dispatch — one M-cycle after EI committed.
            self.cpu.ime.write_immediate(if self.cpu.ime_delay {
                crate::cpu::InterruptMasterEnable::Enabled
            } else {
                crate::cpu::InterruptMasterEnable::Disabled
            });

            // Clear any staged bus write/read from the previous M-cycle.
            self.staged_bus_write = None;
            self.staged_bus_read = None;

            // Timer ticks once per M-cycle (BOGA).
            self.timers.mcycle();

            // Drain g151 → g154: the CLK9-clocked g151 DFF latches the
            // reload-cycle pulse during timers.mcycle(); g154 captures
            // immediately after this CLK9↑, raising IF.bit2 before the
            // CPU's dispatch read at this M-cycle.
            if let Some(interrupt) = self.timers.take_pending_interrupt() {
                self.interrupts.request(interrupt);
            }

            // PPU master-clock rising edge at the M-cycle boundary (dot 0).
            let oam_bus = self.dma.oam_bus_owner();
            let ppu_result = self.ppu.on_master_clock_rise(&self.vram_bus.vram, oam_bus);
            if ppu_result.request_vblank {
                self.interrupts.request(Interrupt::VideoBetweenFrames);
            }
            let (ns, pix) = self.apply_ppu_result(&ppu_result);
            new_screen |= ns;
            if pixel.is_none() {
                pixel = pix;
            }

            // SUKO is combinational — check for STAT edge after PPU rise.
            if self.ppu.check_stat_edge() {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            // Capture interrupt state so the CPU's dispatch check sees it.
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
        }

        // ── CPU dot advance ──
        let dot_action = self.cpu.next_dot(self.last_read_value);
        self.current_dot_action = dot_action;

        // IE push bug + ctl_int_entry_m6: vector-resolve fires between
        // ISR M3 and M4. Clears zkog/zloz (per netlist R_n =
        // NOR(ctl_int_entry_m6, sys_reset)) and the dispatched IF bit
        // (per netlist inta[i] = AND(ctl_int_entry_m6, irq_latch_gated_q_n[i])).
        if self.cpu.take_pending_vector_resolve() {
            // Read the post-latch priority chain output (matches the
            // hardware vector resolve which reads irq_latch_gated, not
            // raw IF).
            if let Some(interrupt) = self.cpu.dispatch.vector() {
                self.interrupts.clear(interrupt);
                self.cpu.bus_counter = interrupt.vector();
            } else {
                self.cpu.bus_counter = 0x0000;
            }
            self.cpu.dispatch.clear_dispatch();
        }

        let dot = self.cpu.dot_for_execute();
        self.current_dot = dot;

        // data_phase_n↓ at the dot 1→2 boundary closes the per-bit
        // irq_latch_inst<i> enabled D-latch. Subsequent IF requests this
        // M-cycle still set `requested` but do not propagate to the
        // latch until the next data_phase_n↑ at the M-cycle boundary.
        //
        // Skipped only when the halt RS-latch is set (true halt-state
        // spin): the CPU phase ring is frozen and data_phase is held
        // LOW. HALT body M-cycle (halt_rs_latched=false) still pulses
        // data_phase normally so the per-bit latch gates correctly
        // against IF rises during HALT body's data-phase.
        if dot.index() == 2 && !self.cpu.halt_rs_latched() {
            self.cpu.dispatch.set_data_phase_n(false);
        }

        // step_zkog drives zaij combinational + zkog SR-latch update.
        // zaij = ime ∧ data_phase ∧ int_take ∧ xogs.
        // HALT body M-cycle acts as a fetch cycle for xogs (ctl_op_halt
        // overlap fetch is the next-instruction prefetch) — so zaij can
        // fire during HALT body's data-phase, capturing zacw at M_h
        // start for the immediate-dispatch path.
        let halt_body = self.cpu.is_halted() && !self.cpu.halt_rs_latched();
        let halt_spin = self.cpu.halt_rs_latched();
        let data_phase = !halt_spin && (dot.index() == 2 || dot.index() == 3);
        let write_phase = !halt_spin && dot.index() == 3;
        let ctl_fetch = self.cpu.is_fetch_phase() || halt_body;
        let xogs = (data_phase && ctl_fetch) || halt_spin;
        let ime_enabled = self.cpu.ime.output() == crate::cpu::InterruptMasterEnable::Enabled;
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
        self.cpu
            .dispatch
            .step_zkog(ime_enabled, data_phase, write_phase, xogs);

        // Stage CPU bus writes at dot 0. On hardware, the CPU places
        // the address on the bus at phase A and asserts cpu_wr; CUPA
        // rises at dot 2 (phase E), spanning 1.498 dots through dot 3
        // (CUPA falling). PPU register latches are transparent during
        // CUPA-high and capture at CUPA-falling. The emulator drives
        // cpu_bus.data at dot 2 rise (CUPA-rising equivalent); PPU
        // registers apply via drive_ppu_bus at the same dot, memory
        // commits via write_byte at fall() of dot 3.
        if is_mcycle_boundary && let Some((address, _value)) = self.cpu.pending_bus_write() {
            self.staged_bus_write = Some(StagedBusWrite {
                address,
                applied: false,
                locked_at_snapshot: None,
                locked_at_mid: None,
            });
        }

        // Stage CPU bus reads at dot 0. On hardware, the addressed
        // peripheral's tri-state driver (`tobe`/`wafu`) enables at
        // dot 2.005 and pulls cpu_port_d to the source value; the
        // CPU latches the bus at end of M-cycle. The driver-output
        // settling window extends past the latch, so peripheral
        // state changes after dot 2 do not propagate to the bus
        // in time — captured here, applied at dot 2.
        if is_mcycle_boundary && let Some(address) = self.cpu.pending_bus_read() {
            self.staged_bus_read = Some(StagedBusRead {
                address,
                applied: false,
            });
        }

        // BOWA (dot 0): record OAM bug from address in the upcoming action.
        if dot.bowa()
            && let DotAction::InternalOamBug { address } = &self.current_dot_action
            && (0xFE00..=0xFEFF).contains(address)
        {
            match self.pending_oam_bug {
                Some(OamBugKind::Read) => {}
                _ => {
                    self.pending_oam_bug = Some(OamBugKind::Write);
                }
            }
        }

        // ── Non-boundary dots: PPU rise + interrupt capture AFTER CPU dot advance ──
        if !is_mcycle_boundary {
            // Snapshot LCDC.1 BEFORE the staged bus write applies — the
            // alet-rising-edge DFF capture (SOBU on TEKY → FEPO → XYLO)
            // wins the gate-delay race against CUPA-rising's transparent-
            // latch propagation by ~14 ns (spec §6.10 line 1840). Other
            // combinational consumers read `regs` directly and see
            // post-CUPA values after `drive_ppu_bus` below.
            self.ppu.snapshot_pre_cupa_lcdc();

            // Apply staged bus write at BusDot 2 rise (phase E). On
            // hardware, CUPA rises at dot 2 coincident with alet rising
            // and gates the PPU register latches transparent across
            // dots 2-3 (CUPA pulse ~1.498 dots wide). The CPU drives
            // cpu_port_d at this dot; consumers latch from it at their
            // sub-phase event (PPU regs combinationally during CUPA-
            // high, memory at fall() of dot 3 / CUPA-falling).
            if dot.as_u8() == 2
                && let Some(staged) = self.staged_bus_write.as_ref()
                && !staged.applied
            {
                let address = staged.address;
                let value = self.cpu.pending_bus_write().map(|(_, v)| v).expect(
                    "staged_bus_write requires pending_bus_write to be Some during the M-cycle",
                );
                self.cpu_bus.data = value;
                if self.drive_ppu_bus(address, self.cpu_bus.data) {
                    self.interrupts.request(Interrupt::VideoStatus);
                }
                // Capture OAM/VRAM lock at CUPA-rising (dot 2), the
                // start of the write strobe. Combined with the commit-
                // time lock state via AND in `write_byte_with_lock` —
                // block only when locked across the entire CUPA window
                // (M-cycle-quantized lock per spec §4.9).
                let locked_at_snapshot = match address {
                    0xFE00..=0xFE9F => Some(self.ppu.oam_write_locked()),
                    0x8000..=0x9FFF => Some(self.ppu.vram_write_locked()),
                    _ => None,
                };
                let staged = self.staged_bus_write.as_mut().unwrap();
                staged.applied = true;
                staged.locked_at_snapshot = locked_at_snapshot;
            }

            // PPU master-clock rising edge for non-boundary dots.
            let oam_bus = self.dma.oam_bus_owner();
            let ppu_result = self.ppu.on_master_clock_rise(&self.vram_bus.vram, oam_bus);
            if ppu_result.request_vblank {
                self.interrupts.request(Interrupt::VideoBetweenFrames);
            }

            let (ns, pix) = self.apply_ppu_result(&ppu_result);
            new_screen |= ns;
            if pixel.is_none() {
                pixel = pix;
            }

            // SUKO is combinational — check for STAT edge after PPU rise.
            let stat_edge = self.ppu.check_stat_edge();
            if stat_edge {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            // Capture interrupt state for non-boundary dots. irq_latched
            // ticks in the matching fall(), capturing this dot's irq_pending
            // for the cascade-settling counter.
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);
        }

        // VOGA capture (HBlank) happens on the master-clock rising edge via
        // HblankPipeline::capture_voga(): VOGA is ALET-clocked, and ALET
        // rises in-phase with ck1_ck2 (master-clock rising). The mode()
        // function uses `xymu && !wodu` to predict HBlank state for CPU
        // STAT reads, so settle_alet is not needed. G4.2 confirmed WODU
        // doesn't depend on XYMU, making the prediction reliable.

        // MOPA rising edge (dot 2): fire OAM bug.
        if dot.mopa()
            && !dot.boga()
            && let Some(kind) = self.pending_oam_bug.take()
        {
            match kind {
                OamBugKind::Read => self.ppu.oam_bug_read(),
                OamBugKind::Write => self.ppu.oam_bug_write(),
            }
        }

        self.clock_phase = ClockPhase::High;
        PhaseResult { new_screen, pixel }
    }

    /// Falling edge: PPU falling phase, interrupt latch capture,
    /// bus writes, M-cycle subsystems (serial, DMA, audio).
    fn fall(&mut self) -> PhaseResult {
        let mut new_screen = false;
        let dot = self.current_dot;
        let is_mcycle_boundary = dot.boga();

        // Driver-enable edge (`tobe↑` / `wafu↑` at dot 2.005, spec §4.6):
        // the addressed peripheral opens its tri-state driver and starts
        // putting its value on the bus. Any per-address mid-M-cycle flux
        // (OAM/VRAM lock transitions, STAT/LY bit changes) propagates
        // combinationally to the latch edge and is resolved there.
        if dot.as_u8() == 2
            && let Some(staged) = self.staged_bus_read.as_ref()
            && !staged.applied
        {
            let address = staged.address;
            self.cpu_bus.data = self.bus_value_at_drive_enable(address);
            self.staged_bus_read.as_mut().unwrap().applied = true;
        }

        // PPU master-clock falling edge: divider chain (WUVU/VENA/TALU),
        // CATU, scanline boundaries, fetcher, DFF8/DFF9, LCD-off.
        let oam_bus = self.dma.oam_bus_owner();
        let video_result = self.ppu.on_master_clock_fall(is_mcycle_boundary, oam_bus);

        // Mid-CUPA sample for the staged OAM/VRAM write (spec §10.5.6
        // AJUJ-glitch window). On hardware, the CPU's CUPA strobe is
        // high throughout dots 2-3 of the write M-cycle; if AJUJ is
        // briefly high at ANY edge during that window, the per-byte
        // strobe asserts and the write lands. The discretized model
        // samples lock state at 3 edges (snapshot at rise of dot 2,
        // mid at fall of dot 2 just after AVAP processing, commit at
        // fall of dot 3) and blocks only if locked at ALL three. AVAP
        // detection fires inside `on_master_clock_fall` above; the
        // begin_rendering deferral to next rise (hblank_pipeline.rs)
        // makes `mode2=0 AND mode3=0` observable at this exact edge.
        if dot.as_u8() == 2
            && let Some(staged) = self.staged_bus_write.as_ref()
            && staged.applied
            && staged.locked_at_mid.is_none()
        {
            let address = staged.address;
            let locked_at_mid = match address {
                0xFE00..=0xFE9F => Some(self.ppu.oam_write_locked()),
                0x8000..=0x9FFF => Some(self.ppu.vram_write_locked()),
                _ => None,
            };
            self.staged_bus_write.as_mut().unwrap().locked_at_mid = locked_at_mid;
        }

        // CPU data latch: at data_phase_n↑ (~end of M-cycle, dot 3.995),
        // the SM83 captures cpu_port_d into its internal data register.
        // The bus value was set at dot 2 (above) when the peripheral's
        // tri-state driver enabled — capture from cpu_bus.data here and
        // fire commit_bus_read for the side effects (bus-latch drive,
        // trace recording) at the same timing the original single-stage
        // cpu_read had.
        if let DotAction::Read { address } = &self.current_dot_action {
            let address = *address;
            if (0xFE00..=0xFEFF).contains(&address) {
                self.pending_oam_bug = Some(OamBugKind::Read);
            }
            // Latch edge (`data_phase_n↑` at dot 3.995, spec §13.6):
            // the CPU captures the bus into its internal data register.
            // The final value resolves the drive-enable snapshot against
            // any per-address mid-M-cycle flux — OAM/VRAM lock state at
            // the latch edge, STAT/LY per-bit transitions during the
            // drive window.
            let value = self.bus_value_at_latch(address, self.cpu_bus.data);
            self.last_read_value = value;
            self.commit_bus_read(address, value);
        }

        // Bus writes on the falling edge. The cpu_wr window closes
        // before mid-M-cycle source clocks rise (POPU, SUKO), so a
        // FF0F-write-clear pulse on lyta/movu/etc. (the IF dffsr r_n
        // inputs) releases before the source-clock capture — rise
        // wins same-dot. Apply CPU writes first, then IF mutations.
        match &self.current_dot_action {
            DotAction::Idle | DotAction::InternalOamBug { .. } | DotAction::Read { .. } => {}
            DotAction::Write { address, value: _ } => {
                let address = *address;
                if (0xFE00..=0xFEFF).contains(&address) {
                    self.pending_oam_bug = Some(OamBugKind::Write);
                }
                // drive_ppu_bus already fired at dot 2 for PPU registers
                // (CUPA-rising visibility); for non-PPU addresses it's a
                // no-op. Memory commits here at fall() of dot 3 (CUPA-
                // falling / M-cycle boundary equivalent), reading the
                // CPU-driven value from cpu_bus.data.
                let (locked_at_snapshot, locked_at_mid) = self
                    .staged_bus_write
                    .as_ref()
                    .map(|s| (s.locked_at_snapshot, s.locked_at_mid))
                    .unwrap_or((None, None));
                self.write_byte_with_cupa_lock(
                    address,
                    self.cpu_bus.data,
                    locked_at_snapshot,
                    locked_at_mid,
                );
            }
        }

        // VBlank IF: the divider chain now runs in fall(), so POPU
        // (VBlank) transitions happen here, not in rise().
        if video_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }

        // SUKO is combinational — check for STAT edge after every phase.
        let stat_edge = self.ppu.check_stat_edge();
        if stat_edge {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        let (ns, pixel) = self.apply_ppu_result(&video_result);
        new_screen |= ns;

        if is_mcycle_boundary {
            // Serial ticks once per M-cycle. Run before update_interrupt_state
            // so a serial-complete IF is visible to the same-fall capture and
            // the next rise's irq_latched sees it — keeping serial in the
            // mid-M-cycle source class alongside VBlank/STAT.
            let counter = self.timers.internal_counter();
            if let Some(interrupt) = self.serial.mcycle(counter, &mut *self.link) {
                self.interrupts.request(interrupt);
            }

            // OAM DMA: transfer one byte per M-cycle.
            if let Some((src_addr, dst_offset)) = self.dma.mcycle() {
                let byte = self.read_dma_source(src_addr);
                let dst_addr = 0xfe00 + dst_offset as u16;
                let oam_addr = match ppu::memory::MappedAddress::map(dst_addr) {
                    ppu::memory::MappedAddress::Oam(addr) => addr,
                    _ => unreachable!(),
                };
                self.ppu.write_oam(oam_addr, byte);
                if let Some(trace) = &mut self.bus_trace {
                    trace.push(BusAccess {
                        address: src_addr,
                        value: byte,
                        kind: BusAccessKind::DmaRead,
                    });
                    trace.push(BusAccess {
                        address: dst_addr,
                        value: byte,
                        kind: BusAccessKind::DmaWrite,
                    });
                }
                match Bus::of(src_addr) {
                    Some(Bus::External) => {
                        self.external.drive(byte);
                    }
                    Some(Bus::Vram) => {
                        self.vram_bus.drive(byte);
                    }
                    None => {}
                }
            }

            // External bus decay.
            self.external.tick_decay();

            self.audio.mcycle(self.timers.internal_counter());
        }

        // Capture interrupt state AFTER bus writes and M-cycle subsystems so
        // IF updates from a CPU write to 0xFF0F, STAT edges from PPU register
        // writes, and serial completion are all visible on the same fall.
        // irq_latched ticks on the next rise to sample irq_pending into the DFF.
        {
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);
        }

        self.clock_phase = ClockPhase::Low;
        PhaseResult { new_screen, pixel }
    }

    /// Process a PPU tick result: write pixel to back buffer; on a
    /// VSYNC pulse, present iff MEDA has pulsed since LCD-on (the
    /// LCD only latches frames after the first VSYNC). On the LCD-off
    /// transition, blank the display to match hardware's LCD-off state.
    /// Returns `(new_screen, pixel)` — `new_screen` only fires on
    /// VSYNC, matching hardware frame boundaries. LCD-off blanking
    /// is a separate signal and does not count as a new screen for
    /// harness/UI frame budgets.
    fn apply_ppu_result(&mut self, result: &PpuTickResult) -> (bool, Option<ppu::PixelOutput>) {
        if let Some(pixel) = result.pixel {
            if pixel.x < ppu::screen::PIXELS_PER_LINE && pixel.y < ppu::screen::NUM_SCANLINES {
                self.screen
                    .draw_pixel(pixel.x, pixel.y, PaletteIndex(pixel.shade));
            }
        }
        if result.new_frame {
            if self.ppu.control().video_enabled() && self.ppu.vsync_committed() {
                self.screen.present();
                if let Some(sgb) = &mut self.sgb {
                    sgb.update_screen(&self.screen);
                }
            }
            return (true, result.pixel);
        }
        if result.lcd_disabled {
            self.screen.blank();
            if let Some(sgb) = &mut self.sgb {
                sgb.update_screen(&self.screen);
            }
        }
        (false, result.pixel)
    }
}
