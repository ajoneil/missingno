//! The Game Boy Color model's implementation of the system seam's display and
//! debugger hooks: colour screen framing, the CGB register view the sidebar
//! draws, and the per-vblank snapshot that carries it.

use std::any::Any;
use std::sync::Arc;

use missingno_core::cdl::CdlWindow;
use missingno_core::inspect;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{DebugView, InspectSnapshot};
use missingno_core::video::{Frame, RgbaFrame};

use missingno_gb::Console;
use missingno_gb::debugger::inspection::{
    self as parts, ColorSnapshot, CpuSource, GbSnapshot, PpuSource,
};
use missingno_gb::frame::NATIVE_SIZE;
use missingno_gb::ppu::types::palette::Palette;
use missingno_gb::system::ConsoleUi;

use crate::screen::Color555;
use crate::{Cgb, GameBoyColor, VramDmaStatus};

/// The 8 corrected display palettes of one CGB palette RAM.
pub fn cram_palettes(color: impl Fn(u8, u8) -> Color555) -> [Palette; 8] {
    std::array::from_fn(|palette| {
        Palette::new(std::array::from_fn(|index| {
            color(palette as u8, index as u8).to_corrected_rgb8()
        }))
    })
}

/// The CGB-only register state the sidebar draws — absent on DMG. Plain data,
/// read live when paused or copied into the snapshot while the core runs.
#[derive(Clone)]
pub struct CgbView {
    /// KEY1 speed bit: running at double speed.
    pub double_speed: bool,
    /// VBK ($FF4F) bank select.
    pub vram_bank: u8,
    /// Effective SVBK ($FF70) work-RAM bank.
    pub wram_bank: u8,
    /// OPRI ($FF6C) object-priority register.
    pub opri: u8,
    /// BCPS ($FF68) background palette index.
    pub bcps: u8,
    /// OCPS ($FF6A) object palette index.
    pub ocps: u8,
    /// VRAM-DMA (HDMA/GDMA) engine state.
    pub vram_dma: VramDmaStatus,
}

impl CgbView {
    pub fn capture(console: &GameBoyColor) -> Self {
        let model = console.model();
        let ppu = console.ppu();
        let (bcps, ocps) = ppu.model().palette_index_registers();
        Self {
            double_speed: model.double_speed(),
            vram_bank: console.vram().selected_bank(),
            wram_bank: model.wram_bank(),
            opri: ppu.read_object_priority(),
            bcps,
            ocps,
            vram_dma: model.vram_dma_status(),
        }
    }
}

/// The CGB sidebar: CPU and PPU sections composed from the shared Game Boy
/// part-builders plus the colour console's own state, folded into the hardware
/// each field describes — KEY1 speed and WRAM banking in the CPU section;
/// VRAM banking, palette registers, HDMA, and CRAM swatches in the PPU section
/// — rather than a standalone section. Shared by the live console (paused) and
/// the running snapshot so the two agree by construction.
pub fn cgb_sidebar_sections(
    cpu: &impl CpuSource,
    ppu: &impl PpuSource,
    ints: &missingno_gb::interrupts::Registers,
    view: &CgbView,
    background: &[Palette; 8],
    objects: &[Palette; 8],
) -> Vec<inspect::Section> {
    use inspect::{Row, Section, SectionBlock};

    let speed = if view.double_speed { "2x" } else { "1x" };
    let mut cpu_content = parts::cpu_blocks(cpu, ints);
    cpu_content.push(SectionBlock::Rows(vec![
        Row::value("speed", speed),
        Row::value("svbk", view.wram_bank.to_string()),
    ]));

    let ppu_content = vec![
        parts::ppu_position_block(ppu),
        SectionBlock::Rule,
        parts::ppu_background_block(ppu),
        SectionBlock::Rule,
        parts::ppu_window_block(ppu),
        SectionBlock::Rule,
        parts::ppu_sprites_block(ppu),
        SectionBlock::Rows(vec![
            Row::value("vbk", view.vram_bank.to_string()),
            Row::value("opri", format!("{:02X}", view.opri)),
            Row::value("bcps", format!("{:02X}", view.bcps)),
            Row::value("ocps", format!("{:02X}", view.ocps)),
            Row::value("hdma", hdma_status(view.vram_dma)),
        ]),
        SectionBlock::Swatches(cram_swatches("bg", background)),
        SectionBlock::Swatches(cram_swatches("obj", objects)),
    ];

    vec![
        Section {
            name: "CPU",
            summary: parts::cpu_summary(cpu),
            active: Some(!cpu.halted()),
            detail: None,
            blocks: cpu_content,
        },
        Section {
            name: "PPU",
            summary: parts::ppu_summary(ppu),
            active: Some(ppu.control().video_enabled()),
            detail: Some(parts::ppu_detail(ppu)),
            blocks: ppu_content,
        },
    ]
}

/// The eight resolved palettes of one CRAM bank as swatch rows.
fn cram_swatches(prefix: &str, palettes: &[Palette; 8]) -> Vec<inspect::SwatchRow> {
    use missingno_gb::ppu::types::palette::PaletteIndex;

    palettes
        .iter()
        .enumerate()
        .map(|(index, palette)| inspect::SwatchRow::Colors {
            label: format!("{prefix}{index}"),
            colors: (0..4).map(|i| palette.color(PaletteIndex(i))).collect(),
        })
        .collect()
}

fn hdma_status(status: VramDmaStatus) -> String {
    match status {
        VramDmaStatus::Idle => "idle".to_owned(),
        VramDmaStatus::General { remaining } => format!("gdma {remaining}B"),
        VramDmaStatus::HBlank {
            remaining,
            source,
            dest,
        } => format!("hdma {remaining}B {source:04X}\u{2192}{dest:04X}"),
    }
}

/// A per-vblank snapshot: the model-shared state plus the CGB register view.
pub struct CgbSnapshot {
    pub base: GbSnapshot,
    pub cgb: CgbView,
}

impl InspectSnapshot for CgbSnapshot {
    fn frame(&self) -> u64 {
        self.base.frame
    }
    fn family_state(&self) -> &dyn Any {
        self
    }
    fn register_groups(&self) -> Vec<inspect::RegisterGroup> {
        self.base.register_groups()
    }
    fn sidebar_sections(&self) -> Vec<inspect::Section> {
        let ColorSnapshot::Cgb {
            background,
            objects,
        } = &self.base.colors
        else {
            return self.base.sidebar_sections();
        };
        cgb_sidebar_sections(
            &self.base.cpu,
            &self.base.ppu,
            &self.base.interrupts,
            &self.cgb,
            background,
            objects,
        )
    }
    fn memory_window(&self) -> Option<&inspect::MemoryWindow> {
        self.base.memory_window()
    }
}

impl ConsoleUi for Cgb {
    const MONOCHROME_PALETTE: bool = false;

    fn screen_display(console: &Console<Self>, new_screen: Option<Self::Screen>) -> Option<Frame> {
        if !console.ppu().control().video_enabled() {
            Some(Frame::Rgba(RgbaFrame::blank(NATIVE_SIZE.0, NATIVE_SIZE.1)))
        } else {
            new_screen.map(|screen| {
                Frame::Rgba(RgbaFrame {
                    width: NATIVE_SIZE.0,
                    height: NATIVE_SIZE.1,
                    pixels: screen.to_corrected_rgba().into(),
                    pixel_aspect: 1.0,
                })
            })
        }
    }

    fn snapshot(
        console: &Console<Self>,
        frame: u64,
        symbols: Arc<SymbolTable>,
        cdl: CdlWindow,
    ) -> DebugView {
        let ppu = console.ppu().model();
        let colors = ColorSnapshot::Cgb {
            background: cram_palettes(|palette, index| ppu.bg_color(palette, index)),
            objects: cram_palettes(|palette, index| ppu.obj_color(palette, index)),
        };
        let base = GbSnapshot::capture(console, colors, frame, symbols, cdl);
        Box::new(CgbSnapshot {
            cgb: CgbView::capture(console),
            base,
        })
    }

    fn sidebar_sections(console: &Console<Self>) -> Vec<inspect::Section> {
        let ppu = console.ppu().model();
        let background = cram_palettes(|palette, index| ppu.bg_color(palette, index));
        let objects = cram_palettes(|palette, index| ppu.obj_color(palette, index));
        cgb_sidebar_sections(
            console.cpu(),
            console.ppu(),
            console.interrupts(),
            &CgbView::capture(console),
            &background,
            &objects,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_gb::cartridge::Cartridge;
    use missingno_gb::debugger::Debugger;

    #[test]
    fn snapshot_sidebar_sections_match_live() {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        let mut debugger = Debugger::new(GameBoyColor::new(Cartridge::new(rom, None), None));
        for _ in 0..4 {
            debugger.step();
        }
        let console = debugger.game_boy();

        let live = Cgb::sidebar_sections(console);
        let snapshot = Cgb::snapshot(
            console,
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
