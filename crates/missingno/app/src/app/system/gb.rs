//! The Game Boy family's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation. The
//! header picks the core; the serial link, printer, boot ROM, and battery-save
//! format are frontend policy wired in here.

use missingno_gb::frame::GbFrame;
use missingno_gb::ppu::types::palette::{PaletteChoice, PaletteIndex};
use missingno_gb::system::{GbConsole, LINK_CABLE, LINK_DISCONNECTED, LINK_PRINTER};
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge, serial_transfer::SerialLink};
use missingno_gbc::GameBoyColor;

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
        // The lightest palette shade is the panel's unlit paper — what the
        // reflective LCD shows through the inter-pixel matrix. SGB colours
        // don't draw from this palette, so no shade names the paper tone.
        (!self.use_sgb_colors).then(|| self.palette.palette().color(PaletteIndex(0)))
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

/// The console's pad and its link port; the Game Boy has no panel controls.
pub const CONTROLS: ControlMap =
    ControlMap::new(missingno_gb::system::PAD, missingno_gb::system::PORTS, &[]);

/// The battery-backed contents to persist: raw SRAM plus the wall-clock RTC
/// tail. The save-file format is frontend policy, so the core takes this as a
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

/// Receives the console `launch` selects. Two concrete arms rather than one
/// generic method so a caller can require its own model traits on each.
pub trait GbLaunch {
    type Output;
    fn dmg(self, console: GameBoy) -> Self::Output;
    fn cgb(self, console: GameBoyColor) -> Self::Output;
}

/// The one DMG-vs-CGB selection point for every executable path (GUI load,
/// trace): CGB-aware media — enhanced or required — boots the CGB core, like a
/// cartridge slotted into a real GBC; DMG-only media boots the DMG core.
pub fn launch<L: GbLaunch>(
    rom: Vec<u8>,
    save_data: Option<Vec<u8>>,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
    launcher: L,
) -> L::Output {
    let cartridge = build_cartridge(rom, save_data);
    let boot_rom = matching_boot_rom(boot_rom, cartridge.is_cgb());
    if cartridge.is_cgb() {
        let mut console = GameBoyColor::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.cgb(console)
    } else {
        let mut console = GameBoy::new(cartridge, boot_rom);
        if let Some(link) = link {
            console.set_link(link);
        }
        launcher.dmg(console)
    }
}

fn matching_boot_rom(boot_rom: Option<BootRom>, cgb_core: bool) -> Option<BootRom> {
    match (&boot_rom, cgb_core) {
        (Some(BootRom::Dmg(_)), true) | (Some(BootRom::Cgb(_)), false) => {
            eprintln!("warning: boot ROM model does not match the selected core; ignoring it");
            None
        }
        _ => boot_rom,
    }
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
            Box::new(GbConsole::with_link(console, battery_save, self.link))
        }
        fn cgb(self, console: GameBoyColor) -> Self::Output {
            Box::new(GbConsole::with_link(console, battery_save, self.link))
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
