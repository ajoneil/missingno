//! Step loop for `GameBoyColor`.
//!
//! Copied from `missingno-gb`'s `impl GameBoy` step machinery with two
//! DMG-only paths removed:
//!
//! - **OAM corruption bug arming** — CGB hardware doesn't have the DMG
//!   OAM-corruption bug, so the `arm_oam_bugs` and
//!   `apply_pending_oam_bug` call sites are dropped.
//! - **SGB co-processor** — CGB doesn't host SGB; the `sgb` field and
//!   its screen-update hook are gone.
//!
//! Everything else is identical to the DMG step loop today. As CGB
//! features land (CGB palettes, banking, HDMA, KEY1 / double-speed),
//! this file diverges from the DMG version naturally.

use missingno_gb::{
    ClockPhase,
    cpu::{InterruptMasterEnable, mcycle::TCycle},
    cpu_bus::{BusAccess, BusAccessKind},
    execute::{PhaseResult, StepResult},
    interrupts::Interrupt,
    memory::Bus,
    ppu::{self, PpuTickResult},
};

use crate::{
    GameBoyColor,
    screen::{self, GREYSCALE_PALETTE},
};

impl GameBoyColor {
    pub fn step(&mut self) -> StepResult {
        self.step_traced(false).0
    }

    /// Step one instruction, optionally recording all bus accesses.
    pub fn step_traced(&mut self, trace: bool) -> (StepResult, Vec<BusAccess>) {
        if trace {
            self.bus_trace.enable();
        }

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

    fn step_instruction(&mut self) -> StepResult {
        let mut new_screen = false;
        self.cpu.data_latch = 0;

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

            if self.clock_phase == ClockPhase::Low {
                tcycles += 1;
                if self.cpu.at_instruction_boundary() {
                    break;
                }
            }
        }
        // step_traced accumulates the dirty flag via take_sram_dirty
        // across the (up to two) step_instruction calls; this inner
        // value is unused by callers.
        StepResult {
            new_screen,
            sram_dirty: false,
            tcycles,
        }
    }

    pub fn step_phase(&mut self) -> PhaseResult {
        self.execute_phase()
    }

    pub fn step_tcycle(&mut self) -> bool {
        let mut new_screen = false;
        loop {
            let result = self.execute_phase();
            new_screen |= result.new_screen;
            if self.clock_phase == ClockPhase::Low {
                break;
            }
        }
        self.cpu.take_instruction_boundary();
        new_screen
    }

    fn execute_phase(&mut self) -> PhaseResult {
        match self.clock_phase {
            ClockPhase::Low => self.rise(),
            ClockPhase::High => self.fall(),
        }
    }

    fn rise(&mut self) -> PhaseResult {
        let is_mcycle_boundary = self.cpu.consume_boundary_pending();
        let mut new_screen = false;
        let mut pixel = None;

        if is_mcycle_boundary {
            let (ns, pix) = self.tick_mcycle_boundary_rise();
            new_screen |= ns;
            pixel = pix;
        }

        self.cpu.next_tcycle();
        self.apply_vector_resolve();

        let tcycle = self.cpu.last_tcycle();
        self.step_dispatch_logic(tcycle);

        // APU prescaler tick (apuv ↑) on every master-clock rise.
        self.audio
            .tcycle(self.timers.internal_counter(), tcycle.as_u8());

        if is_mcycle_boundary {
            self.stage_mcycle_bus_activity();
        }
        if !is_mcycle_boundary {
            let (ns, pix) = self.tick_non_boundary_rise(tcycle);
            new_screen |= ns;
            if pixel.is_none() {
                pixel = pix;
            }
        }

        self.clock_phase = ClockPhase::High;
        PhaseResult { new_screen, pixel }
    }

    fn fall(&mut self) -> PhaseResult {
        let tcycle = self.cpu.last_tcycle();
        let is_mcycle_boundary = self.cpu.at_mcycle_boundary();

        // CH3's BUSA / AZUS DFFs latch on apu_4mhz ↑ (= our fall).
        self.audio.fall_sync();

        if tcycle.as_u8() == 2 {
            self.apply_read_drive_enable();
        }

        let oam_bus = self.dma.oam_bus_owner();
        let video_result = self.ppu.on_master_clock_fall(is_mcycle_boundary, oam_bus);

        if tcycle.as_u8() == 2 {
            self.sample_mid_cupa_lock();
        }

        self.commit_read_latch();
        self.commit_write();

        if video_result.request_vblank {
            self.interrupts.request(Interrupt::VideoBetweenFrames);
        }
        if self.ppu.check_stat_edge() {
            self.interrupts.request(Interrupt::VideoStatus);
        }

        let (new_screen, pixel) = self.apply_ppu_result(&video_result);

        // OAM DMA control gates clock on dma_phi = !data_phase; tick every
        // master-clock edge so the engage/arm edges are both seen. data_phase
        // is held LOW during halt-spin, freezing the engine.
        let data_phase = !self.cpu.halt_rs_latched() && matches!(tcycle.as_u8(), 2 | 3);
        self.drive_dma(data_phase);

        if is_mcycle_boundary {
            self.tick_mcycle_boundary_fall();
        }

        self.recapture_interrupts();
        self.clock_phase = ClockPhase::Low;
        PhaseResult { new_screen, pixel }
    }

    fn tick_mcycle_boundary_rise(&mut self) -> (bool, Option<ppu::PixelOutput>) {
        self.cpu.tick_irq_latched();

        self.cpu.dispatch.set_data_phase_n(true);
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
        self.cpu.dispatch.tick_zacw();

        self.cpu.irq.ime.write_immediate(if self.cpu.irq.ime_delay {
            InterruptMasterEnable::Enabled
        } else {
            InterruptMasterEnable::Disabled
        });

        self.cpu_bus.clear_activity();

        self.timers.mcycle();
        if let Some(interrupt) = self.timers.take_pending_interrupt() {
            self.interrupts.request(interrupt);
        }

        let counter = self.timers.internal_counter();
        if let Some(interrupt) = self.serial.mcycle(counter) {
            self.interrupts.request(interrupt);
        }

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

    fn tick_non_boundary_rise(&mut self, tcycle: TCycle) -> (bool, Option<ppu::PixelOutput>) {
        self.ppu.snapshot_pre_cupa_lcdc();

        if tcycle.as_u8() == 2 && let Some(address) = self.cpu_bus.pending_write() {
            let value = self
                .cpu
                .pending_bus_write()
                .map(|(_, v)| v)
                .expect("cpu_bus pending write requires cpu.pending_bus_write to be Some");
            self.cpu_bus.drive(value);
            if self.drive_ppu_bus(address, value) {
                self.interrupts.request(Interrupt::VideoStatus);
            }
            self.cpu_bus
                .record_snapshot_lock(self.ppu.write_lock(address));
        }

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
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);

        (new_screen, pixel)
    }

    fn apply_vector_resolve(&mut self) {
        if self.cpu.take_pending_vector_resolve() {
            if let Some(interrupt) = self.cpu.dispatch.vector() {
                self.interrupts.clear(interrupt);
                self.cpu.pc = interrupt.vector();
            } else {
                self.cpu.pc = 0x0000;
            }
            self.cpu.dispatch.clear_dispatch();
        }
    }

    fn step_dispatch_logic(&mut self, tcycle: TCycle) {
        if tcycle.as_u8() == 2 && !self.cpu.halt_rs_latched() {
            self.cpu.dispatch.set_data_phase_n(false);
        }

        let halt_body = self.cpu.is_halted() && !self.cpu.halt_rs_latched();
        let halt_spin = self.cpu.halt_rs_latched();
        let data_phase = !halt_spin && (tcycle.as_u8() == 2 || tcycle.as_u8() == 3);
        let write_phase = !halt_spin && tcycle.as_u8() == 3;
        let ctl_fetch = self.cpu.is_fetch_phase() || halt_body;
        let xogs = (data_phase && ctl_fetch) || halt_spin;
        let ime_enabled = self.cpu.irq.ime.output() == InterruptMasterEnable::Enabled;
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
        self.cpu
            .dispatch
            .step_zkog(ime_enabled, data_phase, write_phase, xogs);
    }

    fn stage_mcycle_bus_activity(&mut self) {
        if let Some((address, _value)) = self.cpu.pending_bus_write() {
            self.cpu_bus.stage_write(address);
        } else if let Some(address) = self.cpu.pending_bus_read() {
            self.cpu_bus.stage_read(address);
        }
    }

    fn apply_read_drive_enable(&mut self) {
        if let Some(address) = self.cpu_bus.pending_read() {
            let value = self.bus_value_at_drive_enable(address);
            self.cpu_bus.drive(value);
        }
    }

    fn sample_mid_cupa_lock(&mut self) {
        if let Some(address) = self.cpu_bus.mid_sample_pending() {
            self.cpu_bus.record_mid_lock(self.ppu.write_lock(address));
        }
    }

    fn commit_read_latch(&mut self) {
        use missingno_gb::cpu::mcycle::BusAction;
        if let BusAction::Read { address } = &self.cpu.last_bus_action {
            let address = *address;
            let value = self.bus_value_at_latch(address, self.cpu_bus.data);
            self.cpu.data_latch = value;
            self.commit_bus_read(address, value);
        }
    }

    fn commit_write(&mut self) {
        use missingno_gb::cpu::mcycle::BusAction;
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

        self.external.tick_decay();
    }

    /// Advance the OAM-DMA control gates one master-clock edge. The byte
    /// transfer commits at the M-cycle data phase in `tick_mcycle_boundary_fall`.
    fn drive_dma(&mut self, data_phase: bool) {
        self.dma.tick(data_phase);
    }

    fn recapture_interrupts(&mut self) {
        let triggered = self.interrupts.triggered();
        self.cpu.update_interrupt_state(triggered);
        self.cpu
            .dispatch
            .update_latch(self.interrupts.enabled, self.interrupts.requested);
    }

    /// Process a PPU tick: draw the pixel, present on VSYNC (only if
    /// MEDA has pulsed since LCD-on), blank on LCD-off.
    ///
    /// The DMG PPU emits a 2-bit shade index (post-BGP) per pixel; on
    /// CGB hardware the PPU's palette lookup yields 15-bit RGB. Until
    /// CGB palette memory and the CGB palette path land, we map the
    /// shade through [`GREYSCALE_PALETTE`].
    fn apply_ppu_result(&mut self, result: &PpuTickResult) -> (bool, Option<ppu::PixelOutput>) {
        if let Some(pixel) = result.pixel
            && pixel.x < screen::PIXELS_PER_LINE
            && pixel.y < screen::NUM_SCANLINES
        {
            let rgb = GREYSCALE_PALETTE[(pixel.shade & 0x3) as usize];
            self.screen.draw_pixel(pixel.x, pixel.y, rgb);
        }
        if result.new_frame {
            if self.ppu.control().video_enabled() && self.ppu.vsync_committed() {
                self.screen.present();
            }
            return (true, result.pixel);
        }
        if result.lcd_disabled {
            self.screen.blank();
        }
        (false, result.pixel)
    }
}
