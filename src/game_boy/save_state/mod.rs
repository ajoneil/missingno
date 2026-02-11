mod base64;
mod cartridge;
mod sgb;
mod video;

pub use base64::*;
pub use cartridge::*;
pub use sgb::*;
pub use video::*;

use std::{fs, path::Path};

use nanoserde::{DeRon, SerRon};

use super::{
    GameBoy, audio::Audio, cpu::Cpu, cpu::cycles::Cycles,
    interrupts::Registers as InterruptRegisters, joypad::Joypad, memory::Ram, serial_transfer,
    timers::Timers,
};

#[derive(SerRon, DeRon)]
pub struct SaveState {
    rom_title: String,
    rom_checksum: u16,
    cpu: Cpu,
    screen: ScreenState,
    video: VideoState,
    audio: Audio,
    ram: Ram,
    joypad: Joypad,
    interrupts: InterruptRegisters,
    serial: serial_transfer::Registers,
    timers: Timers,
    cartridge: MbcState,
    dma: Option<Cycles>,
    #[nserde(default)]
    dma_source: u8,
    sgb: Option<SgbState>,
}

impl SaveState {
    pub fn capture(gb: &GameBoy) -> Self {
        let mapped = &gb.mapped;

        Self {
            rom_title: gb.cartridge().title().to_string(),
            rom_checksum: gb.cartridge().global_checksum(),
            cpu: gb.cpu.clone(),
            screen: ScreenState::from_screen(&gb.screen),
            video: mapped.video.save_state(),
            audio: mapped.audio.clone(),
            ram: mapped.ram.clone(),
            joypad: mapped.joypad.clone(),
            interrupts: mapped.interrupts.clone(),
            serial: mapped.serial.clone(),
            timers: mapped.timers.clone(),
            cartridge: mapped.cartridge.save_mbc_state(),
            dma: None,
            dma_source: mapped.dma_source,
            sgb: mapped.sgb.as_ref().map(|sgb| sgb.save_state()),
        }
    }

    pub fn into_game_boy(self, rom: Vec<u8>) -> Result<GameBoy, String> {
        use super::{cartridge::Cartridge, video::Video};

        let cartridge = Cartridge::from_state(rom, self.cartridge);

        if cartridge.title() != self.rom_title || cartridge.global_checksum() != self.rom_checksum {
            return Err(format!(
                "Save state is for '{}' (checksum {:04x}), but loaded ROM is '{}' (checksum {:04x})",
                self.rom_title,
                self.rom_checksum,
                cartridge.title(),
                cartridge.global_checksum()
            ));
        }

        let screen = self.screen.to_screen();
        let video = Video::from_state(self.video);

        let sgb = if cartridge.supports_sgb() {
            Some(match self.sgb {
                Some(state) => super::sgb::Sgb::from_state(state),
                None => super::sgb::Sgb::new(),
            })
        } else {
            None
        };

        Ok(GameBoy {
            cpu: self.cpu,
            screen,
            mcycle_counter: 0,
            mapped: super::MemoryMapped {
                cartridge,
                ram: self.ram,
                video,
                audio: self.audio,
                joypad: self.joypad,
                interrupts: self.interrupts,
                serial: self.serial,
                timers: self.timers,
                dma: None,
                dma_source: self.dma_source,
                sgb,
            },
        })
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), String> {
        let data = self.serialize_ron();
        fs::write(path, data).map_err(|e| format!("Failed to write save state: {e}"))
    }

    pub fn load_from_file(path: &Path) -> Result<Self, String> {
        let data =
            fs::read_to_string(path).map_err(|e| format!("Failed to read save state: {e}"))?;
        Self::deserialize_ron(&data).map_err(|e| format!("Failed to parse save state: {e}"))
    }
}
