//! The Game Boy family's frame formats and CPU-side colour resolvers. They
//! stay parallel to a generic indexed path so the user's palette choice and
//! the SGB-colours toggle re-apply at draw time on delivered frames.

use missingno_core::video::{ConsoleFrame, RgbaFrame};
use rgb::RGB8;

use crate::ppu::{
    screen::{self, Screen},
    types::palette::{Palette, PaletteChoice, PaletteIndex},
};
use crate::sgb::{MaskMode, SgbRenderData};

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

/// A Game Boy frame awaiting CPU-side colour resolution.
// One frame per variant per frame tick; indirection would just add a hop.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum GbFrame {
    GameBoy(GameBoyScreen),
    Sgb(SgbScreen),
}

/// Lit shades on the panel's transmission axis; the unlit panel sits one step
/// below the lightest of them, at level 0.
pub const SHADE_LEVELS: u8 = 4;

/// The gradient a response level is read through: the unlit panel then the four
/// lit shades, evenly spaced. The even spacing is a tuned assumption — the shade
/// tones are measured, their positions along the response curve are not.
pub fn gradient_stops(palette: &Palette) -> [RGB8; 5] {
    [
        palette.disabled(),
        palette.color(PaletteIndex(0)),
        palette.color(PaletteIndex(1)),
        palette.color(PaletteIndex(2)),
        palette.color(PaletteIndex(3)),
    ]
}

impl GbFrame {
    /// Resolve to RGBA under a chosen monochrome palette and the SGB-colours
    /// choice — the frontend's colour policy applied to a delivered frame.
    pub fn resolve_with(&self, palette: &Palette, use_sgb_colors: bool) -> RgbaFrame {
        RgbaFrame {
            width: NATIVE_SIZE.0,
            height: NATIVE_SIZE.1,
            pixels: self.to_pixels(palette, use_sgb_colors).into(),
        }
    }

    /// The driven display's per-pixel shade indices, row-major. `None` where no
    /// shade drives the pixels — the LCD off (undriven cells, not shade 0), or
    /// an SGB frame.
    pub fn shades(&self) -> Option<Vec<u8>> {
        match self {
            GbFrame::GameBoy(GameBoyScreen::Display(screen)) => {
                let pixels = screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize;
                let mut shades = Vec::with_capacity(pixels);
                for y in 0..screen::NUM_SCANLINES {
                    for x in 0..screen::PIXELS_PER_LINE {
                        shades.push(screen.pixel(x, y).0);
                    }
                }
                Some(shades)
            }
            GbFrame::GameBoy(GameBoyScreen::Off) | GbFrame::Sgb(_) => None,
        }
    }

    fn to_pixels(&self, palette: &Palette, use_sgb_colors: bool) -> Vec<u8> {
        match self {
            GbFrame::GameBoy(GameBoyScreen::Display(screen)) => {
                screen_to_pixels(screen, palette, None, use_sgb_colors)
            }
            // An off LCD drives no cell: the whole panel shows the unlit tone.
            GbFrame::GameBoy(GameBoyScreen::Off) => {
                let unlit = palette.disabled();
                let pixels = screen::PIXELS_PER_LINE as usize * screen::NUM_SCANLINES as usize;
                [unlit.r, unlit.g, unlit.b, 255].repeat(pixels)
            }
            GbFrame::Sgb(SgbScreen::Display(screen, sgb)) => {
                screen_to_pixels(screen, palette, Some(sgb), use_sgb_colors)
            }
            GbFrame::Sgb(SgbScreen::Freeze(sgb)) => {
                screen_to_pixels(&Screen::default(), palette, Some(sgb), use_sgb_colors)
            }
        }
    }
}

impl ConsoleFrame for GbFrame {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn resolve_rgba(&self) -> RgbaFrame {
        self.resolve_with(PaletteChoice::default().palette(), true)
    }

    fn response_levels(&self) -> Option<Box<[f32]>> {
        if matches!(self, GbFrame::GameBoy(GameBoyScreen::Off)) {
            // An off LCD drives no cell: the whole panel sits at the unlit level.
            let pixels = (NATIVE_SIZE.0 * NATIVE_SIZE.1) as usize;
            return Some(vec![0.0; pixels].into());
        }
        let shades = self.shades()?;
        Some(
            shades
                .iter()
                .map(|&shade| (shade as f32 + 1.0) / SHADE_LEVELS as f32)
                .collect(),
        )
    }

    fn response_stops(&self) -> Option<Box<[RGB8]>> {
        self.response_levels()?;
        Some(gradient_stops(PaletteChoice::default().palette()).into())
    }

    fn clone_box(&self) -> Box<dyn ConsoleFrame> {
        Box::new(self.clone())
    }
}

pub fn screen_to_pixels(
    screen: &Screen,
    palette: &Palette,
    sgb: Option<&SgbRenderData>,
    use_sgb_colors: bool,
) -> Vec<u8> {
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
                                sgb_data.backdrop().to_rgb8()
                            } else {
                                palette.color(palette_index)
                            }
                        }
                        MaskMode::Disabled | MaskMode::Freeze => {
                            if !use_sgb_colors {
                                palette.color(palette_index)
                            } else if palette_index.0 == 0 {
                                // Shade 0 is transparent on the SNES; the shared backdrop shows through
                                sgb_data.backdrop().to_rgb8()
                            } else {
                                let cell_x = x as usize / 8;
                                let cell_y = y as usize / 8;
                                let pal_id = sgb_data.attribute_map.cells[cell_y][cell_x] as usize;
                                sgb_data.palettes[pal_id].colors[palette_index.0 as usize].to_rgb8()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sgb::{AttributeMap, Rgb555, SgbPalette};

    #[test]
    fn shade_zero_shows_the_shared_backdrop() {
        let mut screen = Screen::default();
        screen.draw_pixel(0, 0, PaletteIndex(1));
        screen.present();

        let mut palettes = [SgbPalette::default(); 4];
        for (i, palette) in palettes.iter_mut().enumerate() {
            palette.colors[0] = Rgb555(i as u16 + 1);
        }
        let mut attribute_map = AttributeMap::new();
        attribute_map.cells = [[2; 20]; 18];
        let sgb = SgbRenderData {
            palettes,
            attribute_map,
            mask_mode: MaskMode::Disabled,
            video_enabled: true,
        };

        let pixels = screen_to_pixels(
            &screen,
            PaletteChoice::default().palette(),
            Some(&sgb),
            true,
        );
        let rgb = |c: Rgb555| c.to_rgb8();
        // (0,0) is shade 1: attribute-selected palette 2's colour 1
        assert_eq!(
            pixels[..3],
            [
                rgb(palettes[2].colors[1]).r,
                rgb(palettes[2].colors[1]).g,
                rgb(palettes[2].colors[1]).b
            ]
        );
        // (1,0) is shade 0: the backdrop (palette 0's colour 0), not palette 2's own colour 0
        assert_eq!(
            pixels[4..7],
            [rgb(Rgb555(1)).r, rgb(Rgb555(1)).g, rgb(Rgb555(1)).b]
        );
    }

    #[test]
    fn a_driven_screen_states_one_level_per_shade() {
        // A driven display's shade 0 sits one step above the unlit panel.
        let frame = GbFrame::GameBoy(GameBoyScreen::Display(Screen::default()));
        let levels = frame.response_levels().unwrap();
        assert_eq!(levels.len(), 160 * 144);
        assert!(levels.iter().all(|&level| level == 0.25));
    }

    #[test]
    fn an_off_screen_sits_at_the_unlit_level() {
        // No cell is driven with the LCD off, so the whole panel reads the
        // unlit tone — below shade 0, not equal to it.
        let frame = GbFrame::GameBoy(GameBoyScreen::Off);
        let levels = frame.response_levels().unwrap();
        assert_eq!(levels.len(), 160 * 144);
        assert!(levels.iter().all(|&level| level == 0.0));
    }

    #[test]
    fn each_shade_lands_on_its_own_stop() {
        let mut screen = Screen::default();
        for shade in 0..SHADE_LEVELS {
            screen.draw_pixel(shade as u8, 0, PaletteIndex(shade));
        }
        screen.present();
        let frame = GbFrame::GameBoy(GameBoyScreen::Display(screen));
        let levels = frame.response_levels().unwrap();
        let stops = frame.response_stops().unwrap();
        for shade in 0..SHADE_LEVELS as usize {
            let level = levels[shade];
            // Level 0 is the unlit stop, so shade n is stop n + 1.
            assert_eq!(level, (shade as f32 + 1.0) / SHADE_LEVELS as f32);
            assert_eq!(stops[shade + 1], stops[(level * 4.0).round() as usize]);
        }
    }

    #[test]
    fn an_sgb_frame_drives_no_transmission_axis() {
        // SGB frames are not drawn from the monochrome palette.
        let sgb = SgbRenderData {
            palettes: [SgbPalette::default(); 4],
            attribute_map: AttributeMap::new(),
            mask_mode: MaskMode::Disabled,
            video_enabled: true,
        };
        let frame = GbFrame::Sgb(SgbScreen::Freeze(sgb));
        assert!(frame.response_levels().is_none());
        assert!(frame.response_stops().is_none());
    }
}
