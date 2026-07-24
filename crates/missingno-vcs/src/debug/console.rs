//! The plain-console half of the seam: a VCS driven frame by frame through a
//! television, with no debugging backend attached.

use std::time::Duration;

use missingno_core::state::{StateRecord, SystemStateSchema};
use missingno_core::system::{
    ConsoleSwitch, ControlId, ControlInput, FrameOutcome, StateError, SystemConsole, SystemDebugger,
};
use missingno_core::video::{
    self, DisplayTechnology, Frame as VideoFrame, IndexedFrame, Television,
};

use crate::TvStandard;
use crate::cartridge::CartridgeError;
use crate::console::Vcs;
use crate::state_schema::vcs_state_schema;
use crate::tia::VISIBLE_CLOCKS;
use crate::tv_standard::pixel_aspect;

use super::controls::{CONSOLE_SWITCHES, apply_control};
use super::debugger_seam::VcsDebugger;
use super::frame::{
    FRAME_BUDGET_LINES, VSYNC_LOCK_LINES, blank_frame, frame_interval, indexed_frame,
};
use super::probe::{core_cart_type, probe_tv_standard};
use super::save_state::{load_state_into, rom_fingerprint, save_state_bytes};

pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    cart_type: Option<&str>,
) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    // The library's metadata is authoritative; carts carry no region header and
    // the size heuristic can't always name the board, so fall back only when
    // the game-db is silent — then probe the standard from the ROM's own field
    // length. Pacing, aspect, and palette follow the standard.
    let cart = cart_type.and_then(core_cart_type);
    let region = match tv_standard {
        Some(standard) => standard,
        None => probe_tv_standard(rom, cart),
    };
    Ok(Box::new(VcsConsole::new(
        Vcs::new(rom, region, cart)?,
        title,
        rom_fingerprint(rom),
        blank_frame(),
    )))
}

pub(super) struct VcsConsole {
    vcs: Vcs,
    title: String,
    rom_sha256: String,
    last_frame: IndexedFrame,
    tv: Television<VISIBLE_CLOCKS>,
}

impl VcsConsole {
    pub(super) fn new(
        vcs: Vcs,
        title: String,
        rom_sha256: String,
        last_frame: IndexedFrame,
    ) -> Self {
        VcsConsole {
            vcs,
            title,
            rom_sha256,
            last_frame,
            tv: Television::new(VSYNC_LOCK_LINES),
        }
    }
}

impl SystemConsole for VcsConsole {
    fn step_frame(&mut self) -> FrameOutcome {
        let standard = self.vcs.tv_standard();
        // Drive the console scanline by scanline through the television, which
        // integrates VSYNC to decide the field. Bounded so a kernel that never
        // syncs cannot stall the emulation thread.
        let mut display = None;
        for _ in 0..FRAME_BUDGET_LINES {
            let line = self.vcs.step_scanline();
            if let Some(field) = self.tv.feed(video::Scanline {
                pixels: line.pixels,
                vsync: line.vsync,
            }) {
                self.last_frame = indexed_frame(&field.lines, standard);
                display = Some(VideoFrame::Indexed(self.last_frame.clone()));
                break;
            }
        }
        FrameOutcome {
            display,
            sram_dirty: false,
        }
    }

    fn reset(&mut self) {
        self.vcs.power_cycle();
    }

    fn set_control(&mut self, control: ControlId, input: ControlInput) {
        apply_control(&mut self.vcs, control, input);
    }

    fn console_switches(&self) -> &'static [ConsoleSwitch] {
        &CONSOLE_SWITCHES
    }

    fn drain_audio_samples(&mut self) -> Vec<(f32, f32)> {
        self.vcs.drain_audio_samples()
    }

    fn audio_coupling(&self) -> Option<missingno_core::HighPass> {
        Some(crate::board::AUDIO_COUPLING.high_pass())
    }

    fn screen_display(&self) -> VideoFrame {
        VideoFrame::Indexed(self.last_frame.clone())
    }

    fn video_out(&self) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: self.vcs.tv_standard(),
            pixel_aspect: pixel_aspect(self.vcs.tv_standard()),
        }
    }

    fn game_title(&self) -> String {
        self.title.clone()
    }

    fn frame_interval(&self) -> Duration {
        frame_interval(self.vcs.tv_standard())
    }

    fn state_schema(&self) -> Option<&'static SystemStateSchema> {
        Some(vcs_state_schema())
    }

    fn read_state(&self) -> Option<StateRecord> {
        Some(crate::snapshot::read_state(&self.vcs))
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        save_state_bytes(&self.vcs, &self.last_frame, &self.rom_sha256)
    }

    fn load_state(&mut self, bytes: &[u8]) -> Result<(), StateError> {
        load_state_into(&mut self.vcs, bytes, &self.rom_sha256)
    }

    fn into_debugger(self: Box<Self>) -> Box<dyn SystemDebugger> {
        Box::new(VcsDebugger::new(
            crate::debugger::Debugger::new(self.vcs),
            self.title,
            self.rom_sha256,
            self.last_frame,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_out_reports_a_crt_with_the_carts_standard() {
        // A 4 KiB ROM whose reset vector points at its origin; the caller-
        // supplied standard maps straight onto the CRT descriptor.
        let mut rom = vec![0xEA; 0x1000];
        rom[0xFFC] = 0x00;
        rom[0xFFD] = 0xF0;
        for standard in [TvStandard::Ntsc, TvStandard::Pal, TvStandard::Secam] {
            let console =
                create_console(&rom, "test".into(), Some(standard), None).expect("console builds");
            match console.video_out() {
                DisplayTechnology::Crt {
                    standard: reported,
                    pixel_aspect,
                } => {
                    assert_eq!(reported, standard);
                    assert_eq!(pixel_aspect, crate::tv_standard::pixel_aspect(standard));
                }
                other => panic!("VCS should drive a CRT, got {other:?}"),
            }
        }
    }
}
