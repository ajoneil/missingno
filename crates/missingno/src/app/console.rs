use missingno_gb::{
    BootRom, Console, Dmg, GameBoy, Model, cartridge::Cartridge, execute::StepResult,
    joypad::Button, ppu::types::palette::Palette, serial_transfer::SerialLink, sgb::MaskMode,
};
use missingno_gbc::{Cgb, GameBoyColor};

use crate::app::library::activity::FrameCapture;
use crate::app::screen::{CgbScreen, GameBoyScreen, ScreenDisplay, SgbScreen};
use crate::render::cram_palettes;

/// The console a loaded game runs on, picked from the cartridge header:
/// CGB-aware ROMs get the CGB core, everything else the DMG core.
pub enum AnyConsole {
    Dmg(GameBoy),
    Cgb(GameBoyColor),
}

impl From<GameBoy> for AnyConsole {
    fn from(console: GameBoy) -> Self {
        Self::Dmg(console)
    }
}

impl From<GameBoyColor> for AnyConsole {
    fn from(console: GameBoyColor) -> Self {
        Self::Cgb(console)
    }
}

impl AnyConsole {
    pub fn new(cartridge: Cartridge, boot_rom: Option<BootRom>) -> Self {
        if cartridge.is_cgb() {
            Self::Cgb(GameBoyColor::new(cartridge, boot_rom))
        } else {
            Self::Dmg(GameBoy::new(cartridge, boot_rom))
        }
    }

    pub fn step(&mut self) -> StepResult {
        match self {
            Self::Dmg(console) => console.step(),
            Self::Cgb(console) => console.step(),
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Dmg(console) => console.reset(),
            Self::Cgb(console) => console.reset(),
        }
    }

    pub fn press_button(&mut self, button: Button) {
        match self {
            Self::Dmg(console) => console.press_button(button),
            Self::Cgb(console) => console.press_button(button),
        }
    }

    pub fn release_button(&mut self, button: Button) {
        match self {
            Self::Dmg(console) => console.release_button(button),
            Self::Cgb(console) => console.release_button(button),
        }
    }

    pub fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        match self {
            Self::Dmg(console) => console.drain_audio_samples(),
            Self::Cgb(console) => console.drain_audio_samples(),
        }
    }

    pub fn set_link(&mut self, link: Box<dyn SerialLink>) {
        match self {
            Self::Dmg(console) => console.set_link(link),
            Self::Cgb(console) => console.set_link(link),
        }
    }

    pub fn cartridge(&self) -> &Cartridge {
        match self {
            Self::Dmg(console) => console.cartridge(),
            Self::Cgb(console) => console.cartridge(),
        }
    }

    pub fn cpu_tcycles_per_dot(&self) -> u8 {
        match self {
            Self::Dmg(console) => console.cpu_steps_per_dot(),
            Self::Cgb(console) => console.cpu_steps_per_dot(),
        }
    }

    pub fn screen_display(&self) -> ScreenDisplay {
        match self {
            Self::Dmg(console) => Dmg::screen_display(console, Some(console.screen().clone())),
            Self::Cgb(console) => Cgb::screen_display(console, Some(console.screen().clone())),
        }
        .expect("screen_display is always Some when given a screen")
    }

    pub fn capture_frame(&self, use_sgb_colors: bool, palette_name: &str) -> FrameCapture {
        match self {
            Self::Dmg(console) => Dmg::capture_frame(console, use_sgb_colors, palette_name),
            Self::Cgb(console) => Cgb::capture_frame(console, use_sgb_colors, palette_name),
        }
    }
}

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
    /// The display for a step's screen result; `None` leaves the screen pane as-is.
    fn screen_display(
        console: &Console<Self>,
        new_screen: Option<Self::Screen>,
    ) -> Option<ScreenDisplay>;

    fn colors(console: &Console<Self>, user_palette: &Palette) -> ConsoleColors;

    fn capture_frame(
        console: &Console<Self>,
        use_sgb_colors: bool,
        palette_name: &str,
    ) -> FrameCapture;
}

impl ConsoleUi for Dmg {
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

    fn capture_frame(
        console: &Console<Self>,
        use_sgb_colors: bool,
        palette_name: &str,
    ) -> FrameCapture {
        let sgb_data = console
            .sgb()
            .map(|sgb| sgb.render_data(console.ppu().control().video_enabled()));
        FrameCapture::capture(
            console.screen().front(),
            sgb_data.as_ref(),
            use_sgb_colors,
            palette_name,
        )
    }
}

impl ConsoleUi for Cgb {
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

    fn capture_frame(
        console: &Console<Self>,
        _use_sgb_colors: bool,
        _palette_name: &str,
    ) -> FrameCapture {
        FrameCapture::capture_cgb(console.screen())
    }
}
