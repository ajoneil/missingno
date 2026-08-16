use crate::{
    Console, ConsoleShadow, Model,
    cpu::mcycle::TCycle,
    cpu_bus::{BusAccess, BusAccessKind},
    ppu,
};

impl<M: Model> Console<M> {
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
    pub(super) fn dma_move(&mut self, source: u16, dest: u16) {
        let byte = self.read_dma_source(source);
        self.chassis.dma_commit(source, dest, byte);
    }

    /// HDMA trigger, evaluated each dot's fall with this fall's write
    /// commit visible: the pend forms on the post-rise mode view and
    /// commits to cancel-immunity one fall later (the pend pipeline
    /// lives in the model).
    pub(super) fn tick_vram_dma_trigger(&mut self, dot_work: bool, pre_fall_mode: ppu::Mode) {
        if dot_work {
            // The engine thaws at the IF rise, ahead of the CPU's halt-exit
            // latency (a wake-coincident block is decided before the first
            // fetch and the dispatch pick); level re-evaluation and the
            // taken-clear wait for the CPU's own resume. The model owns the
            // trigger pipeline and hands back its committed bus claim.
            self.model.vram_dma_edge(&mut self.chassis, pre_fall_mode);
        }
    }

    /// OAM DMA control gates clock on dma_phi = !data_phase; tick
    /// every master-clock edge so the engage (dma_phi rising) and arm
    /// (dma_phi_n rising) edges are both seen. data_phase is held LOW
    /// during halt-spin, freezing the engine (MATU/counter get no edge).
    pub(super) fn clock_oam_dma_gate(&mut self, tcycle: TCycle) {
        let data_phase = !self.chassis.cpu.halt_rs_latched() && matches!(tcycle.as_u8(), 2 | 3);
        self.drive_dma(data_phase);
    }

    /// M-cycle-boundary work on the falling edge (data phase): commit the
    /// OAM DMA byte for this M-cycle, plus external-bus decay. A CPU write
    /// that collided with DMA on the source bus open-drains at the OAM
    /// slot DMA deposits. (Audio mcycle is at boundary rise.)
    pub(super) fn tick_mcycle_boundary_fall(&mut self) {
        let oam = self.chassis.dma.peek_transfer();
        // The CGB VRAM DMA arbitrates the shared bus before the OAM byte moves:
        // it may take this M-cycle's OAM deposit (single-speed contention) or
        // stall the OAM engine (a double-speed switch-cancel escape byte). DMG:
        // never suppresses.
        let suppress_oam = self.model.vram_dma_arbitrate_oam(&mut self.chassis);
        if !suppress_oam && let Some((src_addr, dst_offset)) = oam {
            self.dma_move(src_addr, 0xfe00 + dst_offset as u16);
        }

        // A source-bank register write (VBK/SVBK) latches here at the boundary,
        // after the coincident byte's source read above reads the pre-write
        // bank. Reuses the CPU write-commit path (map_write); no-op on the DMG.
        if let Some(crate::DmaBankWrite { address, value }) =
            self.chassis.dma_conflict.pending_bank_write.take()
        {
            self.write_byte_with_write_strobe_lock(address, value, None, None);
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
}
