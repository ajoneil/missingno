//! The ColecoVision's debugger seam: its inspection state and the machine
//! binding over the console. The Z80, the VDP and the PSG present themselves
//! through their own chip crates; the board adds the launch surface, its ports
//! and its panel.

mod ports;

use std::time::Duration;

use missingno_core::TvStandard;
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect::{RegisterGroup, Section};
use missingno_core::launch::{LaunchOptionDescriptor, tv_standard_option};
use missingno_core::machine::{
    BoundaryState, Machine, MachineConsole, StateIdentity, rom_fingerprint,
};
use missingno_core::ports::{PanelControl, PeripheralId, PlugError, PortDescriptor, PortId};
use missingno_core::state::{StateRecord, SystemStateSchema};
use missingno_core::state_file::StateFrame;
use missingno_core::system::{
    ControlId, ControlInput, DebugView, InspectSnapshot, RunningStatus, StateError, SystemConsole,
};
use missingno_core::video::{DisplayTechnology, Frame, IndexedFrame};
use missingno_core::waveform::ChannelWave;
use missingno_ti_psg::inspect::Registers as PsgRegisters;
use missingno_ti_vdp::inspect::{VdpView, ti_palette};
use missingno_ti_vdp::{Frame as VdpFrame, Standard, VISIBLE_WIDTH};
use missingno_zilog_z80::inspect::RegisterFile;

use crate::console::{CLOCK_HZ, ColecoVision, PORT1, PORT2, part_for, tstates_per_frame};
use crate::firmware::{BIOS_SIZE, bios_option};
use crate::state_schema::colecovision_state_schema;
use ports::HAND_CONTROLLER;

pub use crate::cartridge::title_from_rom;
pub use ports::{PANEL, PORTS};

/// Pixel aspect at the VDP's 5.37 MHz dot clock — a display-side calibratable
/// stage. PAL paints the same line time's 313 lines into the 625-line height
/// that 262 fill on 525, so its pixels are 25/21 wider.
fn pixel_aspect(standard: TvStandard) -> f32 {
    match standard {
        TvStandard::Pal => 8.0 / 7.0 * 25.0 / 21.0,
        _ => 8.0 / 7.0,
    }
}

#[derive(Clone)]
pub struct ColecoVisionInspectState {
    pub registers: RegisterFile,
    pub vdp: VdpView,
    pub psg: PsgRegisters,
    pub frame: u64,
}

/// The per-frame snapshot for the running view.
pub struct ColecoVisionSnapshot {
    pub state: ColecoVisionInspectState,
}

impl InspectSnapshot for ColecoVisionSnapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
    fn register_groups(&self) -> Vec<RegisterGroup> {
        missingno_zilog_z80::inspect::register_groups(&self.state.registers)
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        sidebar_sections(&self.state)
    }
}

/// The Z80 register file, the VDP's position/status/registers, and the PSG's
/// channels. The board has no mapper to show.
fn sidebar_sections(state: &ColecoVisionInspectState) -> Vec<Section> {
    vec![
        missingno_core::inspect::cpu_section(missingno_zilog_z80::inspect::register_groups(
            &state.registers,
        )),
        missingno_ti_vdp::inspect::section(&state.vdp),
        missingno_ti_psg::inspect::section(&state.psg, CLOCK_HZ),
    ]
}

/// The VDP's visible raster under the datasheet palette.
fn indexed(frame: &VdpFrame) -> IndexedFrame {
    IndexedFrame {
        width: frame.width as u32,
        height: frame.height as u32,
        pixels: frame.pixels.as_slice().into(),
        palette: ti_palette(),
    }
}

/// ColecoVision media is recognised by its `.col` file extension.
pub fn is_colecovision_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("col"))
}

/// The options the ColecoVision accepts at launch: the standard the board was
/// cut for, and the BIOS in its socket.
pub fn launch_options(_rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
    vec![
        tv_standard_option([TvStandard::Ntsc, TvStandard::Pal]),
        bios_option(),
    ]
}

/// A console bound to its media, so a save state can refuse a ROM it was not
/// written for. An unstated standard is NTSC, the board's home-market cut.
pub fn create_console(
    rom: &[u8],
    title: String,
    tv_standard: Option<TvStandard>,
    bios: [u8; BIOS_SIZE],
) -> Result<Box<dyn SystemConsole>, String> {
    let standard = match tv_standard {
        None => Standard::Ntsc,
        Some(stated) => part_for(stated)
            .ok_or_else(|| format!("the ColecoVision was not cut for {}", stated.display_name()))?,
    };
    let cv = ColecoVision::new(rom, standard, bios).map_err(|error| error.to_string())?;
    let console = MachineConsole::<ColecoVisionSystem>::new(cv, title);
    Ok(Box::new(console.with_identity(StateIdentity {
        rom_fingerprint: rom_fingerprint(rom),
    })))
}

pub struct ColecoVisionSystem;

impl Machine for ColecoVisionSystem {
    type Core = ColecoVision;
    type Frame = IndexedFrame;
    type InspectState = ColecoVisionInspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);

    fn frame_interval(cv: &ColecoVision) -> Duration {
        Duration::from_secs_f64(tstates_per_frame(cv.standard()) as f64 / CLOCK_HZ as f64)
    }

    const RUN_BUDGET: u32 = 400_000;

    fn pc(cv: &ColecoVision) -> u16 {
        cv.cpu.pc
    }

    fn peek(cv: &ColecoVision, address: u16) -> u8 {
        cv.peek(address)
    }

    fn instruction_set() -> Option<&'static dyn missingno_core::isa::InstructionSet> {
        Some(&missingno_zilog_z80::Z80)
    }

    fn step_instruction(cv: &mut ColecoVision) {
        cv.step_instruction();
    }

    fn take_frame(cv: &mut ColecoVision) -> Option<IndexedFrame> {
        cv.take_frame().map(indexed)
    }

    fn step_frame(cv: &mut ColecoVision) -> Option<IndexedFrame> {
        // Only a wait chain can stretch a frame past its T-states.
        cv.step_frame(4 * tstates_per_frame(cv.standard()))
            .map(indexed)
    }

    fn power_cycle(cv: &mut ColecoVision) {
        cv.power_cycle();
    }

    fn apply_control(cv: &mut ColecoVision, control: ControlId, input: ControlInput) {
        cv.apply_control(control, input);
    }

    fn ports() -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(_cv: &ColecoVision, port: PortId) -> Option<PeripheralId> {
        matches!(port, PORT1 | PORT2).then_some(HAND_CONTROLLER)
    }

    fn plug(
        _cv: &mut ColecoVision,
        port: PortId,
        peripheral: PeripheralId,
    ) -> Result<(), PlugError> {
        match (port, peripheral) {
            (PORT1 | PORT2, HAND_CONTROLLER) => Ok(()),
            (PORT1 | PORT2, _) => Err(PlugError::NotAccepted),
            _ => Err(PlugError::UnknownPort),
        }
    }

    fn panel_controls() -> &'static [PanelControl] {
        PANEL
    }

    fn drain_audio_samples(cv: &mut ColecoVision) -> Vec<(f32, f32)> {
        cv.drain_audio_samples()
    }

    fn video_out(cv: &ColecoVision) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: cv.tv_standard(),
            pixel_aspect: pixel_aspect(cv.tv_standard()),
        }
    }

    fn display_frame(frame: &IndexedFrame) -> Frame {
        Frame::Indexed(frame.clone())
    }

    fn blank_display() -> Frame {
        Frame::Indexed(IndexedFrame::blank(
            VISIBLE_WIDTH as u32,
            Standard::Ntsc.visible_lines() as u32,
            ti_palette(),
        ))
    }

    fn state_schema() -> Option<&'static SystemStateSchema> {
        Some(colecovision_state_schema())
    }

    fn read_state(cv: &ColecoVision) -> Option<StateRecord> {
        crate::snapshot::read_state(cv)
    }

    fn capture_boundary(cv: &ColecoVision) -> Result<BoundaryState, StateError> {
        crate::snapshot::capture(cv)
    }

    fn restore_boundary(
        cv: &mut ColecoVision,
        record: &StateRecord,
        memory: &[(String, Vec<u8>)],
        frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        crate::snapshot::restore(cv, record, memory, frame)
    }

    fn step_over_target(cv: &ColecoVision) -> Option<u16> {
        missingno_zilog_z80::step_over_target(cv.peek(cv.cpu.pc), cv.cpu.pc)
    }

    fn inspect(cv: &ColecoVision, frame_count: u64) -> ColecoVisionInspectState {
        let psg = cv.psg();
        ColecoVisionInspectState {
            registers: RegisterFile::of(&cv.cpu),
            vdp: VdpView::of(cv.vdp()),
            psg: PsgRegisters {
                tone_periods: psg.tone_periods(),
                attenuations: psg.attenuations(),
                noise_mode: psg.noise_mode(),
                noise_rate: psg.noise_rate(),
                variant: psg.variant(),
            },
            frame: frame_count,
        }
    }

    fn set_wave_capture(cv: &mut ColecoVision, on: bool) {
        cv.set_wave_capture(on);
    }

    fn channel_waves(cv: &ColecoVision) -> Option<Vec<ChannelWave>> {
        cv.channel_waves()
    }

    fn set_graphics_capture(cv: &mut ColecoVision, on: bool) {
        cv.set_graphics_capture(on);
    }

    fn graphics_view(cv: &ColecoVision) -> Option<GraphicsView> {
        cv.graphics_capture()
            .then(|| missingno_ti_vdp::graphics::graphics_view(cv.vdp()))
    }

    fn register_groups(state: &ColecoVisionInspectState) -> Vec<RegisterGroup> {
        missingno_zilog_z80::inspect::register_groups(&state.registers)
    }

    fn sidebar_sections(state: &ColecoVisionInspectState) -> Vec<Section> {
        sidebar_sections(state)
    }

    fn snapshot(state: &ColecoVisionInspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(ColecoVisionSnapshot { state })
    }

    fn running_status(state: &ColecoVisionInspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.registers.pc.into(),
            sp: state.registers.sp.into(),
            video_label: "VDP",
            video_summary: format!("line {} · dot {}", state.vdp.line, state.vdp.dot),
            frame,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn console() -> ColecoVision {
        ColecoVision::new(&[0; 0x2000], Standard::Ntsc, [0; BIOS_SIZE]).expect("a flat image")
    }

    #[test]
    fn the_sidebar_names_the_cpu_and_both_chips() {
        let state = ColecoVisionSystem::inspect(&console(), 0);
        let names: Vec<&str> = sidebar_sections(&state)
            .iter()
            .map(|section| section.name)
            .collect();
        assert_eq!(names, ["CPU", "VDP", "PSG"]);
    }

    /// 59,736 T for NTSC's 262 lines and 71,364 for PAL's 313, at 3.579545 MHz.
    #[test]
    fn each_cut_paces_and_presents_its_own_standard() {
        for (standard, micros, aspect) in [
            (TvStandard::Ntsc, 16_688, 8.0 / 7.0),
            (TvStandard::Pal, 19_936, 8.0 / 7.0 * 25.0 / 21.0),
        ] {
            let console =
                create_console(&[0; 0x2000], "test".into(), Some(standard), [0; BIOS_SIZE])
                    .expect("a part cut for the standard");
            assert_eq!(console.frame_interval().as_micros(), micros, "{standard:?}");
            assert_eq!(
                console.video_out(),
                DisplayTechnology::Crt {
                    standard,
                    pixel_aspect: aspect,
                }
            );
        }
    }

    #[test]
    fn media_is_recognised_by_extension() {
        assert!(is_colecovision_rom(std::path::Path::new("game.col")));
        assert!(is_colecovision_rom(std::path::Path::new("GAME.COL")));
        assert!(!is_colecovision_rom(std::path::Path::new("game.sg")));
    }

    #[test]
    fn the_launch_publishes_the_bios_socket() {
        let published: Vec<&str> = launch_options(&[]).iter().map(|option| option.id).collect();
        assert_eq!(published, ["tv-standard", crate::firmware::BIOS]);
    }

    #[test]
    fn graphics_capture_off_decodes_nothing() {
        let mut console = console();
        assert!(ColecoVisionSystem::graphics_view(&console).is_none());
        console.set_graphics_capture(true);
        assert!(ColecoVisionSystem::graphics_view(&console).is_some());
    }
}
