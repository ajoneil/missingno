use iced::widget::shader;
use rgb::RGB8;

use missingno_gb::{
    ppu::{
        screen::{self, Screen},
        types::palette::{Palette, PaletteChoice, PaletteIndex},
    },
    sgb::SgbRenderData,
};

use super::texture_renderer::TextureRenderer;

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

/// A frame of palette indices plus the palette to resolve them with,
/// converted to RGBA at draw time. Height is per-frame: systems without a
/// hardware frame (emergent sync) legitimately vary line counts. The
/// palette is shared, not static — systems with programmable colour RAM
/// send the palette as it stood when the frame completed.
#[derive(Clone, Debug)]
pub struct IndexedFrame {
    pub width: u32,
    pub height: u32,
    /// Row-major palette indices, `width * height` entries.
    pub pixels: std::sync::Arc<[u8]>,
    pub palette: std::sync::Arc<[RGB8]>,
}

impl IndexedFrame {
    // Only the feature-gated indexed families construct blank frames.
    #[cfg_attr(
        not(any(feature = "vcs", feature = "sms", feature = "nes")),
        allow(dead_code)
    )]
    pub fn blank(width: u32, height: u32, palette: std::sync::Arc<[RGB8]>) -> Self {
        IndexedFrame {
            width,
            height,
            pixels: vec![0; (width * height) as usize].into(),
            palette,
        }
    }

    pub fn to_rgba(&self) -> Vec<u8> {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for &index in self.pixels.iter() {
            let color = self
                .palette
                .get(index as usize)
                .copied()
                .unwrap_or(RGB8::new(0, 0, 0));
            rgba.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
        rgba
    }
}

#[derive(Clone, Debug)]
pub enum GameBoyScreen {
    Display(Screen),
    Off,
}

#[derive(Clone, Debug)]
pub enum SgbScreen {
    Display(Screen, SgbRenderData),
    Freeze(SgbRenderData),
}

/// CGB output, pre-corrected to display RGBA — no user palette applies.
#[derive(Clone, Debug)]
pub enum CgbScreen {
    Display(Vec<u8>),
    Off,
}

impl From<GameBoyScreen> for ScreenDisplay {
    fn from(screen: GameBoyScreen) -> Self {
        ScreenDisplay::GameBoy(screen)
    }
}

impl From<SgbScreen> for ScreenDisplay {
    fn from(screen: SgbScreen) -> Self {
        ScreenDisplay::Sgb(screen)
    }
}

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
                self.cgb_rgba = Some(cgb_blank_rgba().into());
                self.indexed = None;
            }
            ScreenDisplay::Indexed(frame) => {
                self.sgb_render_data = None;
                self.cgb_rgba = None;
                self.indexed = Some(frame);
            }
        }
    }

    /// The active frame's pixel dimensions.
    fn dimensions(&self) -> (u32, u32) {
        match &self.indexed {
            Some(frame) => (frame.width, frame.height),
            None => (screen::PIXELS_PER_LINE as u32, screen::NUM_SCANLINES as u32),
        }
    }
}

/// A powered-but-blank CGB LCD: all white.
pub fn cgb_blank_rgba() -> Vec<u8> {
    vec![255; screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize * 4]
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

pub fn screen_to_pixels(
    screen: &Screen,
    palette: &Palette,
    sgb: Option<&SgbRenderData>,
    use_sgb_colors: bool,
) -> Vec<u8> {
    use missingno_gb::sgb::MaskMode;

    let mut pixels =
        Vec::with_capacity(screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize * 4);

    for y in 0..screen::NUM_SCANLINES {
        for x in 0..screen::PIXELS_PER_LINE {
            let palette_index = screen.pixel(x, y);
            let color = if let Some(sgb_data) = sgb {
                if !sgb_data.video_enabled {
                    if use_sgb_colors {
                        RGB8::new(255, 255, 255)
                    } else {
                        palette.color(PaletteIndex(0))
                    }
                } else {
                    match sgb_data.mask_mode {
                        MaskMode::Black => RGB8::new(0, 0, 0),
                        MaskMode::BackdropColor => {
                            if use_sgb_colors {
                                sgb_data.palettes[0].colors[0].to_rgb8()
                            } else {
                                palette.color(palette_index)
                            }
                        }
                        MaskMode::Disabled | MaskMode::Freeze => {
                            if use_sgb_colors {
                                let cell_x = x as usize / 8;
                                let cell_y = y as usize / 8;
                                let pal_id = sgb_data.attribute_map.cells[cell_y][cell_x] as usize;
                                sgb_data.palettes[pal_id].colors[palette_index.0 as usize].to_rgb8()
                            } else {
                                palette.color(palette_index)
                            }
                        }
                    }
                }
            } else {
                palette.color(palette_index)
            };
            pixels.extend_from_slice(&[color.r, color.g, color.b, 255]);
        }
    }

    pixels
}

pub fn iced_color(color: RGB8) -> iced::Color {
    iced::Color::from_rgb8(color.r, color.g, color.b)
}
