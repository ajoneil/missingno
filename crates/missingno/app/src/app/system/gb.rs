//! The Game Boy family's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation. The
//! header picks the core; the serial link, printer, boot ROM, and battery-save
//! format are app policy wired in here.

use missingno_gb::cartridge::{GbCartType, GbCartridgeError};
use missingno_gb::frame::{GbFrame, gradient_stops};
use missingno_gb::ppu::types::palette::PaletteChoice;
use missingno_gb::system::{LINK_CABLE, LINK_DISCONNECTED, LINK_PRINTER, create_console_with_link};
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge, serial_transfer::SerialLink};
use missingno_gbc::GameBoyColor;
use missingno_gbc::launch::BootRomFit;
pub use missingno_gbc::launch::{
    BOARD, BOOT_ROM, ENHANCEMENT_CGB, ENHANCEMENT_SGB, ENHANCEMENTS, GbLaunch, RUNNER,
    RunnerPreference, board_from_launch, launch_options,
};

use missingno_core::cartridge::BoardVocabulary;
use missingno_core::ports::PeripheralId;
use missingno_core::video::{ConsoleFrame, RgbaFrame};
use missingno_gamedb::Peripheral;

use super::{ControlMap, LaunchValue, MediaFact, MediaLoad, Platform, SystemConsole};
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
        // SGB colours are not drawn from the panel's monochrome axis.
        if self.use_sgb_colors {
            return None;
        }
        frame.response_levels()
    }

    fn response_stops(&self) -> Option<Box<[rgb::RGB8]>> {
        (!self.use_sgb_colors).then(|| gradient_stops(self.palette.palette()).into())
    }
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

/// What the cartridge header answers for itself: the console it is slotted
/// into, as the hardware would read it, the variants its flags claim, and the
/// board it declares.
pub fn stated_by_media(rom: &[u8]) -> Vec<MediaFact> {
    let cgb = Cartridge::peek_cgb(rom);
    let enhancements = cgb
        .then_some(ENHANCEMENT_CGB)
        .into_iter()
        .chain(Cartridge::peek_sgb(rom).then_some(ENHANCEMENT_SGB))
        .map(str::to_owned)
        .collect();
    let mut stated = vec![
        MediaFact {
            option: RUNNER,
            value: LaunchValue::Choice(if cgb { "cgb" } else { "dmg" }.to_owned()),
        },
        MediaFact {
            option: ENHANCEMENTS,
            value: LaunchValue::Flags(enhancements),
        },
    ];
    if let Ok(board) = GbCartType::from_header(rom) {
        stated.push(MediaFact {
            option: BOARD,
            value: LaunchValue::Board(board.to_value()),
        });
    }
    stated
}

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
fn build_cartridge(
    rom: Vec<u8>,
    board: Option<GbCartType>,
    save_data: Option<Vec<u8>>,
) -> Result<Cartridge, GbCartridgeError> {
    let (ram, rtc) = match save_data {
        Some(blob) => {
            let (ram, rtc) = crate::sram::split_blob(blob);
            (Some(ram), rtc)
        }
        None => (None, None),
    };
    let mut cartridge = Cartridge::new(rom, board, ram)?;
    if let Some((snapshot, saved_at)) = rtc {
        let elapsed = crate::sram::now_unix().saturating_sub(saved_at);
        cartridge.restore_rtc(snapshot, elapsed);
    }
    Ok(cartridge)
}

/// The app's executable paths (GUI load, trace) reach the core selection
/// through here, adding the save-backed cartridge and a word to the user when
/// the boot ROM they named is dropped.
pub fn launch<L: GbLaunch>(
    rom: Vec<u8>,
    board: Option<GbCartType>,
    save_data: Option<Vec<u8>>,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
    runner: RunnerPreference,
    launcher: L,
) -> Result<L::Output, String> {
    let cartridge =
        build_cartridge(rom, board, save_data).map_err(|refusal| refusal.to_string())?;
    let (output, boot_rom) =
        missingno_gbc::launch::console(cartridge, boot_rom, link, runner, launcher)
            .map_err(|refused| format!("{RUNNER}: {refused}"))?;
    if boot_rom == BootRomFit::Dropped {
        eprintln!("warning: boot ROM model does not match the selected core; ignoring it");
    }
    Ok(output)
}

/// What hangs off the link port: an explicit cable is the user's own word, and
/// a virtual printer attaches only where the catalogue states the release is
/// played with one, staying inert unless the game prints.
fn link_port(
    cable: Option<Box<dyn SerialLink>>,
    print_sink: Option<crate::printer::PrintSink>,
    peripherals: &[Peripheral],
) -> (Option<Box<dyn SerialLink>>, PeripheralId) {
    if let Some(cable) = cable {
        return (Some(cable), LINK_CABLE);
    }
    match print_sink.filter(|_| peripherals.contains(&Peripheral::Printer)) {
        Some(sink) => (
            Some(Box::new(crate::printer::GbPrinter::new(sink)) as Box<dyn SerialLink>),
            LINK_PRINTER,
        ),
        None => (None, LINK_DISCONNECTED),
    }
}

/// The factory both platform descriptors register: the header picks the
/// core. The serial link is a Game Boy peripheral, so it is taken here, with
/// prints landing in the game's folder.
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
    let (link, kind) = link_port(
        media.serial_link.take(),
        media.print_sink,
        media.peripherals,
    );
    let boot_rom = match media.launch.file(BOOT_ROM) {
        Some(bytes) => Some(
            BootRom::from_bytes(bytes.to_vec())
                .map_err(|length| format!("{BOOT_ROM}: {length} bytes is no boot ROM image"))?,
        ),
        None => None,
    };
    let runner = RunnerPreference::from_launch(&media.launch)
        .map_err(|value| format!("{RUNNER}: no such console \"{value}\""))?;
    let board =
        board_from_launch(&media.launch).map_err(|refusal| format!("{BOARD}: {refusal}"))?;
    launch(
        media.rom.to_vec(),
        board,
        media.save_data,
        boot_rom,
        link,
        runner,
        Boxed { link: kind },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_gb::frame::GameBoyScreen;
    use missingno_gb::ppu::screen::Screen;
    use missingno_gb::ppu::types::palette::PaletteIndex;

    /// A user-configured cable, which owes the link port nothing else.
    struct Cable;
    impl SerialLink for Cable {
        fn exchange_bit(&mut self, _out_bit: bool) -> bool {
            true
        }
        fn clock(&mut self) -> bool {
            false
        }
    }

    fn sink() -> Option<crate::printer::PrintSink> {
        Some(std::sync::mpsc::channel().0)
    }

    /// The printer is a box fact, so it attaches only where the catalogue
    /// states the release is played with one.
    #[test]
    fn a_printer_attaches_only_where_the_release_states_one() {
        let (printer, kind) = link_port(None, sink(), &[Peripheral::Printer]);
        assert!(printer.is_some());
        assert_eq!(kind, LINK_PRINTER);

        for stated in [&[][..], &[Peripheral::LinkCable]] {
            let (link, kind) = link_port(None, sink(), stated);
            assert!(link.is_none(), "{stated:?} attached something");
            assert_eq!(kind, LINK_DISCONNECTED);
        }
    }

    /// The cable the user configured outranks the catalogue's word.
    #[test]
    fn an_explicit_cable_takes_the_link_port() {
        let (link, kind) = link_port(Some(Box::new(Cable)), sink(), &[Peripheral::Printer]);
        assert!(link.is_some());
        assert_eq!(kind, LINK_CABLE);
    }

    fn policy(use_sgb_colors: bool) -> GbPalettePolicy {
        GbPalettePolicy {
            palette: PaletteChoice::Green,
            use_sgb_colors,
        }
    }

    #[test]
    fn the_policy_states_the_chosen_panels_five_stops() {
        let palette = PaletteChoice::Green.palette();
        let stops = policy(false).response_stops().unwrap();
        assert_eq!(
            *stops,
            [
                palette.disabled(),
                palette.color(PaletteIndex(0)),
                palette.color(PaletteIndex(1)),
                palette.color(PaletteIndex(2)),
                palette.color(PaletteIndex(3)),
            ]
        );
    }

    #[test]
    fn the_policy_passes_the_frames_levels_through() {
        // The frame states the axis; the policy only decides whether to use it.
        let frame = GbFrame::GameBoy(GameBoyScreen::Display(Screen::default()));
        let levels = policy(false).response_levels(&frame).unwrap();
        assert_eq!(*levels, *frame.response_levels().unwrap());
        assert!(levels.iter().all(|&level| level == 0.25));
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
