//! The Game Boy family's frame formats and CPU-side colour resolvers. They
//! stay parallel to the generic indexed path so the user's palette choice
//! and the SGB-colours toggle re-apply at draw time on delivered frames.

use rgb::RGB8;

use missingno_gb::{
    ppu::{
        screen::{self, Screen},
        types::palette::{Palette, PaletteIndex},
    },
    sgb::SgbRenderData,
};

use super::ScreenDisplay;

/// The fixed 160×144 LCD.
pub const NATIVE_SIZE: (u32, u32) = (screen::PIXELS_PER_LINE as u32, screen::NUM_SCANLINES as u32);

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

/// A powered-but-blank CGB LCD: all white.
pub fn cgb_blank_rgba() -> Vec<u8> {
    vec![255; screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize * 4]
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
