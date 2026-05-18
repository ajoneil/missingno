use super::{
    ClockPhase, GameBoy,
    cpu::mcycle::BusAction,
    cpu_bus::{BusAccess, BusAccessKind},
    interrupts::Interrupt,
    memory::Bus,
    ppu::{self, PpuTickResult, types::palette::PaletteIndex},
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

impl GameBoy {
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

        loop {
            assert!(
                phases_remaining > 0,
                "step() exceeded {PHASE_BUDGET} phase budget — possible infinite loop in CPU"
            );
            phases_remaining -= 1;

            let result = self.execute_phase();
            new_screen |= result.new_screen;

            // Check for instruction boundary after completing a T-cycle
            // (clock is Low = just finished fall() = T-cycle complete).
            if self.clock_phase == ClockPhase::Low {
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

    /// Execute one phase (half-T-cycle) of hardware. When the clock is
    /// Low, execute rise() (Low→High edge). When High, execute
    /// fall() (High→Low edge).
    fn execute_phase(&mut self) -> PhaseResult {
        match self.clock_phase {
            ClockPhase::Low => self.rise(),
            ClockPhase::High => self.fall(),
        }
    }

    /// Rising edge: tick boundary subsystems on M-cycle boundaries,
    /// advance the CPU, apply staged writes, tick the PPU, fire OAM bug.
    fn rise(&mut self) -> PhaseResult {
        let mut new_screen = false;
        let mut pixel = None;
        let is_mcycle_boundary = self.cpu.consume_boundary_pending();

        if is_mcycle_boundary {
            // yoii captures dispatch.latched() before data_phase_n↑
            // refreshes the per-bit irq_latch — preserves pre-release
            // values held through the prior M-cycle's data phase.
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
            self.cpu.ime.write_immediate(if self.cpu.ime_delay {
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

            if self.ppu.check_stat_edge() {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
        }

        self.cpu.next_tcycle();

        // Vector resolve (ISR M3→M4): clear zkog/zloz + the dispatched
        // IF bit, latch the vector into bus_counter. Reads the priority
        // chain output (post-latch), matching the IE-push-bug timing.
        if self.cpu.take_pending_vector_resolve() {
            if let Some(interrupt) = self.cpu.dispatch.vector() {
                self.interrupts.clear(interrupt);
                self.cpu.bus_counter = interrupt.vector();
            } else {
                self.cpu.bus_counter = 0x0000;
            }
            self.cpu.dispatch.clear_dispatch();
        }

        let tcycle = self.cpu.last_tcycle();

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
        let ime_enabled = self.cpu.ime.output() == crate::cpu::InterruptMasterEnable::Enabled;
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
        self.cpu
            .dispatch
            .step_zkog(ime_enabled, data_phase, write_phase, xogs);

        // Stage this M-cycle's bus activity. The CPU asserts at most one
        // of cpu_rd / cpu_wr per M-cycle; we apply it at T-cycle 2 below.
        if is_mcycle_boundary {
            if let Some((address, _value)) = self.cpu.pending_bus_write() {
                self.cpu_bus.stage_write(address);
            } else if let Some(address) = self.cpu.pending_bus_read() {
                self.cpu_bus.stage_read(address);
            }
        }

        // BOWA: arm OAM corruption from any OAM-range address on the
        // CPU bus this M-cycle. CUFE fires at MOPA (T-cycle 2 rise);
        // arming must be visible at T-cycle 0 so the same M-cycle's
        // MOPA edge picks it up.
        if tcycle.as_u8() == 0 {
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

        if !is_mcycle_boundary {
            // Snapshot LCDC.1 BEFORE the staged write applies — the
            // alet-rising DFF capture (SOBU on TEKY → FEPO → XYLO)
            // beats CUPA-rising's transparent-latch propagation by
            // ~14 ns. Other consumers read post-CUPA `regs` directly.
            self.ppu.snapshot_pre_cupa_lcdc();

            // Apply staged write at CUPA-rising (T-cycle 2). PPU
            // registers latch combinationally during CUPA-high; memory
            // commits at CUPA-falling in fall().
            if tcycle.as_u8() == 2 && let Some(address) = self.cpu_bus.pending_write() {
                let value = self.cpu.pending_bus_write().map(|(_, v)| v).expect(
                    "cpu_bus pending write requires cpu.pending_bus_write to be Some",
                );
                self.cpu_bus.drive(value);
                if self.drive_ppu_bus(address, value) {
                    self.interrupts.request(Interrupt::VideoStatus);
                }
                // Snapshot OAM/VRAM lock at CUPA-rising. AND'd with the
                // mid and commit samples — write blocks only if locked
                // throughout the entire CUPA window.
                self.cpu_bus
                    .record_snapshot_lock(self.ppu.write_lock(address));
            }

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

            if self.ppu.check_stat_edge() {
                self.interrupts.request(Interrupt::VideoStatus);
            }

            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
            self.cpu
                .dispatch
                .update_latch(self.interrupts.enabled, self.interrupts.requested);
        }

        // MOPA-rising fires any armed OAM bug.
        if tcycle.as_u8() == 2 {
            self.ppu.apply_pending_oam_bug();
        }

        self.clock_phase = ClockPhase::High;
        PhaseResult { new_screen, pixel }
    }

    /// Falling edge: PPU falling phase, interrupt latch capture,
    /// bus writes, M-cycle subsystems (serial, DMA, audio).
    fn fall(&mut self) -> PhaseResult {
        let mut new_screen = false;
        let tcycle = self.cpu.last_tcycle();
        let is_mcycle_boundary = self.cpu.at_mcycle_boundary();

        // Driver-enable edge (tobe↑ / wafu↑): the addressed peripheral
        // opens its tri-state driver. Mid-M-cycle flux propagates
        // combinationally to the latch edge below.
        if tcycle.as_u8() == 2 && let Some(address) = self.cpu_bus.pending_read() {
            let value = self.bus_value_at_drive_enable(address);
            self.cpu_bus.drive(value);
        }

        // PPU master-clock fall: divider chain, CATU, scanline
        // boundaries, fetcher, DFF8/DFF9, LCD-off.
        let oam_bus = self.dma.oam_bus_owner();
        let video_result = self.ppu.on_master_clock_fall(is_mcycle_boundary, oam_bus);

        // Mid-CUPA sample: catches the AJUJ-glitch window where AVAP
        // ends mode-2 mid-strobe and the rendering deferral leaves
        // mode2=0 ∧ mode3=0 observable here.
        if tcycle.as_u8() == 2 && let Some(address) = self.cpu_bus.mid_sample_pending() {
            self.cpu_bus.record_mid_lock(self.ppu.write_lock(address));
        }

        // CPU data latch (data_phase_n↑ near the end of T-cycle 3).
        // Resolves the drive-enable snapshot against mid-M-cycle flux
        // before the SM83 captures cpu_port_d.
        if let BusAction::Read { address } = &self.cpu.last_bus_action {
            let address = *address;
            let value = self.bus_value_at_latch(address, self.cpu_bus.data);
            self.cpu.data_latch = value;
            self.commit_bus_read(address, value);
        }

        // CPU writes commit at CUPA-falling (end of T-cycle 3). PPU
        // registers were already written at CUPA-rising via
        // drive_ppu_bus in rise(); this commits memory.
        match &self.cpu.last_bus_action {
            BusAction::Idle | BusAction::InternalOamBug { .. } | BusAction::Read { .. } => {}
            BusAction::Write { address, value: _ } => {
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

        // VBlank IF: POPU transitions happen here since the divider
        // chain runs in fall().
        if video_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }

        if self.ppu.check_stat_edge() {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        let (ns, pixel) = self.apply_ppu_result(&video_result);
        new_screen |= ns;

        if is_mcycle_boundary {
            // OAM DMA: transfer one byte per M-cycle.
            if let Some((src_addr, dst_offset)) = self.dma.mcycle() {
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

            self.external.tick_decay();
            self.audio.mcycle(self.timers.internal_counter());
        }

        // Re-capture interrupt state after bus writes and M-cycle
        // subsystems so IF mutations from CPU writes to 0xFF0F, STAT
        // edges from PPU register writes, and serial completion are all
        // visible by the time the next rise() ticks irq_latched.
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

    /// Process a PPU tick: draw the pixel, present on VSYNC (only if
    /// MEDA has pulsed since LCD-on), blank on LCD-off. Returns
    /// `(new_screen, pixel)` — `new_screen` fires only on VSYNC, never
    /// on LCD-off blank.
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
