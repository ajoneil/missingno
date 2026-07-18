//! The Game Boy family's load-path registration: media recognition, control
//! labels, and the console factory over the crate's seam implementation. The
//! header picks the core; the serial link, printer, boot ROM, and battery-save
//! format are frontend policy wired in here.

use missingno_gb::system::GbConsole;
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge, serial_transfer::SerialLink};
use missingno_gbc::GameBoyColor;

use super::{MediaLoad, SystemConsole, SystemDebugger};

pub use missingno_gb::media::{is_gb_rom, is_gbc_rom, title_from_rom};

/// Dual-mode media ships as `.gbc` files, so the Game Boy platform's dialog
/// filter must include that extension too.
pub const ROM_EXTENSIONS: &[&str] = &["gb", "gbc"];
pub const GBC_ROM_EXTENSIONS: &[&str] = &["gbc"];
pub const DEFAULT_ROM_EXTENSION: &str = "gb";
pub const SAVE_FILTER_NAME: &str = "Game Boy Save";
pub const SAVE_EXTENSIONS: &[&str] = &["sav"];

/// The family's names for the shared control ids, indexed by id; also the
/// bindings UI's primary labels.
pub const CONTROL_LABELS: [&str; 8] = ["Start", "Select", "A", "B", "Up", "Down", "Left", "Right"];

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
/// trace, headless): CGB-aware media — enhanced or required — boots the CGB
/// core, like a cartridge slotted into a real GBC; DMG-only media boots the
/// DMG core.
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
pub fn create_console(media: MediaLoad) -> Option<Box<dyn SystemConsole>> {
    struct Boxed;
    impl GbLaunch for Boxed {
        type Output = Box<dyn SystemConsole>;
        fn dmg(self, console: GameBoy) -> Self::Output {
            Box::new(GbConsole::new(console, battery_save))
        }
        fn cgb(self, console: GameBoyColor) -> Self::Output {
            Box::new(GbConsole::new(console, battery_save))
        }
    }
    let link = media.serial_link.take().or_else(|| {
        media
            .print_sink
            .map(|sink| Box::new(crate::printer::GbPrinter::new(sink)) as Box<dyn SerialLink>)
    });
    Some(launch(
        media.rom.to_vec(),
        media.save_data,
        media.boot_rom,
        link,
        Boxed,
    ))
}

/// Build a headless debugger over the same seam the GUI uses, wiring the boot
/// ROM, an optional serial link, and the battery-save format. The headless
/// server never persists, so the battery-save hook stays inert.
pub fn headless_debugger(
    rom: Vec<u8>,
    save_data: Option<Vec<u8>>,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
) -> Box<dyn SystemDebugger> {
    struct Build;
    impl GbLaunch for Build {
        type Output = Box<dyn SystemConsole>;
        fn dmg(self, console: GameBoy) -> Self::Output {
            Box::new(GbConsole::new(console, battery_save))
        }
        fn cgb(self, console: GameBoyColor) -> Self::Output {
            Box::new(GbConsole::new(console, battery_save))
        }
    }
    match launch(rom, save_data, boot_rom, link, Build).into_debugger() {
        Ok(debugger) => debugger,
        Err(_) => unreachable!("the Game Boy console always has a debugger backend"),
    }
}
