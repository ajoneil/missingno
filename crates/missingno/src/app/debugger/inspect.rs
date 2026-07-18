//! The debugger panes' inspection surface. The GUI-free data views live in
//! `missingno_gb::debugger::inspection` (and the CGB register view in
//! `missingno_gbc`); this module re-exports them and layers the pane-facing
//! [`InspectSource`] surface — the live console while paused, or a per-vblank
//! snapshot while the core runs on the emulation thread — on top.

use std::any::Any;

use missingno_gb::debugger::instructions::ReadInstructionMemory;
use missingno_gb::interrupts;
use missingno_gb::ppu::{memory::VramView, types::palette::Palette};
use missingno_gb::{Console, Dmg, GameBoy, Model};
use missingno_gbc::{Cgb, GameBoyColor, cram_palettes};

use crate::app::console::{ConsoleColors, colors_from_snapshot};

pub use missingno_core::cdl::CdlWindow;
pub use missingno_core::symbols::SymbolTable;
pub use missingno_core::system::DebugView;
pub use missingno_gb::debugger::inspection::{
    AudioView, CpuSource, EnvelopeChannelView, GbSnapshot, PpuSource, WaveChannelView,
};
pub use missingno_gbc::{CgbSnapshot, CgbView};

// --- Model colour hooks ------------------------------------------------------

/// How each model resolves the debugger's render palettes and its CGB
/// register view — the model-specific, iced-adjacent slice of inspection that
/// stays frontend-side.
pub trait GbColors: Model {
    fn colors(console: &Console<Self>, user_palette: &Palette) -> ConsoleColors;
    fn cgb(console: &Console<Self>) -> Option<CgbView>;
}

impl GbColors for Dmg {
    fn colors(console: &Console<Self>, user_palette: &Palette) -> ConsoleColors {
        ConsoleColors::Dmg {
            palette: if console.sgb().is_some() {
                Palette::CLASSIC
            } else {
                *user_palette
            },
        }
    }

    fn cgb(_console: &Console<Self>) -> Option<CgbView> {
        None
    }
}

impl GbColors for Cgb {
    fn colors(console: &Console<Self>, _user_palette: &Palette) -> ConsoleColors {
        let ppu = console.ppu().model();
        ConsoleColors::Cgb {
            background: cram_palettes(|palette, index| ppu.bg_color(palette, index)),
            objects: cram_palettes(|palette, index| ppu.obj_color(palette, index)),
        }
    }

    fn cgb(console: &Console<Self>) -> Option<CgbView> {
        Some(CgbView::capture(console))
    }
}

// --- Inspection source -------------------------------------------------------

/// Everything the debugger panes and sidebar render from, behind one
/// model-erased surface: the live console while paused, or the per-vblank
/// snapshot while the core runs on the emulation thread.
pub trait InspectSource {
    fn cpu(&self) -> &dyn CpuSource;
    fn ppu(&self) -> &dyn PpuSource;
    fn vram(&self) -> &dyn VramView;
    fn audio(&self) -> AudioView;
    fn interrupts(&self) -> interrupts::Registers;
    fn instruction_memory(&self) -> &dyn ReadInstructionMemory;
    fn colors(&self, user_palette: &Palette) -> ConsoleColors;
    /// CGB register state for the sidebar; `None` on DMG.
    fn cgb(&self) -> Option<CgbView>;
    /// The 16KB ROM bank mapped at 0x4000–0x7FFF, for symbol resolution.
    fn switchable_rom_bank(&self) -> Option<u16>;
}

impl<M: GbColors> InspectSource for Console<M> {
    fn cpu(&self) -> &dyn CpuSource {
        Console::cpu(self)
    }
    fn ppu(&self) -> &dyn PpuSource {
        Console::ppu(self)
    }
    fn vram(&self) -> &dyn VramView {
        Console::vram(self)
    }
    fn audio(&self) -> AudioView {
        AudioView::capture(Console::audio(self))
    }
    fn interrupts(&self) -> interrupts::Registers {
        Console::interrupts(self).clone()
    }
    fn instruction_memory(&self) -> &dyn ReadInstructionMemory {
        self
    }
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        M::colors(self, user_palette)
    }
    fn cgb(&self) -> Option<CgbView> {
        M::cgb(self)
    }
    fn switchable_rom_bank(&self) -> Option<u16> {
        self.cartridge().switchable_rom_bank()
    }
}

impl InspectSource for GbSnapshot {
    fn cpu(&self) -> &dyn CpuSource {
        &self.cpu
    }
    fn ppu(&self) -> &dyn PpuSource {
        &self.ppu
    }
    fn vram(&self) -> &dyn VramView {
        &*self.vram
    }
    fn audio(&self) -> AudioView {
        self.audio.clone()
    }
    fn interrupts(&self) -> interrupts::Registers {
        self.interrupts.clone()
    }
    fn instruction_memory(&self) -> &dyn ReadInstructionMemory {
        &self.memory
    }
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        colors_from_snapshot(&self.colors, user_palette)
    }
    fn cgb(&self) -> Option<CgbView> {
        None
    }
    fn switchable_rom_bank(&self) -> Option<u16> {
        self.switchable_rom_bank
    }
}

impl InspectSource for CgbSnapshot {
    fn cpu(&self) -> &dyn CpuSource {
        self.base.cpu()
    }
    fn ppu(&self) -> &dyn PpuSource {
        self.base.ppu()
    }
    fn vram(&self) -> &dyn VramView {
        self.base.vram()
    }
    fn audio(&self) -> AudioView {
        self.base.audio()
    }
    fn interrupts(&self) -> interrupts::Registers {
        self.base.interrupts()
    }
    fn instruction_memory(&self) -> &dyn ReadInstructionMemory {
        self.base.instruction_memory()
    }
    fn colors(&self, user_palette: &Palette) -> ConsoleColors {
        self.base.colors(user_palette)
    }
    fn cgb(&self) -> Option<CgbView> {
        Some(self.cgb.clone())
    }
    fn switchable_rom_bank(&self) -> Option<u16> {
        self.base.switchable_rom_bank()
    }
}

/// The Game Boy family's typed pane surface: its trait-erased inspection
/// source plus the GB-typed sidecar and colour context its panes render
/// with. Bundled so shared pane plumbing carries one optional GB field.
#[derive(Clone, Copy)]
pub struct GbPaneContext<'b> {
    pub source: &'b dyn InspectSource,
    pub colors: &'b ConsoleColors,
    pub symbols: &'b SymbolTable,
    pub cdl: &'b CdlWindow,
}

/// Recover the Game Boy inspection surface from a family-erased state — the
/// live console while paused, or its snapshot while running. `None` for any
/// other family, whose own panes downcast their typed state.
pub fn as_inspect_source(state: &dyn Any) -> Option<&dyn InspectSource> {
    if let Some(console) = state.downcast_ref::<GameBoy>() {
        Some(console as &dyn InspectSource)
    } else if let Some(console) = state.downcast_ref::<GameBoyColor>() {
        Some(console as &dyn InspectSource)
    } else if let Some(snapshot) = state.downcast_ref::<GbSnapshot>() {
        Some(snapshot as &dyn InspectSource)
    } else {
        state
            .downcast_ref::<CgbSnapshot>()
            .map(|snapshot| snapshot as &dyn InspectSource)
    }
}
