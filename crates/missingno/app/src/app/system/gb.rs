//! The Game Boy family's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation. The
//! header picks the core; the serial link, printer, boot ROM, and battery-save
//! format are app policy wired in here.

use missingno_gb::frame::{self, GameBoyScreen, GbFrame};
use missingno_gb::ppu::types::palette::{Palette, PaletteChoice, PaletteIndex};
use missingno_gb::system::{LINK_CABLE, LINK_DISCONNECTED, LINK_PRINTER, create_console_with_link};
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge, serial_transfer::SerialLink};
use missingno_gbc::GameBoyColor;
use missingno_gbc::launch::BootRomFit;
pub use missingno_gbc::launch::GbLaunch;

use missingno_core::ports::PeripheralId;
use missingno_core::video::{ConsoleFrame, RgbaFrame};

use super::{ControlMap, MediaLoad, Platform, SystemConsole};
use missingno_iced::PalettePolicy;

/// The Game Boy family's colour policy: the user's monochrome palette plus the
/// Super Game Boy borders, re-applied to a delivered frame at draw time. This is
/// the one place a delivered Game Boy frame is coloured — the renderer holds it
/// as an opaque [`PalettePolicy`]. Only the DMG core emits index frames that
/// reach it; the CGB core delivers resolved RGBA, which the renderer draws
/// directly.
#[derive(Clone)]
struct GbPalettePolicy {
    palette: PaletteChoice,
    use_sgb_colors: bool,
}

impl PalettePolicy for GbPalettePolicy {
    fn resolve(&self, frame: &dyn ConsoleFrame) -> RgbaFrame {
        match frame.as_any().downcast_ref::<GbFrame>() {
            Some(gb) => gb.resolve_with(self.palette.palette(), self.use_sgb_colors),
            None => frame.resolve_rgba(),
        }
    }

    fn clone_box(&self) -> Box<dyn PalettePolicy> {
        Box::new(self.clone())
    }

    fn panel_base(&self) -> Option<rgb::RGB8> {
        // The unlit panel is what the reflective LCD shows through the
        // inter-pixel matrix. SGB colours don't draw from this palette, so no
        // tone there names the panel.
        (!self.use_sgb_colors).then(|| self.palette.palette().disabled())
    }

    fn response_levels(&self, frame: &dyn ConsoleFrame) -> Option<Box<[f32]>> {
        if self.use_sgb_colors {
            return None;
        }
        let frame = frame.as_any().downcast_ref::<GbFrame>()?;
        if matches!(frame, GbFrame::GameBoy(GameBoyScreen::Off)) {
            // An off LCD drives no cell: the whole panel sits at the unlit level.
            let pixels = (frame::NATIVE_SIZE.0 * frame::NATIVE_SIZE.1) as usize;
            return Some(vec![0.0; pixels].into());
        }
        let shades = frame.shades()?;
        Some(
            shades
                .iter()
                .map(|&shade| (shade as f32 + 1.0) / SHADE_LEVELS as f32)
                .collect(),
        )
    }

    fn level_color(&self, level: f32) -> rgb::RGB8 {
        let stops = gradient_stops(self.palette.palette());
        let last = stops.len() - 1;
        let position = level.clamp(0.0, 1.0) * last as f32;
        let lower = (position as usize).min(last);
        let upper = (lower + 1).min(last);
        let fraction = position - lower as f32;
        let between = |a: u8, b: u8| {
            (a as f32 + (b as f32 - a as f32) * fraction)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        rgb::RGB8::new(
            between(stops[lower].r, stops[upper].r),
            between(stops[lower].g, stops[upper].g),
            between(stops[lower].b, stops[upper].b),
        )
    }
}

/// Lit shades on the panel's transmission axis; the unlit panel sits one step
/// below the lightest of them, at level 0.
const SHADE_LEVELS: u8 = 4;

/// The gradient a response level is read through: the unlit panel then the four
/// lit shades, evenly spaced. The even spacing is a tuned assumption — the shade
/// tones are measured, their positions along the response curve are not.
fn gradient_stops(palette: &Palette) -> [rgb::RGB8; 5] {
    [
        palette.disabled(),
        palette.color(PaletteIndex(0)),
        palette.color(PaletteIndex(1)),
        palette.color(PaletteIndex(2)),
        palette.color(PaletteIndex(3)),
    ]
}

/// The Game Boy colour policy for a chosen palette and SGB-colours setting.
pub fn dmg_palette_policy(palette: PaletteChoice, use_sgb_colors: bool) -> Box<dyn PalettePolicy> {
    Box::new(GbPalettePolicy {
        palette,
        use_sgb_colors,
    })
}

/// The colour policy a platform's delivered frames need, or `None` where the
/// core resolves its own colour (every family but the Game Boy).
pub fn palette_policy(
    platform: Platform,
    palette: PaletteChoice,
    use_sgb_colors: bool,
) -> Option<Box<dyn PalettePolicy>> {
    matches!(platform, Platform::GameBoy | Platform::GameBoyColor)
        .then(|| dmg_palette_policy(palette, use_sgb_colors))
}

pub use missingno_gb::media::{is_gb_rom, is_gbc_rom, title_from_rom};

/// Dual-mode media ships as `.gbc` files, so the Game Boy platform's dialog
/// filter must include that extension too.
pub const ROM_EXTENSIONS: &[&str] = &["gb", "gbc"];
pub const GBC_ROM_EXTENSIONS: &[&str] = &["gbc"];
pub const DEFAULT_ROM_EXTENSION: &str = "gb";
pub const SAVE_FILTER_NAME: &str = "Game Boy Save";
pub const SAVE_EXTENSIONS: &[&str] = &["sav"];

/// The console's pad and its link port; the Game Boy has no panel controls.
pub const CONTROLS: ControlMap =
    ControlMap::new(missingno_gb::system::PAD, missingno_gb::system::PORTS, &[]);

/// The battery-backed contents to persist: raw SRAM plus the wall-clock RTC
/// tail. The save-file format is app policy, so the core takes this as a
/// hook rather than owning a clock.
fn battery_save(cartridge: &Cartridge) -> Option<Vec<u8>> {
    if !cartridge.has_battery() {
        return None;
    }
    crate::sram::save_blob(cartridge, crate::sram::now_unix())
}

/// A cartridge from ROM + saved battery contents: any RTC tail in the save
/// restores the clock and catches it up on the time since the save.
fn build_cartridge(rom: Vec<u8>, save_data: Option<Vec<u8>>) -> Cartridge {
    let (ram, rtc) = match save_data {
        Some(blob) => {
            let (ram, rtc) = crate::sram::split_blob(blob);
            (Some(ram), rtc)
        }
        None => (None, None),
    };
    let mut cartridge = Cartridge::new(rom, ram);
    if let Some((snapshot, saved_at)) = rtc {
        let elapsed = crate::sram::now_unix().saturating_sub(saved_at);
        cartridge.restore_rtc(snapshot, elapsed);
    }
    cartridge
}

/// The app's executable paths (GUI load, trace) reach the core selection
/// through here, adding the save-backed cartridge and a word to the user when
/// the boot ROM they named is dropped.
pub fn launch<L: GbLaunch>(
    rom: Vec<u8>,
    save_data: Option<Vec<u8>>,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
    launcher: L,
) -> L::Output {
    let cartridge = build_cartridge(rom, save_data);
    let (output, boot_rom) = missingno_gbc::launch::console(cartridge, boot_rom, link, launcher);
    if boot_rom == BootRomFit::Dropped {
        eprintln!("warning: boot ROM model does not match the selected core; ignoring it");
    }
    output
}

/// The factory both platform descriptors register: the header picks the
/// core. The serial link is a Game Boy peripheral, so it is taken here; a
/// virtual printer sits on the link port by default, staying inert unless a
/// game prints, with prints landing in the game's folder.
pub fn create_console(media: MediaLoad) -> Result<Box<dyn SystemConsole>, String> {
    struct Boxed {
        link: PeripheralId,
    }
    impl GbLaunch for Boxed {
        type Output = Box<dyn SystemConsole>;
        fn dmg(self, console: GameBoy) -> Self::Output {
            Box::new(create_console_with_link(console, battery_save, self.link))
        }
        fn cgb(self, console: GameBoyColor) -> Self::Output {
            Box::new(create_console_with_link(console, battery_save, self.link))
        }
    }
    let (link, kind) = match media.serial_link.take() {
        Some(cable) => (Some(cable), LINK_CABLE),
        None => match media.print_sink {
            Some(sink) => (
                Some(Box::new(crate::printer::GbPrinter::new(sink)) as Box<dyn SerialLink>),
                LINK_PRINTER,
            ),
            None => (None, LINK_DISCONNECTED),
        },
    };
    Ok(launch(
        media.rom.to_vec(),
        media.save_data,
        media.boot_rom,
        link,
        Boxed { link: kind },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_gb::ppu::screen::Screen;

    fn policy(use_sgb_colors: bool) -> GbPalettePolicy {
        GbPalettePolicy {
            palette: PaletteChoice::Green,
            use_sgb_colors,
        }
    }

    #[test]
    fn levels_land_on_the_five_gradient_stops() {
        let policy = policy(false);
        let palette = PaletteChoice::Green.palette();
        assert_eq!(policy.level_color(0.0), palette.disabled());
        assert_eq!(policy.level_color(0.25), palette.color(PaletteIndex(0)));
        assert_eq!(policy.level_color(0.5), palette.color(PaletteIndex(1)));
        assert_eq!(policy.level_color(0.75), palette.color(PaletteIndex(2)));
        assert_eq!(policy.level_color(1.0), palette.color(PaletteIndex(3)));
    }

    #[test]
    fn a_level_between_stops_is_their_mix() {
        let policy = policy(false);
        let palette = PaletteChoice::Green.palette();
        let (a, b) = (
            palette.color(PaletteIndex(0)),
            palette.color(PaletteIndex(1)),
        );
        let mid = policy.level_color(0.375);
        assert_eq!(mid.r, ((a.r as f32 + b.r as f32) / 2.0).round() as u8);
        assert_eq!(mid.g, ((a.g as f32 + b.g as f32) / 2.0).round() as u8);
        assert_eq!(mid.b, ((a.b as f32 + b.b as f32) / 2.0).round() as u8);
    }

    #[test]
    fn a_driven_screen_states_one_level_per_shade() {
        // A driven display's shade 0 sits one step above the unlit panel.
        let frame = GbFrame::GameBoy(GameBoyScreen::Display(Screen::default()));
        let levels = policy(false).response_levels(&frame).unwrap();
        assert_eq!(levels.len(), 160 * 144);
        assert!(levels.iter().all(|&level| level == 0.25));
        assert_eq!(
            policy(false).level_color(levels[0]),
            PaletteChoice::Green.palette().color(PaletteIndex(0))
        );
    }

    #[test]
    fn an_off_screen_sits_at_the_unlit_level() {
        // No cell is driven with the LCD off, so the whole panel reads the
        // unlit tone — below shade 0, not equal to it.
        let frame = GbFrame::GameBoy(GameBoyScreen::Off);
        let levels = policy(false).response_levels(&frame).unwrap();
        assert_eq!(levels.len(), 160 * 144);
        assert!(levels.iter().all(|&level| level == 0.0));
        assert_eq!(
            policy(false).level_color(0.0),
            PaletteChoice::Green.palette().disabled()
        );
    }

    #[test]
    fn sgb_coloured_frames_state_no_response_axis() {
        // SGB colours don't come from the monochrome palette, so there is no
        // transmission axis to accumulate along.
        let frame = GbFrame::GameBoy(GameBoyScreen::Off);
        assert!(policy(true).response_levels(&frame).is_none());
        assert!(policy(true).panel_base().is_none());
    }
}
