//! The SG-1000's debugger seam: its inspection state and the machine binding
//! over the console. One owned state struct serves both the paused view
//! (refreshed after every step) and the per-frame snapshot the running view
//! renders from, and one module per chip on the board turns it into
//! sections.

pub mod graphics;

mod cpu;
mod palette;
mod ports;
mod psg;
mod vdp;

use std::time::Duration;

use missingno_core::TvStandard;
use missingno_core::cartridge::BoardVocabulary;
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect::{RegisterGroup, Section};
use missingno_core::launch::{
    LaunchOptionDescriptor, LaunchValue, LaunchValues, board_option, tv_standard_option,
};
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
use missingno_ti_psg::{NoiseMode, NoiseRate, Variant};
use missingno_ti_vdp::{Frame as VdpFrame, Standard, VISIBLE_WIDTH};
use missingno_zilog_z80::inspect::RegisterFile;

use crate::cartridge::CartType;
use crate::console::{CLOCK_HZ, JOY1, JOY2, Sg1000, part_for, tstates_per_frame};
use crate::state_schema::sg1000_state_schema;
use palette::ti_palette;
use ports::CONTROL_PAD;

pub use ports::{PANEL, PORTS};
pub use vdp::VdpLayout;

/// Pixel aspect at the VDP's 5.37 MHz dot clock — a display-side calibratable
/// stage. PAL paints the same line time's 313 lines into the 625-line height
/// that 262 fill on 525, so its pixels are 25/21 wider.
fn pixel_aspect(standard: TvStandard) -> f32 {
    match standard {
        TvStandard::Pal => 8.0 / 7.0 * 25.0 / 21.0,
        _ => 8.0 / 7.0,
    }
}

const CODE_WINDOW_ROWS: usize = 10;

#[derive(Clone)]
pub struct Sg1000InspectState {
    pub registers: RegisterFile,
    pub standard: Standard,
    pub line: u16,
    pub dot: u16,
    pub vdp_status: u8,
    pub vdp_registers: [u8; 8],
    pub vdp_layout: VdpLayout,
    /// SN76489AN: the three tone periods, the four 4-bit attenuations, and
    /// what the noise register selects — read through the part this board
    /// fits, since the variants read their registers differently.
    pub psg_variant: Variant,
    pub psg_periods: [u16; 3],
    pub psg_volumes: [u8; 4],
    pub psg_noise_mode: NoiseMode,
    pub psg_noise_rate: NoiseRate,
    /// Raw bytes at the program counter, for the code window.
    pub code_window: Vec<(u16, [u8; 4])>,
    pub frame: u64,
}

/// The per-frame snapshot for the running view.
pub struct Sg1000Snapshot {
    pub state: Sg1000InspectState,
}

impl Sg1000Snapshot {
    pub fn new(state: Sg1000InspectState) -> Self {
        Sg1000Snapshot { state }
    }
}

impl InspectSnapshot for Sg1000Snapshot {
    fn frame(&self) -> u64 {
        self.state.frame
    }
    fn family_state(&self) -> &dyn std::any::Any {
        &self.state
    }
    fn register_groups(&self) -> Vec<RegisterGroup> {
        cpu::register_groups(&self.state)
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        sidebar_sections(&self.state)
    }
}

/// The sidebar sections, shared by the live view and the running snapshot: the
/// Z80 register file, the VDP's position/status/registers, and the PSG's
/// channels. The board has no mapper to show.
fn sidebar_sections(state: &Sg1000InspectState) -> Vec<Section> {
    vec![
        missingno_core::inspect::cpu_section(cpu::register_groups(state)),
        vdp::section(state),
        psg::section(state),
    ]
}

/// The VDP's visible raster under the palette the console presents its indices
/// through — the one copy a completed frame takes on its way to a consumer.
fn indexed(frame: &VdpFrame) -> IndexedFrame {
    IndexedFrame {
        width: frame.width as u32,
        height: frame.height as u32,
        pixels: frame.pixels.as_slice().into(),
        palette: ti_palette(),
    }
}

/// SG-1000 media is recognised by its `.sg` file extension.
pub fn is_sg1000_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sg"))
}

/// The board the cartridge's silicon sits on.
pub const BOARD: &str = "board";

/// The options the SG-1000 accepts at launch. A cartridge carries no header, so
/// what a catalogue says about its board is all a loader has — the media itself
/// settles nothing.
pub fn launch_options(_rom: &[u8]) -> Vec<LaunchOptionDescriptor> {
    vec![
        tv_standard_option([TvStandard::Ntsc, TvStandard::Pal]),
        board_option(BOARD, CartType::catalogue().iter().cloned()),
    ]
}

/// The board the launch values state, or `None` where nothing states one. A
/// catalogue states the whole board; a picker names one, and the parts it
/// carries stay unmeasured.
pub fn board_from_launch(values: &LaunchValues) -> Result<Option<CartType>, String> {
    match values.value(BOARD) {
        None => Ok(None),
        Some(LaunchValue::Board(board)) => CartType::from_value(board).map(Some),
        Some(LaunchValue::Choice(name)) => CartType::from_name(name)
            .map(Some)
            .ok_or_else(|| format!("unknown SG-1000 board \"{name}\"")),
        Some(_) => Err(format!("the {BOARD} option states a board")),
    }
}

/// A console bound to its media, so a save state can refuse a ROM it was not
/// written for. No header states a region, so an unstated standard is NTSC,
/// the board's home-market cut.
pub fn create_console(
    rom: &[u8],
    title: String,
    cart_type: Option<CartType>,
    tv_standard: Option<TvStandard>,
) -> Result<Box<dyn SystemConsole>, String> {
    let standard = match tv_standard {
        None => Standard::Ntsc,
        Some(stated) => part_for(stated)
            .ok_or_else(|| format!("the SG-1000 was not cut for {}", stated.display_name()))?,
    };
    let sg = Sg1000::new(rom, cart_type, standard).map_err(|error| error.to_string())?;
    let console = MachineConsole::<Sg1000System>::new(sg, title);
    Ok(Box::new(console.with_identity(StateIdentity {
        rom_fingerprint: rom_fingerprint(rom),
    })))
}

pub struct Sg1000System;

impl Machine for Sg1000System {
    type Core = Sg1000;
    type Frame = IndexedFrame;
    type InspectState = Sg1000InspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);

    fn frame_interval(sg: &Sg1000) -> Duration {
        Duration::from_secs_f64(tstates_per_frame(sg.standard()) as f64 / CLOCK_HZ as f64)
    }

    const RUN_BUDGET: u32 = 400_000;

    fn pc(sg: &Sg1000) -> u16 {
        sg.cpu.pc
    }

    fn peek(sg: &Sg1000, address: u16) -> u8 {
        sg.peek(address)
    }

    fn instruction_set() -> Option<&'static dyn missingno_core::isa::InstructionSet> {
        Some(&missingno_zilog_z80::Z80)
    }

    fn step_instruction(sg: &mut Sg1000) {
        sg.step_instruction();
    }

    fn take_frame(sg: &mut Sg1000) -> Option<IndexedFrame> {
        sg.take_frame().map(indexed)
    }

    fn step_frame(sg: &mut Sg1000) -> Option<IndexedFrame> {
        // Only a wait chain can stretch a frame past its T-states.
        sg.step_frame(4 * tstates_per_frame(sg.standard()))
            .map(indexed)
    }

    fn power_cycle(sg: &mut Sg1000) {
        sg.power_cycle();
    }

    fn apply_control(sg: &mut Sg1000, control: ControlId, input: ControlInput) {
        sg.apply_control(control, input);
    }

    fn ports() -> &'static [PortDescriptor] {
        PORTS
    }

    fn plugged(_sg: &Sg1000, port: PortId) -> Option<PeripheralId> {
        matches!(port, JOY1 | JOY2).then_some(CONTROL_PAD)
    }

    fn plug(_sg: &mut Sg1000, port: PortId, peripheral: PeripheralId) -> Result<(), PlugError> {
        match (port, peripheral) {
            (JOY1 | JOY2, CONTROL_PAD) => Ok(()),
            (JOY1 | JOY2, _) => Err(PlugError::NotAccepted),
            _ => Err(PlugError::UnknownPort),
        }
    }

    fn panel_controls() -> &'static [PanelControl] {
        PANEL
    }

    fn drain_audio_samples(sg: &mut Sg1000) -> Vec<(f32, f32)> {
        sg.drain_audio_samples()
    }

    fn video_out(sg: &Sg1000) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: sg.tv_standard(),
            pixel_aspect: pixel_aspect(sg.tv_standard()),
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
        Some(sg1000_state_schema())
    }

    fn read_state(sg: &Sg1000) -> Option<StateRecord> {
        crate::snapshot::read_state(sg)
    }

    /// A save is only faithful at an instruction boundary, where the Z80 holds
    /// no sequencer residue.
    fn capture_boundary(sg: &Sg1000) -> Result<BoundaryState, StateError> {
        crate::snapshot::capture(sg)
    }

    fn restore_boundary(
        sg: &mut Sg1000,
        record: &StateRecord,
        memory: &[(String, Vec<u8>)],
        frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        crate::snapshot::restore(sg, record, memory, frame)
    }

    fn step_over_target(sg: &Sg1000) -> Option<u16> {
        missingno_zilog_z80::step_over_target(sg.peek(sg.cpu.pc), sg.cpu.pc)
    }

    fn inspect(sg: &Sg1000, frame_count: u64) -> Sg1000InspectState {
        let cpu = &sg.cpu;
        let mut code_window = Vec::with_capacity(CODE_WINDOW_ROWS);
        let mut address = cpu.pc;
        for _ in 0..CODE_WINDOW_ROWS {
            code_window.push((
                address,
                [
                    sg.peek(address),
                    sg.peek(address.wrapping_add(1)),
                    sg.peek(address.wrapping_add(2)),
                    sg.peek(address.wrapping_add(3)),
                ],
            ));
            address = address.wrapping_add(4);
        }
        Sg1000InspectState {
            registers: RegisterFile::of(cpu),
            standard: sg.vdp().standard(),
            line: sg.vdp().line(),
            dot: sg.vdp().dot(),
            vdp_status: sg.vdp().peek_status(),
            vdp_registers: *sg.vdp().registers(),
            vdp_layout: VdpLayout::of(sg.vdp()),
            psg_variant: sg.psg().variant(),
            psg_periods: sg.psg().tone_periods(),
            psg_volumes: sg.psg().attenuations(),
            psg_noise_mode: sg.psg().noise_mode(),
            psg_noise_rate: sg.psg().noise_rate(),
            code_window,
            frame: frame_count,
        }
    }

    fn set_wave_capture(sg: &mut Sg1000, on: bool) {
        sg.set_wave_capture(on);
    }

    fn channel_waves(sg: &Sg1000) -> Option<Vec<ChannelWave>> {
        sg.channel_waves()
    }

    fn set_graphics_capture(sg: &mut Sg1000, on: bool) {
        sg.set_graphics_capture(on);
    }

    fn graphics_view(sg: &Sg1000) -> Option<GraphicsView> {
        graphics::graphics_view(sg)
    }

    fn register_groups(state: &Sg1000InspectState) -> Vec<RegisterGroup> {
        cpu::register_groups(state)
    }

    fn sidebar_sections(state: &Sg1000InspectState) -> Vec<Section> {
        sidebar_sections(state)
    }

    fn snapshot(state: &Sg1000InspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(Sg1000Snapshot::new(state))
    }

    fn running_status(state: &Sg1000InspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.registers.pc.into(),
            sp: state.registers.sp.into(),
            video_label: "VDP",
            video_summary: format!("line {} · dot {}", state.line, state.dot),
            frame,
        }
    }
}

/// What the per-chip section tests read a section back through.
#[cfg(test)]
mod fixtures {
    use missingno_core::inspect::{Row, Section, SectionBlock};

    use super::{Machine, Sg1000, Sg1000InspectState, Sg1000System};

    /// What a powered-on board reads, for a test to vary one chip of.
    pub(crate) fn power_on_state() -> Sg1000InspectState {
        let console = Sg1000::new(&[0; 0x2000], None, missingno_ti_vdp::Standard::Ntsc)
            .expect("flat cartridge image");
        Sg1000System::inspect(&console, 0)
    }

    /// Every label/value row a section carries, blocks flattened.
    pub(crate) fn rows(section: &Section) -> Vec<&Row> {
        section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Rows(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .collect()
    }

    pub(crate) fn value_of<'a>(rows: &[&'a Row], label: &str) -> Option<&'a str> {
        rows.iter()
            .find(|row| row.label == label)
            .map(|row| row.value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::power_on_state;
    use super::*;

    #[test]
    fn the_sidebar_names_the_cpu_and_both_chips() {
        let sections = sidebar_sections(&power_on_state());
        let names: Vec<&str> = sections.iter().map(|section| section.name).collect();
        assert_eq!(names, ["CPU", "VDP", "PSG"]);
    }

    /// 59,736 T for NTSC's 262 lines and 71,364 for PAL's 313, at 3.579545 MHz.
    #[test]
    fn each_cut_paces_and_presents_its_own_standard() {
        for (standard, micros, aspect) in [
            (TvStandard::Ntsc, 16_688, 8.0 / 7.0),
            (TvStandard::Pal, 19_936, 8.0 / 7.0 * 25.0 / 21.0),
        ] {
            let console = create_console(&[0; 0x2000], "test".into(), None, Some(standard))
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
        assert!(is_sg1000_rom(std::path::Path::new("game.sg")));
        assert!(is_sg1000_rom(std::path::Path::new("GAME.SG")));
        assert!(!is_sg1000_rom(std::path::Path::new("game.sms")));
    }
}
