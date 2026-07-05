use crate::{Console, ConsoleShadow, Model, cpu::mcycle::BusAction};

impl<M: Model> Console<M> {
    /// Driver-enable edge (tobe↑ / wafu↑) at T-cycle 2: the addressed
    /// peripheral opens its tri-state driver. Mid-M-cycle flux
    /// propagates combinationally to the latch edge in `commit_read_latch`.
    pub(super) fn apply_read_drive_enable(&mut self) {
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
    pub(super) fn sample_mid_cupa_lock(&mut self) {
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
    pub(super) fn commit_read_latch(&mut self, ly_at_latch: Option<u8>) {
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
    pub(super) fn commit_write(&mut self) {
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
}
