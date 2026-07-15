use missingno_gb::{Console, Dmg, Model, ppu::types::palette::Palette, sgb::MaskMode};
use missingno_gbc::Cgb;

use crate::app::debugger::inspect::{CgbView, ColorSnapshot};
use crate::app::library::activity::{CaptureOptions, FrameCapture};
use crate::app::screen::{
    ScreenDisplay,
    gb::{CgbScreen, GameBoyScreen, SgbScreen},
};
use crate::render::cram_palettes;

/// The colours the debugger panes draw with: the user-selected palette on
/// DMG, the corrected CRAM palettes on CGB.
pub enum ConsoleColors {
    Dmg {
        palette: Palette,
    },
    Cgb {
        background: [Palette; 8],
        objects: [Palette; 8],
    },
}

impl ConsoleColors {
    /// CGB tile data has no palette of its own — show it in greyscale.
    pub fn tiles_palette(&self) -> &Palette {
        match self {
            Self::Dmg { palette } => palette,
            Self::Cgb { .. } => &Palette::CLASSIC,
        }
    }

    pub fn is_cgb(&self) -> bool {
        matches!(self, Self::Cgb { .. })
    }
}

/// How the debugger UI renders each console model.
pub trait ConsoleUi: Model {
    /// The platform this model presents as.
    const PLATFORM: crate::app::system::Platform;

    /// DMG renders through a user-selectable monochrome palette; CGB is
    /// colour. Gates the play-mode Display panel's palette picker.
    const MONOCHROME_PALETTE: bool;

    /// The display for a step's screen result; `None` leaves the screen pane as-is.
    fn screen_display(
        console: &Console<Self>,
        new_screen: Option<Self::Screen>,
    ) -> Option<ScreenDisplay>;

    fn colors(console: &Console<Self>, user_palette: &Palette) -> ConsoleColors;

    /// The palette-independent colour data to publish while the core runs, so
    /// the running panes rebuild [`ConsoleColors`] with the live user palette.
    fn color_snapshot(console: &Console<Self>) -> ColorSnapshot;

    /// The CGB-only register state for the debugger sidebar; `None` on DMG.
    fn cgb_view(console: &Console<Self>) -> Option<CgbView>;

    fn capture_frame(console: &Console<Self>, options: &CaptureOptions) -> FrameCapture;
}

impl ConsoleUi for Dmg {
    const PLATFORM: crate::app::system::Platform = crate::app::system::Platform::GameBoy;
    const MONOCHROME_PALETTE: bool = true;

    fn screen_display(
        console: &Console<Self>,
        new_screen: Option<Self::Screen>,
    ) -> Option<ScreenDisplay> {
        let video_enabled = console.ppu().control().video_enabled();
        if let Some(sgb) = console.sgb() {
            let render_data = sgb.render_data(video_enabled);
            if sgb.mask_mode == MaskMode::Freeze {
                Some(ScreenDisplay::Sgb(SgbScreen::Freeze(render_data)))
            } else {
                new_screen.map(|screen| ScreenDisplay::Sgb(SgbScreen::Display(screen, render_data)))
            }
        } else if !video_enabled {
            Some(ScreenDisplay::GameBoy(GameBoyScreen::Off))
        } else {
            new_screen.map(|screen| ScreenDisplay::GameBoy(GameBoyScreen::Display(screen)))
        }
    }

    fn colors(console: &Console<Self>, user_palette: &Palette) -> ConsoleColors {
        ConsoleColors::Dmg {
            palette: if console.sgb().is_some() {
                Palette::CLASSIC
            } else {
                *user_palette
            },
        }
    }

    fn color_snapshot(console: &Console<Self>) -> ColorSnapshot {
        ColorSnapshot::Dmg {
            sgb: console.sgb().is_some(),
        }
    }

    fn cgb_view(_console: &Console<Self>) -> Option<CgbView> {
        None
    }

    fn capture_frame(console: &Console<Self>, options: &CaptureOptions) -> FrameCapture {
        let sgb_data = console
            .sgb()
            .map(|sgb| sgb.render_data(console.ppu().control().video_enabled()));
        FrameCapture::capture(
            console.screen().front(),
            sgb_data.as_ref(),
            options.use_sgb_colors,
            &options.palette_name,
        )
    }
}

impl ConsoleUi for Cgb {
    const PLATFORM: crate::app::system::Platform = crate::app::system::Platform::GameBoyColor;
    const MONOCHROME_PALETTE: bool = false;

    fn screen_display(
        console: &Console<Self>,
        new_screen: Option<Self::Screen>,
    ) -> Option<ScreenDisplay> {
        if !console.ppu().control().video_enabled() {
            Some(ScreenDisplay::Cgb(CgbScreen::Off))
        } else {
            new_screen
                .map(|screen| ScreenDisplay::Cgb(CgbScreen::Display(screen.to_corrected_rgba())))
        }
    }

    fn colors(console: &Console<Self>, _user_palette: &Palette) -> ConsoleColors {
        let ppu = console.ppu().model();
        ConsoleColors::Cgb {
            background: cram_palettes(|palette, index| ppu.bg_color(palette, index)),
            objects: cram_palettes(|palette, index| ppu.obj_color(palette, index)),
        }
    }

    fn color_snapshot(console: &Console<Self>) -> ColorSnapshot {
        let ppu = console.ppu().model();
        ColorSnapshot::Cgb {
            background: cram_palettes(|palette, index| ppu.bg_color(palette, index)),
            objects: cram_palettes(|palette, index| ppu.obj_color(palette, index)),
        }
    }

    fn cgb_view(console: &Console<Self>) -> Option<CgbView> {
        let model = console.model();
        let ppu = console.ppu();
        let (bcps, ocps) = ppu.model().palette_index_registers();
        Some(CgbView {
            double_speed: model.double_speed(),
            vram_bank: console.vram().selected_bank(),
            wram_bank: model.wram_bank(),
            opri: ppu.read_object_priority(),
            bcps,
            ocps,
            vram_dma: model.vram_dma_status(),
        })
    }

    fn capture_frame(console: &Console<Self>, _options: &CaptureOptions) -> FrameCapture {
        FrameCapture::capture_cgb(console.screen())
    }
}
