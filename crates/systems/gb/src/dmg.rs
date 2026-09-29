use crate::audio::DmgApu;
use crate::cartridge::Cartridge;
use crate::chassis::{Chassis, Console};
use crate::model::Model;
use crate::{ppu, sgb};
use missingno_core::state::StateRecord;
use missingno_core::system::StateError;

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
        if let Some(sgb) = &self.sgb {
            let p14_selected = value & 0x10 == 0;
            let p15_selected = value & 0x20 == 0;
            if !p14_selected && !p15_selected {
                return (value & 0xF0) | (0x0F - sgb.joypad_index);
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

    /// A record without an SGB block predates it, so the live SGB stays.
    fn restore_boundary_delta(
        &mut self,
        _chassis: &mut Chassis<Self>,
        record: &StateRecord,
        memory: &[(String, Vec<u8>)],
    ) -> Result<(), StateError> {
        match (sgb::Sgb::from_state(record, memory)?, &mut self.sgb) {
            (Some(saved), Some(sgb)) => *sgb = saved,
            (Some(_), None) => return Err(StateError::IncompatibleRom),
            (None, _) => {}
        }
        Ok(())
    }
}

/// The original Game Boy.
pub type GameBoy = Console<Dmg>;

impl Console<Dmg> {
    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.model.sgb.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use missingno_core::state::StateRecord;
    use missingno_core::system::StateError;

    use super::GameBoy;
    use crate::cartridge::Cartridge;
    use crate::system::ConsoleUi;

    fn console(sgb: bool) -> GameBoy {
        let mut rom = vec![0u8; 0x8000];
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]);
        if sgb {
            (rom[0x146], rom[0x14B]) = (0x03, 0x33);
        }
        let mut console = GameBoy::new(Cartridge::new(rom, None, None).unwrap(), None);
        for _ in 0..1000 {
            console.step();
        }
        console
    }

    type Captured = (StateRecord, Vec<(String, Vec<u8>)>);

    fn capture(console: &GameBoy) -> Captured {
        let record = <super::Dmg as ConsoleUi>::read_state(console).unwrap();
        let memory = <super::Dmg as ConsoleUi>::capture_memory(console)
            .into_iter()
            .map(|(name, data)| (name.to_owned(), data))
            .collect();
        (record, memory)
    }

    fn restore(console: &mut GameBoy, (record, memory): &Captured) -> Result<(), StateError> {
        console.restore_boundary(record, memory.clone(), None)
    }

    #[test]
    fn a_restore_keeps_the_dot_phase() {
        let source = console(false);
        let phase = source.ppu().dot_in_mcycle_phase();
        assert!(
            phase.is_some_and(|p| p != 2),
            "needs a phase a default restore would lose"
        );
        let mut target = console(false);
        restore(&mut target, &capture(&source)).unwrap();
        assert_eq!(target.ppu().dot_in_mcycle_phase(), phase);
    }

    #[test]
    fn a_record_from_before_the_dividers_still_loads() {
        let (mut record, memory) = capture(&console(false));
        let mut old = StateRecord::new();
        for (name, value) in record.iter() {
            if !name.ends_with("_divider") {
                old.set(name, value.clone());
            }
        }
        record = old;
        restore(&mut console(false), &(record, memory)).unwrap();
    }

    #[test]
    fn sgb_state_round_trips_through_the_record() {
        let mut source = console(true);
        let sgb = source.model.sgb.as_mut().unwrap();
        // PAL01, one packet: colour 0 = $1234, palette 0 colour 1 = $5678
        let packet = [1u8, 0x34, 0x12, 0x78, 0x56, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        sgb.write_joypad(0x00);
        sgb.write_joypad(0x30);
        for byte in packet {
            for bit in 0..8 {
                sgb.write_joypad(if byte >> bit & 1 != 0 { 0x10 } else { 0x20 });
                sgb.write_joypad(0x30);
            }
        }
        sgb.write_joypad(0x20);
        sgb.write_joypad(0x30);
        let saved = capture(&source);

        let mut target = console(true);
        restore(&mut target, &saved).unwrap();
        assert_eq!(capture(&target), saved);
        let render = target.sgb().unwrap().render_data();
        assert_eq!(render.color_at(0, 0, 0).0, 0x1234);
        assert_eq!(render.color_at(0, 0, 1).0, 0x5678);
    }

    #[test]
    fn an_sgb_record_does_not_load_onto_a_plain_cartridge() {
        let saved = capture(&console(true));
        assert!(matches!(
            restore(&mut console(false), &saved),
            Err(StateError::IncompatibleRom)
        ));
    }
}
