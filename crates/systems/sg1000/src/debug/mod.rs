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
use missingno_core::graphics::GraphicsView;
use missingno_core::inspect::{RegisterGroup, Section};
use missingno_core::machine::{BoundaryState, Machine, MachineConsole, StateIdentity};
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

use crate::cartridge::CartridgeError;
use crate::console::{JOY1, JOY2, STANDARD, Sg1000, TSTATES_PER_FRAME};
use crate::state_schema::sg1000_state_schema;
use palette::ti_palette;
use ports::CONTROL_PAD;

pub use ports::{PANEL, PORTS};
pub use vdp::VdpLayout;

/// T-state budget per frame step; a frame is 59,736 of them, and only a wait
/// chain can stretch one.
const FRAME_BUDGET: u32 = 4 * TSTATES_PER_FRAME;

/// NTSC pixel aspect at the VDP's 5.37 MHz dot clock — a display-side
/// calibratable stage.
const PIXEL_ASPECT: f32 = 8.0 / 7.0;

const CODE_WINDOW_ROWS: usize = 10;

#[derive(Clone)]
pub struct Sg1000InspectState {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
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

/// A console bound to its media, so a save state can refuse a ROM it was not
/// written for.
pub fn create_console(rom: &[u8], title: String) -> Result<Box<dyn SystemConsole>, CartridgeError> {
    let console = MachineConsole::<Sg1000System>::new(Sg1000::new(rom)?, title);
    Ok(Box::new(console.with_identity(StateIdentity {
        rom_fingerprint: rom_fingerprint(rom),
    })))
}

/// SHA-256 of the cartridge image, taken at load — the digest a save state
/// carries.
fn rom_fingerprint(rom: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(rom).into()
}

pub struct Sg1000System;

impl Machine for Sg1000System {
    type Core = Sg1000;
    type Frame = IndexedFrame;
    type InspectState = Sg1000InspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);
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
        sg.step_frame(FRAME_BUDGET).map(indexed)
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

    fn video_out(_sg: &Sg1000) -> DisplayTechnology {
        DisplayTechnology::Crt {
            standard: TvStandard::Ntsc,
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn display_frame(frame: &IndexedFrame) -> Frame {
        Frame::Indexed(frame.clone())
    }

    fn blank_display() -> Frame {
        Frame::Indexed(IndexedFrame::blank(
            VISIBLE_WIDTH as u32,
            STANDARD.visible_lines() as u32,
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
        // CALL and RST push a return path; run to the next address.
        let opcode = sg.peek(sg.cpu.pc);
        let is_call = opcode == 0xCD || (opcode & 0xC7) == 0xC4;
        is_call.then(|| sg.cpu.pc.wrapping_add(3))
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
            a: cpu.a,
            f: cpu.f,
            b: cpu.b,
            c: cpu.c,
            d: cpu.d,
            e: cpu.e,
            h: cpu.h,
            l: cpu.l,
            ix: cpu.ix,
            iy: cpu.iy,
            sp: cpu.sp,
            pc: cpu.pc,
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
            pc: state.pc.into(),
            sp: state.sp.into(),
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
        let console = Sg1000::new(&[0; 0x2000]).expect("flat cartridge image");
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

    #[test]
    fn media_is_recognised_by_extension() {
        assert!(is_sg1000_rom(std::path::Path::new("game.sg")));
        assert!(is_sg1000_rom(std::path::Path::new("GAME.SG")));
        assert!(!is_sg1000_rom(std::path::Path::new("game.sms")));
    }
}
