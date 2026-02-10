use iced::widget::shader;
use rgb::RGB8;

use crate::game_boy::{
    sgb::SgbRenderData,
    video::{
        palette::{Palette, PaletteChoice},
        screen::{self, Screen},
    },
};

use super::texture_renderer::TextureRenderer;

#[derive(Copy, Clone, Debug)]
pub enum ScreenDisplay {
    GameBoy(GameBoyScreen),
    Sgb(SgbScreen),
}

#[derive(Copy, Clone, Debug)]
pub enum GameBoyScreen {
    Display(Screen),
    Off,
}

#[derive(Copy, Clone, Debug)]
pub enum SgbScreen {
    Display(Screen, SgbRenderData),
    Freeze(SgbRenderData),
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

pub struct ScreenView {
    pub screen: Screen,
    pub palette: PaletteChoice,
    pub sgb_render_data: Option<SgbRenderData>,
}

impl ScreenView {
    pub fn new() -> Self {
        Self {
            screen: Screen::new(),
            palette: PaletteChoice::default(),
            sgb_render_data: None,
        }
    }

    pub fn apply(&mut self, display: ScreenDisplay) {
        match display {
            ScreenDisplay::GameBoy(GameBoyScreen::Display(screen)) => {
                self.screen = screen;
                self.sgb_render_data = None;
            }
            ScreenDisplay::GameBoy(GameBoyScreen::Off) => {
                // NOTE: On real hardware, LCD off produces a different shade than
                // palette index 0. We currently render both the same way.
                self.screen = Screen::new();
                self.sgb_render_data = None;
            }
            ScreenDisplay::Sgb(SgbScreen::Display(screen, sgb_data)) => {
                self.screen = screen;
                self.sgb_render_data = Some(sgb_data);
            }
            ScreenDisplay::Sgb(SgbScreen::Freeze(sgb_data)) => {
                self.sgb_render_data = Some(sgb_data);
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
        let pixels = screen_to_pixels(
            &self.screen,
            self.palette.palette(),
            self.sgb_render_data.as_ref(),
        );
        let renderer = TextureRenderer::with_pixels(
            screen::PIXELS_PER_LINE as u32,
            screen::NUM_SCANLINES as u32,
            pixels,
        );

        <TextureRenderer as shader::Program<Message>>::draw(&renderer, &(), cursor, bounds)
    }
}

pub fn screen_to_pixels(
    screen: &Screen,
    palette: &Palette,
    sgb: Option<&SgbRenderData>,
) -> Vec<u8> {
    use crate::game_boy::sgb::MaskMode;

    let mut pixels =
        Vec::with_capacity(screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize * 4);

    for y in 0..screen::NUM_SCANLINES {
        for x in 0..screen::PIXELS_PER_LINE {
            let palette_index = screen.pixel(x, y);
            let color = if let Some(sgb_data) = sgb {
                if !sgb_data.video_enabled {
                    RGB8::new(255, 255, 255)
                } else {
                    match sgb_data.mask_mode {
                        MaskMode::Black => RGB8::new(0, 0, 0),
                        MaskMode::BackdropColor => sgb_data.palettes[0].colors[0].to_rgb8(),
                        MaskMode::Disabled | MaskMode::Freeze => {
                            let cell_x = x as usize / 8;
                            let cell_y = y as usize / 8;
                            let pal_id = sgb_data.attribute_map.cells[cell_y][cell_x] as usize;
                            sgb_data.palettes[pal_id].colors[palette_index.0 as usize].to_rgb8()
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
