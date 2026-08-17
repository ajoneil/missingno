//! Core selection for the Game Boy family: the cartridge header names the
//! console it is slotted into, and this crate is the one that knows both.

use missingno_gb::serial_transfer::SerialLink;
use missingno_gb::{BootRom, GameBoy, cartridge::Cartridge};

use crate::GameBoyColor;

/// Receives the console [`console`] selects. Two concrete arms rather than one
/// generic method so a caller can require its own model traits on each.
pub trait GbLaunch {
    type Output;
    fn dmg(self, console: GameBoy) -> Self::Output;
    fn cgb(self, console: GameBoyColor) -> Self::Output;
}

/// What became of a candidate boot ROM. A boot ROM only boots the model it was
/// dumped from, so one that does not match the selected core is dropped rather
/// than forced on it; whether to say so is the caller's policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootRomFit {
    /// The candidate matches the selected core, or none was offered.
    Kept,
    /// The candidate was dumped from the other model, and was dropped.
    Dropped,
}

/// The one DMG-vs-CGB selection point: CGB-aware media — enhanced or required —
/// boots the CGB core, like a cartridge slotted into a real GBC; DMG-only media
/// boots the DMG core. Any serial peripheral goes on the selected console's
/// link port.
pub fn console<L: GbLaunch>(
    cartridge: Cartridge,
    boot_rom: Option<BootRom>,
    link: Option<Box<dyn SerialLink>>,
    launcher: L,
) -> (L::Output, BootRomFit) {
    let cgb_core = cartridge.is_cgb();
    let (boot_rom, fit) = match (&boot_rom, cgb_core) {
        (Some(BootRom::Dmg(_)), true) | (Some(BootRom::Cgb(_)), false) => {
            (None, BootRomFit::Dropped)
        }
        _ => (boot_rom, BootRomFit::Kept),
    };
    let output = if cgb_core {
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
    };
    (output, fit)
}
