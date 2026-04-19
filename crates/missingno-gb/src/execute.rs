use super::{
    BusAccess, BusAccessKind, ClockPhase, GameBoy, StagedPpuWrite,
    cpu::mcycle::DotAction,
    interrupts::Interrupt,
    is_ppu_register,
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
        let mut pending_oam_bug: Option<OamBugKind> = None;
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

            let result = self.execute_phase(&mut pending_oam_bug);
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
        let mut pending_oam_bug: Option<OamBugKind> = None;
        self.execute_phase(&mut pending_oam_bug)
    }

    /// Advance to the next dot (T-cycle) boundary — the next Low
    /// state. Executes 1 phase if clock is High, 2 if Low.
    /// Returns true if a new frame was produced.
    pub fn step_dot(&mut self) -> bool {
        let mut new_screen = false;
        let mut pending_oam_bug: Option<OamBugKind> = None;

        // Run phases until clock returns to Low (dot complete)
        loop {
            let result = self.execute_phase(&mut pending_oam_bug);
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
    fn execute_phase(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> PhaseResult {
        match self.clock_phase {
            ClockPhase::Low => self.rise(pending_oam_bug),
            ClockPhase::High => self.fall(pending_oam_bug),
        }
    }

    /// Rising edge: advance CPU state machine, capture bus reads,
    /// tick timer and PPU, fire OAM bugs.
    ///
    /// At M-cycle boundaries, the g42 DFF latches interrupt state from
    /// the previous M-cycle BEFORE the PPU's rising phase fires new IF
    /// bits. Then the PPU rise and interrupt capture run, updating
    /// interrupt_pending for the NEXT g42 latch. Finally, `next_dot`
    /// transitions the CPU, where dispatch checks gate on the just-
    /// latched g42 value.
    fn rise(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> PhaseResult {
        let mut new_screen = false;
        let mut pixel = None;
        let is_mcycle_boundary = !self.cpu.mcycle_active;

        // ── M-cycle boundary: g42 latch, then PPU + interrupt capture ──
        if is_mcycle_boundary {
            // Clear any staged PPU write from the previous M-cycle.
            self.staged_ppu_write = None;

            // Timer ticks once per M-cycle (BOGA).
            self.timers.mcycle();

            // g42 DFF: latch IF & IE from the PREVIOUS M-cycle.
            self.cpu.g42_was_pending = self.cpu.g42_interrupt_pending;
            self.cpu.g42_interrupt_pending = self.cpu.interrupt_pending;

            // PPU master-clock rising edge at the M-cycle boundary (dot 0).
            let ppu_result = self.ppu.on_master_clock_rise();
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

        // IE push bug: check after each M-cycle transition.
        if self.cpu.take_pending_vector_resolve() {
            if let Some(interrupt) = self.interrupts.triggered() {
                self.interrupts.clear(interrupt);
                self.cpu.bus_counter = interrupt.vector();
            } else {
                self.cpu.bus_counter = 0x0000;
            }
        }

        let dot = self.cpu.dot_for_execute();
        self.current_dot = dot;

        // Stage PPU register writes at dot 0. On hardware, the CPU
        // places the address on the bus at phase A and the address
        // decode chain begins propagating. The write is applied at
        // dot 1 rise (cupa fires at the 4th atal half-cycle).
        if is_mcycle_boundary && let Some((address, value)) = self.cpu.pending_bus_write() {
            if is_ppu_register(address) {
                self.staged_ppu_write = Some(StagedPpuWrite {
                    address,
                    value,
                    applied: false,
                });
            }
        }

        // BOWA (dot 0): record OAM bug from address in the upcoming action.
        if dot.bowa()
            && let DotAction::InternalOamBug { address } = &self.current_dot_action
            && (0xFE00..=0xFEFF).contains(address)
        {
            match pending_oam_bug {
                Some(OamBugKind::Read) => {}
                _ => {
                    *pending_oam_bug = Some(OamBugKind::Write);
                }
            }
        }

        // ── Non-boundary dots: PPU rise + interrupt capture AFTER CPU dot advance ──
        if !is_mcycle_boundary {
            // Apply staged PPU write at BusDot 1 rise. On hardware, cupa
            // fires at the 4th atal half-cycle when alet is LOW. Alet LOW
            // = alet has fallen = master clock has risen = our rise().
            // The spec (Section 11.5): "Write strobe fires when alet
            // falls" on the 2nd dot. The register latch becomes
            // transparent — combinational PPU logic sees the new value.
            // CLKPIPE fires later in this same rise() and sees the new
            // value. Alet-clocked DFFs (which capture in fall()) will
            // see the new value in THIS dot's fall().
            if dot.as_u8() == 1 {
                if let Some(staged) = self.staged_ppu_write.as_ref() {
                    if !staged.applied {
                        let (addr, val) = (staged.address, staged.value);
                        if self.drive_ppu_bus(addr, val) {
                            self.interrupts.request(Interrupt::VideoStatus);
                        }
                        self.staged_ppu_write.as_mut().unwrap().applied = true;
                    }
                }
            }

            // Snapshot LY==LYC comparison state before the PPU's
            // master-clock rising edge. ROPO latches LYC comparison at
            // TALU rising edge during on_master_clock_rise(). If the
            // comparison transitions to match, this is a TALU-cascade-
            // driven interrupt.
            let lyc_was_matched = self.ppu.ly_eq_lyc();

            // PPU master-clock rising edge for non-boundary dots.
            let ppu_result = self.ppu.on_master_clock_rise();
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

            // g42 mid-M-cycle cascade propagation: when VBlank or LYC
            // fires from the TALU cascade during PPU rise, g42 samples
            // IF&IE. If the new interrupt makes IF&IE non-zero, g42
            // captures it. On hardware, the cascade needs ~3 CLK9 edges
            // to propagate. Our emulator's divider alignment may place
            // these events at a different dot than hardware, so we
            // accept any non-boundary dot and let mcycle_halted use it
            // as the fast-path signal.
            //
            // Only VBlank and LYC (TALU-cascade-driven) qualify; HBlank
            // and timer arrive through different paths and are excluded.
            // The g42 DFF gates on IF&IE, not just IF — the interrupt
            // source must also be in IE.
            // VBlank fires from the TALU cascade. g42 captures IF&IE.
            // The VBlank event can trigger either the VBlank interrupt
            // (IF bit 0, if IE.vblank) or the STAT interrupt (IF bit 1,
            // if IE.stat and STAT VBlank mode flag set). Either makes
            // IF&IE non-zero, so g42 goes high for either path.
            if ppu_result.request_vblank
                && (self.interrupts.enabled(Interrupt::VideoBetweenFrames)
                    || (stat_edge && self.interrupts.enabled(Interrupt::VideoStatus)))
            {
                self.cpu.g42_mid_mcycle = true;
            }
            if !lyc_was_matched
                && self.ppu.ly_eq_lyc()
                && stat_edge
                && self.interrupts.enabled(Interrupt::VideoStatus)
            {
                self.cpu.g42_mid_mcycle = true;
            }
            // HBlank (mode 0) STAT edge: the ALET-settle cascade
            // (VOGA→XYMU→TARU→STAT→IF→g42) can propagate within the
            // same M-cycle if the edge fires early enough. On hardware,
            // g42 needs ~3 CLK9 edges (1.5 dots) to settle. Edges at
            // dots 0–1 have enough remaining time; dots 2–3 do not.
            // Only relevant during HALT — running instructions use the
            // DFF-latched g42 pipeline which correctly excludes this.
            if stat_edge
                && self.cpu.is_halted()
                && self.interrupts.enabled(Interrupt::VideoStatus)
                && self.ppu.mode() == ppu::rendering::Mode::HorizontalBlank
                && self.current_dot.as_u8() <= 1
            {
                self.cpu.g42_mid_mcycle = true;
            }

            // Capture interrupt state for non-boundary dots.
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
        }

        // VOGA capture (HBlank) happens in the PPU-clock-rise phase via
        // HblankPipeline::capture_voga() — confirmed by dmg-sim: VOGA is
        // clocked by ALET rising (= master-clock falling = PPU clock
        // rise). The mode() function uses `xymu && !wodu` to predict
        // HBlank state for CPU STAT reads, so settle_alet is not needed.
        // G4.2 confirmed WODU doesn't depend on XYMU, making the
        // prediction reliable.

        // g151: CLK9-clocked DFF delays timer overflow → IF by 1 dot.
        // Drain at every rising edge so that overflow detected at fall()
        // is visible to update_interrupt_state in the next fall().
        if let Some(interrupt) = self.timers.take_pending_interrupt() {
            self.interrupts.request(interrupt);
        }

        // MOPA rising edge (dot 2): fire OAM bug.
        if dot.mopa()
            && !dot.boga()
            && let Some(kind) = pending_oam_bug.take()
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
    fn fall(&mut self, pending_oam_bug: &mut Option<OamBugKind>) -> PhaseResult {
        let mut new_screen = false;
        let dot = self.current_dot;
        let is_mcycle_boundary = dot.boga();

        // PPU master-clock falling edge: divider chain (WUVU/VENA/TALU),
        // CATU, scanline boundaries, fetcher, DFF8/DFF9, LCD-off.
        let video_result = self
            .ppu
            .on_master_clock_fall(is_mcycle_boundary, &self.vram_bus.vram);

        // CPU data latch: capture bus value after PPU's master-clock
        // fall updates land. Hardware reads are combinational (spec
        // §10.6): the CPU samples the current DFF state via SM83-
        // internal data_phase. PPU DFF transitions on the same master-
        // clock cycle's TALU-rising edge (MYTA fire, ROPO capture) are
        // visible to the CPU read because they settle before the CPU's
        // data-phase latches. Placing the read after on_master_clock_fall
        // matches that ordering.
        if let DotAction::Read { address } = &self.current_dot_action {
            if (0xFE00..=0xFEFF).contains(address) {
                *pending_oam_bug = Some(OamBugKind::Read);
            }
            self.last_read_value = self.cpu_read(*address);
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

        // g42 mid-M-cycle cascade: events from the TALU cascade (now in
        // fall) need to propagate to g42 for HALT wakeup. VBlank and
        // LYC-driven STAT edges can both trigger g42.
        if video_result.request_vblank
            && (self.interrupts.enabled(Interrupt::VideoBetweenFrames)
                || (stat_edge && self.interrupts.enabled(Interrupt::VideoStatus)))
        {
            self.cpu.g42_mid_mcycle = true;
        }
        // STAT edges from TALU cascade (LYC, VBlank) and from HBlank
        // (VOGA→XYMU, confirmed by dmg-sim to fire in fall()) all need
        // g42 propagation for HALT wakeup. The HBlank edge was
        // previously handled by settle_alet in rise(), but now correctly
        // fires here in fall().
        if stat_edge
            && self.cpu.is_halted()
            && self.interrupts.enabled(Interrupt::VideoStatus)
        {
            // Check if HBlank just fired — mode transitioned to HBlank
            // in this fall(). Only propagate g42 early enough in the
            // M-cycle for the cascade to settle before the next boundary.
            let is_hblank = self.ppu.mode() == ppu::rendering::Mode::HorizontalBlank;
            let is_lyc = self.ppu.ly_eq_lyc();
            if is_lyc || (is_hblank && dot.as_u8() <= 1) {
                self.cpu.g42_mid_mcycle = true;
            }
        }

        // Capture interrupt state so HALT sees it.
        {
            let triggered = self.interrupts.triggered();
            self.cpu.update_interrupt_state(triggered);
        }

        let (ns, pixel) = self.apply_ppu_result(&video_result);
        new_screen |= ns;

        // Bus writes on the falling edge.
        match &self.current_dot_action {
            DotAction::Idle | DotAction::InternalOamBug { .. } | DotAction::Read { .. } => {}
            DotAction::Write { address, value } => {
                let address = *address;
                let value = *value;
                if (0xFE00..=0xFEFF).contains(&address) {
                    *pending_oam_bug = Some(OamBugKind::Write);
                }
                // Skip drive_ppu_bus if the staged write mechanism already
                // applied this write at the correct visibility dot.
                let already_applied = self.staged_ppu_write.as_ref().is_some_and(|s| s.applied);
                if !already_applied && self.drive_ppu_bus(address, value) {
                    self.interrupts.request(Interrupt::VideoStatus);
                }
                self.write_byte(address, value);
            }
        }

        if is_mcycle_boundary {
            // Serial ticks once per M-cycle.
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

        self.clock_phase = ClockPhase::Low;
        PhaseResult { new_screen, pixel }
    }

    /// Process a PPU tick result: write pixel to back buffer, present
    /// on frame boundary. Returns `(new_frame, pixel)`.
    fn apply_ppu_result(&mut self, result: &PpuTickResult) -> (bool, Option<ppu::PixelOutput>) {
        if let Some(pixel) = result.pixel {
            if pixel.x < ppu::screen::PIXELS_PER_LINE && pixel.y < ppu::screen::NUM_SCANLINES {
                self.screen
                    .draw_pixel(pixel.x, pixel.y, PaletteIndex(pixel.shade));
            }
        }
        if result.new_frame {
            self.screen.present();
            if let Some(sgb) = &mut self.sgb {
                sgb.update_screen(&self.screen);
            }
            return (true, result.pixel);
        }
        (false, result.pixel)
    }
}
