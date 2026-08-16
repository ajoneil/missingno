use crate::audio::DmgApu;
use crate::cartridge::Cartridge;
use crate::chassis::Console;
use crate::model::Model;
use crate::{ppu, sgb};

/// The original Game Boy (DMG): SGB co-processor support, the OAM
/// corruption bug, and a 2-bit shade framebuffer.
#[derive(Default)]
pub struct Dmg {
    sgb: Option<sgb::Sgb>,
    /// CGB console arbitration is statically unreachable on DMG — a ZST.
    console_state: (),
}

impl Model for Dmg {
    type Ppu = ppu::model::DmgPpu;
    type Screen = ppu::screen::Screen;
    const HAS_OAM_BUG: bool = true;

    type ConsoleState = ();
    type Apu = DmgApu;

    fn console_state(&self) -> &() {
        &self.console_state
    }
    fn console_state_mut(&mut self) -> &mut () {
        &mut self.console_state
    }

    fn on_present(&mut self, screen: &ppu::screen::Screen) {
        if let Some(sgb) = &mut self.sgb {
            sgb.update_screen(screen);
        }
    }

    fn read_joypad(&self, value: u8) -> u8 {
        if let Some(sgb) = &self.sgb
            && sgb.player_count > 1
        {
            let p14_selected = value & 0x10 == 0;
            let p15_selected = value & 0x20 == 0;
            if !p14_selected && !p15_selected {
                return (value & 0xF0) | (0x0F - sgb.current_player);
            }
        }
        value
    }

    fn on_joypad_write(&mut self, value: u8) {
        if let Some(sgb) = &mut self.sgb {
            sgb.write_joypad(value);
        }
    }

    fn on_reset(&mut self, cartridge: &Cartridge, _has_boot_rom: bool) {
        self.sgb = cartridge.supports_sgb().then(sgb::Sgb::new);
    }
}

/// The original Game Boy.
pub type GameBoy = Console<Dmg>;

impl Console<Dmg> {
    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.model.sgb.as_ref()
    }
}
