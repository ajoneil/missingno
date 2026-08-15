//! The SG-1000's debugger seam: its inspection state and the stepping-system
//! binding over the console. One owned state struct serves both the paused
//! view (refreshed after every step) and the per-frame snapshot the running
//! view renders from.

use std::sync::Arc;
use std::time::Duration;

use missingno_core::inspect::{
    BitColumn, BitRow, BitTable, Register, RegisterGroup, Row, Section, SectionBlock, Sweep,
    SweepZone, Tone, ValueStyle,
};
use missingno_core::ports::{
    ControlDescriptor, ControlKind, PanelBehaviour, PanelControl, PeripheralDescriptor,
    PeripheralId, PlugError, PortDescriptor, PortId, Provider,
};
use missingno_core::stepping::SteppingSystem;
use missingno_core::system::{
    ControlId, ControlInput, ControlRole, DebugView, InspectSnapshot, RunningStatus,
};
use missingno_core::video::IndexedFrame;
use missingno_ti_vdp::Frame;
use rgb::RGB8;

use crate::console::{JOY1, JOY2, Sg1000};

/// T-state budget per frame step; a frame is 59,736 of them, and only a wait
/// chain can stretch one.
const FRAME_BUDGET: u32 = 4 * TSTATES_PER_FRAME;

/// One NTSC frame: 262 lines of 228 T-states at 3.579545 MHz.
const TSTATES_PER_FRAME: u32 = 228 * 262;

/// NTSC pixel aspect at the VDP's 5.37 MHz dot clock — a display-side
/// calibratable stage.
const PIXEL_ASPECT: f32 = 8.0 / 7.0;

/// The raster as the VDP counts it, and the active window inside it.
const DOTS_PER_LINE: u32 = 342;
const LINES_PER_FRAME: u32 = 262;
const PIXELS_PER_LINE: u32 = 256;
const ACTIVE_LINES: u32 = 192;

const CODE_WINDOW_ROWS: usize = 10;

/// The canonical TI datasheet palette. The chip stops at colour indices, so
/// resolving them to RGB is presentation policy the console states; index 0 is
/// the all-planes-transparent external-video pass-through and presents black.
const TI_PALETTE: [[u8; 3]; 16] = [
    [0, 0, 0],
    [0, 0, 0],
    [33, 200, 66],
    [94, 220, 120],
    [84, 85, 237],
    [125, 118, 252],
    [212, 82, 77],
    [66, 235, 245],
    [252, 85, 84],
    [255, 121, 120],
    [212, 193, 84],
    [230, 206, 128],
    [33, 176, 59],
    [201, 91, 186],
    [204, 204, 204],
    [255, 255, 255],
];

#[derive(Clone, Default)]
pub struct Sg1000InspectState {
    pub a: u8,
    pub f: u8,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub ix: u16,
    pub iy: u16,
    pub sp: u16,
    pub pc: u16,
    pub line: u16,
    pub dot: u16,
    pub vdp_status: u8,
    pub vdp_registers: [u8; 8],
    /// SN76489AN: the three tone periods, the four 4-bit attenuations, and the
    /// noise-control byte.
    pub psg_periods: [u16; 3],
    pub psg_volumes: [u8; 4],
    pub psg_noise: u8,
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
        cpu_register_groups(&self.state)
    }
    fn sidebar_sections(&self) -> Vec<Section> {
        sg1000_sidebar_sections(&self.state)
    }
}

/// The Z80 register file as one inspection group, shared by the live view and
/// the running snapshot. `f` renders as a plain byte — no validated Z80 flag
/// table exists yet.
fn cpu_register_groups(state: &Sg1000InspectState) -> Vec<RegisterGroup> {
    let hex = |name, value: u32, bits| Register {
        name,
        value,
        bits,
        style: ValueStyle::Hex,
        help: None,
    };
    vec![RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("a", state.a as u32, 8).help("accumulator"),
            hex("f", state.f as u32, 8).help("flags register"),
            hex("bc", state.bc as u32, 16).help("general-purpose register pair BC"),
            hex("de", state.de as u32, 16).help("general-purpose register pair DE"),
            hex("hl", state.hl as u32, 16).help("general-purpose register pair HL"),
            hex("ix", state.ix as u32, 16).help("index register IX"),
            hex("iy", state.iy as u32, 16).help("index register IY"),
            hex("sp", state.sp as u32, 16).help("stack pointer"),
            hex("pc", state.pc as u32, 16).help("program counter"),
        ],
    }]
}

/// The sidebar sections, shared by the live view and the running snapshot: the
/// Z80 register file, the VDP's position/status/registers, and the PSG's
/// channels. The board has no mapper to show.
fn sg1000_sidebar_sections(state: &Sg1000InspectState) -> Vec<Section> {
    let mut sections = missingno_core::inspect::default_sections(cpu_register_groups(state));
    sections.push(vdp_section(state));
    sections.push(psg_section(state));
    sections
}

/// The SN76489AN: three tone channels and one noise channel. Each channel
/// shows its period (or the noise-control byte) and 4-bit attenuation, with an
/// audibility pip — attenuation $F fully mutes, so anything below it is audible.
fn psg_section(state: &Sg1000InspectState) -> Section {
    let tone = |i: usize, label: &'static str, per_label, per_help, att_label, att_help| {
        SectionBlock::Rows(vec![
            Row::flag(label, state.psg_volumes[i] != 0x0F).help("tone audible — attenuation < $F"),
            Row::value(per_label, format!("{:03X}", state.psg_periods[i])).help(per_help),
            Row::value(att_label, format!("{:X}", state.psg_volumes[i])).help(att_help),
        ])
    };
    let audible = (0..4).filter(|&i| state.psg_volumes[i] != 0x0F).count();
    Section {
        name: "PSG",
        summary: format!("{audible}/4 audible"),
        active: None,
        detail: None,
        blocks: vec![
            tone(
                0,
                "t0",
                "per0",
                "tone 0 period (10-bit)",
                "att0",
                "tone 0 attenuation ($F = mute)",
            ),
            SectionBlock::Rule,
            tone(
                1,
                "t1",
                "per1",
                "tone 1 period (10-bit)",
                "att1",
                "tone 1 attenuation ($F = mute)",
            ),
            SectionBlock::Rule,
            tone(
                2,
                "t2",
                "per2",
                "tone 2 period (10-bit)",
                "att2",
                "tone 2 attenuation ($F = mute)",
            ),
            SectionBlock::Rule,
            SectionBlock::Rows(vec![
                Row::flag("noise", state.psg_volumes[3] != 0x0F)
                    .help("noise audible — attenuation < $F"),
                Row::value("nctl", format!("{:X}", state.psg_noise))
                    .help("noise feedback mode & shift rate"),
                Row::value("att3", format!("{:X}", state.psg_volumes[3]))
                    .help("noise attenuation ($F = mute)"),
            ]),
        ],
    }
}

fn vdp_section(state: &Sg1000InspectState) -> Section {
    let registers = state
        .vdp_registers
        .iter()
        .enumerate()
        .map(|(index, &value)| Row::value(format!("r{index}"), format!("{value:02X}")))
        .collect();
    // The active display occupies the first 192 lines; the border and vblank
    // share the rest. The dot cycle within a line carries no named zones.
    let line = Sweep::new("line", state.line as u32, LINES_PER_FRAME)
        .zones(vec![
            SweepZone {
                name: "active",
                end: ACTIVE_LINES,
                tone: Tone::Rendering,
            },
            SweepZone {
                name: "blank",
                end: LINES_PER_FRAME,
                tone: Tone::Idle,
            },
        ])
        .help("VDP scanline — 0..191 active display, then border and vblank");
    let dot =
        Sweep::new("dot", state.dot as u32, DOTS_PER_LINE).help("VDP dot within the scanline");
    Section {
        name: "VDP",
        summary: format!("line {} · dot {}", state.line, state.dot),
        active: None,
        detail: None,
        blocks: vec![
            SectionBlock::Sweeps(vec![line, dot]),
            SectionBlock::Rule,
            SectionBlock::Table(status_table(state.vdp_status)),
            SectionBlock::Rows(vec![
                Row::value("5th", format!("{:02}", state.vdp_status & 0x1F))
                    .help("sprite number the scanner stopped at"),
            ]),
            SectionBlock::Rule,
            SectionBlock::Rows(registers),
        ],
    }
}

/// The status register's three flags; the low five bits carry the number of
/// the sprite the scan halted on.
fn status_table(status: u8) -> BitTable {
    BitTable {
        columns: vec![
            BitColumn::plain("f"),
            BitColumn::plain("5s"),
            BitColumn::plain("c"),
        ],
        corner: None,
        rows: vec![BitRow {
            name: "status",
            bits: vec![status & 0x80 != 0, status & 0x40 != 0, status & 0x20 != 0],
            tone: Tone::Neutral,
        }],
    }
}

/// SG-1000 media is recognised by its `.sg` file extension.
pub fn is_sg1000_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sg"))
}

pub struct Sg1000System;

impl SteppingSystem for Sg1000System {
    type Core = Sg1000;
    type Frame = Frame;
    type InspectState = Sg1000InspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);
    const RUN_BUDGET: u32 = 400_000;
    const PIXEL_ASPECT: f32 = PIXEL_ASPECT;

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

    fn take_frame(sg: &mut Sg1000) -> Option<Frame> {
        sg.take_frame()
    }

    fn step_frame(sg: &mut Sg1000) -> Option<Frame> {
        sg.step_frame(FRAME_BUDGET)
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

    fn indexed_frame(frame: &Frame) -> IndexedFrame {
        IndexedFrame {
            width: PIXELS_PER_LINE,
            height: ACTIVE_LINES,
            pixels: frame.0.as_flattened().into(),
            palette: ti_palette(),
        }
    }

    fn blank_frame() -> IndexedFrame {
        IndexedFrame::blank(PIXELS_PER_LINE, ACTIVE_LINES, ti_palette())
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
            bc: cpu.bc(),
            de: cpu.de(),
            hl: cpu.hl(),
            ix: cpu.ix,
            iy: cpu.iy,
            sp: cpu.sp,
            pc: cpu.pc,
            line: sg.vdp().line(),
            dot: sg.vdp().dot(),
            vdp_status: sg.vdp().peek_status(),
            vdp_registers: *sg.vdp().registers(),
            psg_periods: sg.psg().tone_periods(),
            psg_volumes: sg.psg().attenuations(),
            psg_noise: sg.psg().noise_control(),
            code_window,
            frame: frame_count,
        }
    }

    fn register_groups(state: &Sg1000InspectState) -> Vec<RegisterGroup> {
        cpu_register_groups(state)
    }

    fn sidebar_sections(state: &Sg1000InspectState) -> Vec<Section> {
        sg1000_sidebar_sections(state)
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

fn ti_palette() -> Arc<[RGB8]> {
    TI_PALETTE
        .iter()
        .map(|&[r, g, b]| RGB8::new(r, g, b))
        .collect()
}

const CONTROL_PAD: PeripheralId = PeripheralId(0);

/// Pause is a switch on the console itself, wired to /NMI rather than to a
/// controller line.
pub const PANEL: &[PanelControl] = &[PanelControl {
    role: ControlRole::Pause,
    label: "Pause",
    behaviour: PanelBehaviour::Momentary,
}];

const PAD_BUTTONS: &[ControlDescriptor] = &[
    button(ControlRole::Action(0), "Button 1"),
    button(ControlRole::Action(1), "Button 2"),
    button(ControlRole::Up, "Up"),
    button(ControlRole::Down, "Down"),
    button(ControlRole::Left, "Left"),
    button(ControlRole::Right, "Right"),
];

const fn button(role: ControlRole, label: &'static str) -> ControlDescriptor {
    ControlDescriptor {
        role,
        label,
        kind: ControlKind::Button,
    }
}

const fn pad_port(port: PortId, label: &'static str) -> PortDescriptor {
    PortDescriptor {
        port,
        label,
        accepts: &[PeripheralDescriptor {
            id: CONTROL_PAD,
            label: "Control pad",
            provider: Provider::Console,
            controls: PAD_BUTTONS,
        }],
    }
}

/// Both joystick sites the multiplexer pair presents. Player 1's stick is
/// wired to the board rather than to a connector, but it reads through the
/// same mux as player 2's.
pub const PORTS: &[PortDescriptor] = &[
    pad_port(JOY1, "Control pad 1"),
    pad_port(JOY2, "Control pad 2"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psg_section_reports_registers_and_audibility() {
        let mut state = Sg1000InspectState {
            psg_periods: [0x1FE, 0, 0],
            // Tone 0 audible (attenuation 0), the rest muted at $F.
            psg_volumes: [0x00, 0x0F, 0x0F, 0x0F],
            ..Sg1000InspectState::default()
        };
        state.psg_noise = 0x05;
        let section = psg_section(&state);
        assert_eq!(section.name, "PSG");

        let rows: Vec<&Row> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Rows(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(
            rows.iter().find(|r| r.label == "per0").map(|r| &r.value),
            Some(&"1FE".to_string())
        );
        assert_eq!(
            rows.iter().find(|r| r.label == "t0").and_then(|r| r.active),
            Some(true)
        );
        assert_eq!(
            rows.iter().find(|r| r.label == "t1").and_then(|r| r.active),
            Some(false)
        );
    }

    #[test]
    fn vdp_section_reports_all_eight_registers() {
        let state = Sg1000InspectState {
            vdp_registers: [0x00, 0x60, 0x0E, 0xFF, 0x03, 0x76, 0x03, 0x01],
            ..Sg1000InspectState::default()
        };
        let section = vdp_section(&state);
        let rows: Vec<&Row> = section
            .blocks
            .iter()
            .filter_map(|block| match block {
                SectionBlock::Rows(rows) => Some(rows),
                _ => None,
            })
            .flatten()
            .collect();
        for (index, value) in ["00", "60", "0E", "FF", "03", "76", "03", "01"]
            .into_iter()
            .enumerate()
        {
            let label = format!("r{index}");
            assert_eq!(
                rows.iter().find(|r| r.label == label).map(|r| &r.value),
                Some(&value.to_string())
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
