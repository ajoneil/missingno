use crate::audio::{Audio, DmgApu};
use crate::cartridge::Cartridge;
use crate::chassis::{Chassis, Console};
use crate::cpu::Cpu;
use crate::joypad::Joypad;
use crate::model::Model;
use crate::ppu::Ppu;
use crate::serial_transfer::Serial;
use crate::timers::Timers;
use crate::{ppu, sgb};
use missingno_core::state::StateRecord;
use missingno_core::system::StateError;

/// Where a cartridge with the SGB flag starts without a boot ROM.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SgbHandoff {
    /// The SGB2 boot ROM's handoff, as the cartridge runs on one.
    #[default]
    Sgb2,
    /// The DMG boot ROM's, as if the SGB had the DMG's boot ROM.
    Dmg,
}

/// The original Game Boy (DMG): SGB co-processor support, the OAM
/// corruption bug, and a 2-bit shade framebuffer.
#[derive(Default)]
pub struct Dmg {
    sgb: Option<sgb::Sgb>,
    sgb_handoff: SgbHandoff,
    /// The SGB commands completed since a client last took them, while it
    /// asks for them.
    sgb_commands: Option<Vec<Vec<u8>>>,
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
        if let Some(sgb) = &mut self.sgb
            && let Some(command) = sgb.write_joypad(value)
            && let Some(log) = &mut self.sgb_commands
        {
            log.push(command);
        }
    }

    fn on_reset(&mut self, cartridge: &Cartridge, has_boot_rom: bool) {
        self.sgb = cartridge.supports_sgb().then(|| {
            if has_boot_rom || self.sgb_handoff == SgbHandoff::Dmg {
                sgb::Sgb::new()
            } else {
                sgb::Sgb::post_boot()
            }
        });
    }

    fn cpu_post_boot(&self, cartridge: &Cartridge) -> Cpu {
        if self.sgb2_handoff(cartridge) {
            sgb::post_boot::cpu()
        } else {
            Cpu::post_boot(cartridge.header_checksum())
        }
    }

    fn timers_post_boot(&self, cartridge: &Cartridge) -> Timers {
        if self.sgb2_handoff(cartridge) {
            sgb::post_boot::timers(cartridge)
        } else {
            Timers::post_boot()
        }
    }

    fn ppu_post_boot(&self, cartridge: &Cartridge) -> Ppu<Self::Ppu> {
        if self.sgb2_handoff(cartridge) {
            sgb::post_boot::ppu(cartridge)
        } else {
            Ppu::post_boot()
        }
    }

    fn audio_post_boot(&self, internal_counter: u16, cartridge: &Cartridge) -> Audio<DmgApu> {
        if self.sgb2_handoff(cartridge) {
            sgb::post_boot::audio(internal_counter)
        } else {
            Audio::post_boot(internal_counter)
        }
    }

    fn joypad_post_boot(&self, cartridge: &Cartridge) -> Joypad {
        if self.sgb2_handoff(cartridge) {
            sgb::post_boot::joypad()
        } else {
            Joypad::new()
        }
    }

    fn serial_post_boot(&self, internal_counter: u16, cartridge: &Cartridge) -> Serial {
        if self.sgb2_handoff(cartridge) {
            sgb::post_boot::serial(internal_counter)
        } else {
            Serial::new()
        }
    }

    fn post_boot_memory(&self, chassis: &mut Chassis<Self>) {
        if self.sgb2_handoff(&chassis.external.cartridge) {
            sgb::post_boot::memory(&mut chassis.external, &mut chassis.high_ram);
        }
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

impl Dmg {
    pub fn with_sgb_handoff(sgb_handoff: SgbHandoff) -> Self {
        Self {
            sgb_handoff,
            ..Self::default()
        }
    }

    // A cartridge with the SGB flag runs on a Super Game Boy 2.
    fn sgb2_handoff(&self, cartridge: &Cartridge) -> bool {
        cartridge.supports_sgb() && self.sgb_handoff == SgbHandoff::Sgb2
    }
}

/// The original Game Boy.
pub type GameBoy = Console<Dmg>;

impl Console<Dmg> {
    pub fn sgb(&self) -> Option<&sgb::Sgb> {
        self.model.sgb.as_ref()
    }

    /// Keep each command the SGB completes from now on, for
    /// [`take_sgb_commands`](Self::take_sgb_commands), or stop.
    pub fn observe_sgb_commands(&mut self, on: bool) {
        self.model.sgb_commands = on.then(Vec::new);
    }

    /// The SGB commands completed since the last take, each its packets'
    /// bytes, in order.
    pub fn take_sgb_commands(&mut self) -> Vec<Vec<u8>> {
        self.model
            .sgb_commands
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
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
    fn a_capture_reads_as_a_synced_console_would() {
        let mut console = console(false);
        let mut deferred = false;
        for _ in 0..20_000 {
            console.step();
            let unsynced = capture(&console);
            deferred |= unsynced.0.get("lx") != Some(&console.ppu().lx().into());
            console.sync_ppu();
            console.sync_audio();
            assert_eq!(capture(&console), unsynced);
        }
        assert!(deferred, "needs a capture the PPU's span had deferred");
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
    fn an_sgb_cartridge_starts_from_the_handoff_asked_for() {
        let mut rom = vec![0u8; 0x8000];
        (rom[0x146], rom[0x14B]) = (0x03, 0x33);
        let div = |handoff| {
            let cartridge = Cartridge::new(rom.clone(), None, None).unwrap();
            let console =
                GameBoy::with_model(cartridge, None, super::Dmg::with_sgb_handoff(handoff));
            (console.cpu().a, console.peek_range(0xFF04, 1)[0])
        };
        assert_eq!(div(super::SgbHandoff::Sgb2), (0xFF, 0xD8));
        assert_eq!(div(super::SgbHandoff::Dmg), (0x01, 0xAB));
    }

    #[test]
    fn sgb_state_round_trips_through_the_record() {
        let mut source = console(true);
        // A boundary restore rebuilds OAM scan as mode 0, so save in VBlank.
        while source.peek_range(0xFF44, 1)[0] < 144 {
            source.step();
        }
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
