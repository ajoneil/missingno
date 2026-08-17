//! Read-only inspection views of the console for the debugger panes.
//!
//! The UI cannot touch the core while it runs on the emulation thread, so the
//! seam copies the pane-relevant state into a [`GbSnapshot`] and the panes
//! render from that. The section builders read the [`CpuSource`]/[`PpuSource`]
//! traits, so one body serves a capture and the live console the tests hold it
//! against.

use std::sync::Arc;

use missingno_core::cdl::CdlWindow;
use missingno_core::inspect;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{InspectSnapshot, RunningStatus};

use crate::cartridge::CartridgeView;
use crate::interrupts;
use crate::{Console, Model};

mod audio;
mod cartridge;
mod cpu;
mod ppu;
mod timers;

pub use audio::{AudioView, NoiseChannelView, PulseChannelView, WaveChannelView, apu_section};
pub use cartridge::cartridge_section;
pub use cpu::{CpuSource, CpuView, cpu_blocks, cpu_register_groups, cpu_summary};
pub use ppu::{
    ColorSnapshot, PpuSource, PpuView, dmg_background_swatches, dmg_fifo_block,
    dmg_object_swatches, mode_label, ppu_background_block, ppu_detail, ppu_position_block,
    ppu_sprites_block, ppu_status_block, ppu_summary, ppu_window_block,
};
pub use timers::{TimersView, timers_section};

// A system composes its own `Vec<Section>` from these shared part-builders
// over the CpuSource/PpuSource surfaces, deciding its own section summaries,
// activity, and where its console-specific content sits. DMG composes with
// `dmg_sidebar_sections`; CGB composes in `missingno-gbc` from the same parts
// plus its colour state.

/// The DMG sidebar: CPU, PPU, Timers, APU and Cartridge sections composed from
/// the shared parts, with the DMG shade swatches sat with the registers they
/// describe. Shared by the live console (paused) and the running snapshot so
/// the two agree.
pub fn dmg_sidebar_sections(
    cpu: &impl CpuSource,
    ppu: &impl PpuSource,
    ints: &interrupts::Registers,
    timers: &TimersView,
    audio: &AudioView,
    cart: &CartridgeView,
) -> Vec<inspect::Section> {
    use inspect::SectionBlock::Rule;

    vec![
        inspect::Section {
            name: "CPU",
            summary: cpu_summary(cpu),
            active: Some(!cpu.halted()),
            detail: None,
            blocks: cpu_blocks(cpu, ints),
        },
        inspect::Section {
            name: "PPU",
            summary: ppu_summary(ppu),
            active: Some(ppu.control().video_enabled()),
            detail: Some(ppu_detail(ppu)),
            blocks: vec![
                ppu_position_block(ppu),
                ppu_status_block(ppu),
                Rule,
                ppu_background_block(ppu),
                dmg_background_swatches(ppu),
                Rule,
                ppu_window_block(ppu),
                Rule,
                ppu_sprites_block(ppu),
                dmg_object_swatches(ppu),
                Rule,
                dmg_fifo_block(ppu),
            ],
        },
        timers_section(timers),
        apu_section(audio),
        cartridge_section(cart),
    ]
}

/// A per-vblank copy of the model-shared debugger state, taken on the
/// emulation thread while the core runs there. The CGB build wraps this with
/// its extra register view.
#[derive(Clone)]
pub struct GbSnapshot {
    pub cpu: CpuView,
    pub ppu: PpuView,
    pub audio: AudioView,
    pub timers: TimersView,
    pub interrupts: interrupts::Registers,
    pub colors: ColorSnapshot,
    pub switchable_rom_bank: Option<u16>,
    pub cartridge: CartridgeView,
    pub symbols: Arc<SymbolTable>,
    pub cdl: CdlWindow,
    pub frame: u64,
}

impl GbSnapshot {
    pub fn capture<M: Model>(
        console: &Console<M>,
        colors: ColorSnapshot,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> Self {
        Self {
            cpu: CpuView::capture(console.cpu()),
            ppu: PpuView::capture(console.ppu()),
            audio: AudioView::capture(console.audio()),
            timers: TimersView::capture(console.timers()),
            interrupts: console.interrupts().clone(),
            colors,
            switchable_rom_bank: console.cartridge().switchable_rom_bank(),
            cartridge: console.cartridge().inspect(),
            symbols,
            cdl,
            frame,
        }
    }

    /// This capture stamped with the UI's frame counter.
    pub fn at_frame(&self, frame: u64) -> Self {
        Self {
            frame,
            ..self.clone()
        }
    }

    /// The one-line summary the frontend shows while the core runs.
    pub fn running_status(&self, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: self.cpu.ir_address.into(),
            sp: self.cpu.stack_pointer.into(),
            video_label: "PPU",
            video_summary: format!("{} · ly {}", mode_label(self.ppu.mode), self.ppu.ly),
            frame,
        }
    }
}

impl InspectSnapshot for GbSnapshot {
    fn frame(&self) -> u64 {
        self.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        self
    }
    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        cpu_register_groups(&self.cpu)
    }
    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        dmg_sidebar_sections(
            &self.cpu,
            &self.ppu,
            &self.interrupts,
            &self.timers,
            &self.audio,
            &self.cartridge,
        )
    }
    fn pc(&self) -> Option<u32> {
        Some(self.cpu.ir_address as u32)
    }
    fn symbols(&self) -> Option<&SymbolTable> {
        Some(&self.symbols)
    }
    fn cdl_window(&self) -> Option<&CdlWindow> {
        Some(&self.cdl)
    }
    fn bank_for(&self, address: u32) -> Option<u16> {
        match address {
            0x4000..=0x7FFF => self.switchable_rom_bank,
            _ => None,
        }
    }
    fn instruction_set(&self) -> Option<&dyn missingno_core::isa::InstructionSet> {
        Some(&crate::isa::Sm83)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;
    use crate::debugger::Debugger;

    pub(super) fn stepped_dmg() -> Debugger<crate::Dmg> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut debugger = Debugger::new(Console::<crate::Dmg>::new(
            Cartridge::new(rom, None, None).unwrap(),
            None,
        ));
        for _ in 0..4 {
            debugger.step();
        }
        debugger
    }

    pub(super) fn row_labels(section: &inspect::Section) -> Vec<String> {
        section
            .blocks
            .iter()
            .flat_map(|block| match block {
                inspect::SectionBlock::Rows(rows) => rows.iter().map(|r| r.label.clone()).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    #[test]
    fn snapshot_register_groups_match_live() {
        let debugger = stepped_dmg();
        let live = debugger.register_groups();
        let snapshot = GbSnapshot::capture(
            debugger.game_boy(),
            ColorSnapshot::Dmg { sgb: false },
            0,
            Arc::new(SymbolTable::default()),
            CdlWindow::default(),
        );
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", snapshot.register_groups())
        );
    }

    #[test]
    fn snapshot_sidebar_sections_match_live() {
        let debugger = stepped_dmg();
        let console = debugger.game_boy();
        let audio = AudioView::capture(console.audio());
        let timers = TimersView::capture(console.timers());
        let cart = console.cartridge().inspect();
        let live = dmg_sidebar_sections(
            console.cpu(),
            console.ppu(),
            console.interrupts(),
            &timers,
            &audio,
            &cart,
        );
        let snapshot = GbSnapshot::capture(
            console,
            ColorSnapshot::Dmg { sgb: false },
            0,
            Arc::new(SymbolTable::default()),
            CdlWindow::default(),
        );
        assert_eq!(
            format!("{live:?}"),
            format!("{:?}", snapshot.sidebar_sections())
        );
    }
}
