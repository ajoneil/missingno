//! The Game Boy Color model's implementation of the system seam's display and
//! debugger hooks: colour screen framing, the CGB register view the sidebar
//! draws, and the per-vblank snapshot that carries it.

use std::any::Any;
use std::sync::Arc;

use missingno_core::cdl::CdlWindow;
use missingno_core::symbols::SymbolTable;
use missingno_core::system::{DebugView, InspectSnapshot};
use missingno_core::video::{Frame, RgbaFrame};

use missingno_gb::Console;
use missingno_gb::debugger::inspection::{ColorSnapshot, GbSnapshot};
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
    fn register_groups(&self) -> Vec<missingno_core::inspect::RegisterGroup> {
        self.base.register_groups()
    }
    fn memory_window(&self) -> Option<&missingno_core::inspect::MemoryWindow> {
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
}
