//! Restoring the colour console's delta from a save state taken at an
//! instruction boundary.

use missingno_core::state::{StateRecord, StateValue};
use missingno_core::system::StateError;

use missingno_gb::Chassis;
use missingno_gb::clock::CpuDivider;
use missingno_gb::ppu::memory::Vram;

use crate::vram_dma::{TransferMode, VramDma};
use crate::{Cgb, CgbConsoleState};

/// A double-speed save carries no boundary-observable dot-phase alignment
/// (the free-running dot clock's parity a speed switch left is Tier-2b
/// state); reconstructing it would be a guess, so refuse the restore.
pub(crate) fn check_double_speed(record: &StateRecord) -> Result<(), StateError> {
    if let Some(StateValue::Bool(true)) = record.get("double_speed") {
        return Err(StateError::DoubleSpeedBoundary);
    }
    Ok(())
}

impl Cgb {
    /// CGB work RAM lives in the model's eight banks, not the shared bus.
    pub(crate) fn restore_wram_banks(&mut self, bytes: &[u8]) {
        let len = bytes.len().min(self.wram.len());
        self.wram[..len].copy_from_slice(&bytes[..len]);
    }

    pub(crate) fn restore_delta(
        &mut self,
        chassis: &mut Chassis<Self>,
        record: &StateRecord,
        memory: &[(String, Vec<u8>)],
    ) -> Result<(), StateError> {
        let int = |name: &str| -> Result<u32, StateError> {
            match record.get(name) {
                Some(StateValue::Int(v)) => Ok(*v),
                _ => Err(StateError::Corrupt),
            }
        };
        let flag = |name: &str| -> Result<bool, StateError> {
            match record.get(name) {
                Some(StateValue::Bool(b)) => Ok(*b),
                _ => Err(StateError::Corrupt),
            }
        };
        let region = |name: &str| -> [u8; 64] {
            let mut out = [0u8; 64];
            if let Some((_, data)) = memory.iter().find(|(n, _)| n == name) {
                let len = data.len().min(64);
                out[..len].copy_from_slice(&data[..len]);
            }
            out
        };

        // Parse every field the delta needs BEFORE mutating any state — a bad or
        // missing field then leaves the console untouched rather than
        // half-restored.
        let svbk = (int("svbk")? as u8) & 0x07;
        let vbk = int("vbk")? as u8;
        let bcps = int("bcps")? as u8;
        let ocps = int("ocps")? as u8;
        let opri = int("opri")? as u8;
        let key1_armed = flag("key1_armed")?;
        let ff72 = int("ff72")? as u8;
        let ff73 = int("ff73")? as u8;
        let ff74 = int("ff74")? as u8;
        let ff75 = int("ff75")? as u8;
        let hdma_active = flag("hdma_active")?;
        let hdma_hblank = flag("hdma_hblank")?;
        let (hdma_source, hdma_dest, hdma_remaining) = if hdma_active && hdma_hblank {
            (
                int("hdma_source")? as u16,
                int("hdma_dest")? as u16,
                (int("hdma_remaining")? as u16) * 16,
            )
        } else {
            (0, 0, 0)
        };
        let bg = region("cram_bg");
        let obj = region("cram_obj");
        let extra_oam = {
            let mut out = [0u8; 24];
            if let Some((_, data)) = memory.iter().find(|(n, _)| n == "extra_oam") {
                let len = data.len().min(24);
                out[..len].copy_from_slice(&data[..len]);
            }
            out
        };

        // Everything parsed: now mutate. Speed is single-speed only (double speed
        // refused in validate_boundary); reset the KEY1/blackout transients to
        // their idle values, but honour the captured arm bit.
        self.double_speed = false;
        self.key1_armed = key1_armed;
        self.speed_switch_blackout = 0;
        self.speed_switch_wake_latency = None;
        self.switch_relock_debit = false;
        self.pre_alet_rendering = false;
        self.pre_alet_lock = None;
        self.read_drive_oam_lock = None;
        self.ff44_ripple_old = None;
        self.console_state = CgbConsoleState::default();
        chassis.clock.set_divider(CpuDivider::One);

        // Undocumented scratch registers and the extra OAM RAM.
        self.ff72 = ff72;
        self.ff73 = ff73;
        self.ff74 = ff74;
        self.ff75 = ff75;
        self.extra_oam = extra_oam;

        // Work-RAM bank select (SVBK) and the VRAM bank select (VBK).
        self.svbk = svbk;
        chassis.vram_bus.vram.write_bank_select(vbk);

        // Palette RAM and its index/priority registers.
        let dmg_compat = self.dmg_compat;
        chassis
            .ppu
            .model_mut()
            .restore_boundary(bg, obj, bcps, ocps, opri, dmg_compat);

        // VRAM-DMA engine: rebuild the cursor for an armed HBlank transfer (a
        // GDMA holds the CPU for its whole run, so it cannot straddle a
        // boundary). The arbitration transients reset to idle — at a boundary no
        // block is in flight.
        self.vram_dma = VramDma::default();
        if hdma_active && hdma_hblank {
            self.vram_dma.cursor.mode = TransferMode::HBlank;
            self.vram_dma.cursor.source = hdma_source;
            self.vram_dma.cursor.dest = hdma_dest;
            self.vram_dma.cursor.remaining = hdma_remaining;
        }

        Ok(())
    }
}
