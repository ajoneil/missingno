use iced::widget::shader;
use rgb::RGB8;

use missingno_gb::{
    ppu::{screen::Screen, types::palette::PaletteChoice},
    sgb::SgbRenderData,
};

use super::texture_renderer::TextureRenderer;

pub mod gb;
use gb::{screen_to_pixels, CgbScreen, GameBoyScreen, SgbScreen};

// One frame per variant per frame tick; indirection would just add a hop.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum ScreenDisplay {
    GameBoy(GameBoyScreen),
    Sgb(SgbScreen),
    Cgb(CgbScreen),
    /// System-agnostic palette-indexed frame, any dimensions.
    Indexed(IndexedFrame),
}

pub use missingno_core::video::IndexedFrame;

impl From<IndexedFrame> for ScreenDisplay {
    fn from(frame: IndexedFrame) -> Self {
        ScreenDisplay::Indexed(frame)
    }
}

#[derive(Clone)]
pub struct ScreenView {
    pub screen: Screen,
    pub palette: PaletteChoice,
    pub sgb_render_data: Option<SgbRenderData>,
    pub use_sgb_colors: bool,
    /// Pre-corrected CGB RGBA frame; bypasses the palette paths when set.
    pub cgb_rgba: Option<std::sync::Arc<[u8]>>,
    /// Palette-indexed frame from a non-GB system; carries its own size.
    pub indexed: Option<IndexedFrame>,
    /// Average each frame with the previous one, like the LCD's slow response.
    pub blend: bool,
    pub prev_rgba: Option<std::sync::Arc<[u8]>>,
}

impl ScreenView {
    pub fn new() -> Self {
        Self {
            screen: Screen::default(),
            palette: PaletteChoice::default(),
            sgb_render_data: None,
            use_sgb_colors: true,
            cgb_rgba: None,
            indexed: None,
            blend: true,
            prev_rgba: None,
        }
    }

    fn resolve_rgba(&self) -> std::sync::Arc<[u8]> {
        if let Some(frame) = &self.indexed {
            return frame.to_rgba().into();
        }
        match &self.cgb_rgba {
            Some(rgba) => rgba.clone(),
            None => screen_to_pixels(
                &self.screen,
                self.palette.palette(),
                self.sgb_render_data.as_ref(),
                self.use_sgb_colors,
            )
            .into(),
        }
    }

    pub fn apply(&mut self, display: ScreenDisplay) {
        self.prev_rgba = Some(self.resolve_rgba());
        match display {
            ScreenDisplay::GameBoy(GameBoyScreen::Display(screen)) => {
                self.screen = screen;
                self.sgb_render_data = None;
                self.cgb_rgba = None;
                self.indexed = None;
            }
            ScreenDisplay::GameBoy(GameBoyScreen::Off) => {
                // NOTE: On real hardware, LCD off produces a different shade than
                // palette index 0. We currently render both the same way.
                self.screen = Screen::default();
                self.sgb_render_data = None;
                self.cgb_rgba = None;
                self.indexed = None;
            }
            ScreenDisplay::Sgb(SgbScreen::Display(screen, sgb_data)) => {
                self.screen = screen;
                self.sgb_render_data = Some(sgb_data);
                self.cgb_rgba = None;
                self.indexed = None;
            }
            ScreenDisplay::Sgb(SgbScreen::Freeze(sgb_data)) => {
                self.sgb_render_data = Some(sgb_data);
                self.cgb_rgba = None;
                self.indexed = None;
            }
            ScreenDisplay::Cgb(CgbScreen::Display(rgba)) => {
                self.sgb_render_data = None;
                self.cgb_rgba = Some(rgba.into());
                self.indexed = None;
            }
            ScreenDisplay::Cgb(CgbScreen::Off) => {
                self.sgb_render_data = None;
                self.cgb_rgba = Some(gb::cgb_blank_rgba().into());
                self.indexed = None;
            }
            ScreenDisplay::Indexed(frame) => {
                self.sgb_render_data = None;
                self.cgb_rgba = None;
                self.indexed = Some(frame);
            }
        }
    }

    /// The active frame's pixel dimensions; the GB formats are fixed-size.
    fn dimensions(&self) -> (u32, u32) {
        match &self.indexed {
            Some(frame) => (frame.width, frame.height),
            None => gb::NATIVE_SIZE,
        }
    }

    /// Widget dimensions filling the available space: indexed frames fit
    /// their true display aspect; the GB paths keep the shell's square fit.
    pub fn fitted_size(&self, available: iced::Size) -> (f32, f32) {
        match &self.indexed {
            Some(frame) => {
                let aspect = frame.width as f32 * frame.pixel_aspect / frame.height as f32;
                let width = available.width.min(available.height * aspect);
                (width, width / aspect)
            }
            None => {
                let shortest = available.width.min(available.height);
                (shortest, shortest)
            }
        }
    }
}

impl<Message> shader::Program<Message> for ScreenView {
    type State = ();
    type Primitive = <TextureRenderer as shader::Program<Message>>::Primitive;

    fn draw(
        &self,
        _state: &Self::State,
        cursor: iced::mouse::Cursor,
        bounds: iced::Rectangle,
    ) -> Self::Primitive {
        let current = self.resolve_rgba();
        let pixels: std::sync::Arc<[u8]> = match &self.prev_rgba {
            Some(prev) if self.blend && prev.len() == current.len() => current
                .iter()
                .zip(prev.iter())
                .map(|(&a, &b)| ((a as u16 + b as u16) / 2) as u8)
                .collect::<Vec<u8>>()
                .into(),
            _ => current,
        };
        let (width, height) = self.dimensions();
        let renderer = TextureRenderer::with_pixels(width, height, pixels);

        <TextureRenderer as shader::Program<Message>>::draw(&renderer, &(), cursor, bounds)
    }
}

pub fn iced_color(color: RGB8) -> iced::Color {
    iced::Color::from_rgb8(color.r, color.g, color.b)
}
