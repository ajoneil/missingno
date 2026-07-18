//! The SMS family's debugger seam: its inspection state and the
//! stepping-system binding over the console. One owned state struct serves
//! both the paused view (refreshed after every step) and the per-frame
//! snapshot the running view renders from.

use std::sync::Arc;
use std::time::Duration;

use missingno_core::inspect::{
    BitRow, BitTable, Register, RegisterGroup, Row, Section, SectionBlock, ValueStyle,
};
use missingno_core::stepping::SteppingSystem;
use missingno_core::system::{ControlId, ControlInput, DebugView, InspectSnapshot, RunningStatus};
use missingno_core::video::IndexedFrame;
use rgb::RGB8;

use crate::console::Sms;
use crate::vdp::{self, Frame};

/// Instruction budget per frame step; generous over the ~15k typical.
const FRAME_BUDGET: u32 = 200_000;

/// NTSC pixel aspect at the VDP's 5.37 MHz dot clock — a display-side
/// calibratable stage.
const PIXEL_ASPECT: f32 = 8.0 / 7.0;

const CODE_WINDOW_ROWS: usize = 10;

#[derive(Clone, Default)]
pub struct SmsInspectState {
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
    pub vdp_registers: [u8; 11],
    pub banks: [u8; 3],
    /// Raw bytes at the program counter, hex-dumped until a Z80
    /// disassembler lands.
    pub code_window: Vec<(u16, [u8; 4])>,
    pub frame: u64,
}

/// The per-frame snapshot for the running view.
pub struct SmsSnapshot {
    pub state: SmsInspectState,
}

impl SmsSnapshot {
    pub fn new(state: SmsInspectState) -> Self {
        SmsSnapshot { state }
    }
}

impl InspectSnapshot for SmsSnapshot {
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
        sms_sidebar_sections(&self.state)
    }
}

/// The Z80 register file as one inspection group, shared by the live view and
/// the running snapshot. `f` renders as a plain byte — no validated Z80 flag
/// table exists yet.
fn cpu_register_groups(state: &SmsInspectState) -> Vec<RegisterGroup> {
    let hex = |name, value: u32, bits| Register {
        name,
        value,
        bits,
        style: ValueStyle::Hex,
    };
    vec![RegisterGroup {
        name: "cpu",
        registers: vec![
            hex("a", state.a as u32, 8),
            hex("f", state.f as u32, 8),
            hex("bc", state.bc as u32, 16),
            hex("de", state.de as u32, 16),
            hex("hl", state.hl as u32, 16),
            hex("ix", state.ix as u32, 16),
            hex("iy", state.iy as u32, 16),
            hex("sp", state.sp as u32, 16),
            hex("pc", state.pc as u32, 16),
        ],
    }]
}

/// The SMS sidebar sections, shared by the live view and the running snapshot:
/// the Z80 register file plus the VDP's position, status, and registers.
fn sms_sidebar_sections(state: &SmsInspectState) -> Vec<Section> {
    let mut sections = missingno_core::inspect::default_sections(cpu_register_groups(state));
    sections.push(vdp_section(state));
    sections
}

fn vdp_section(state: &SmsInspectState) -> Section {
    let registers = state
        .vdp_registers
        .iter()
        .enumerate()
        .map(|(index, &value)| Row::value(format!("r{index}"), format!("{value:02X}")))
        .collect();
    Section {
        name: "VDP",
        summary: format!("line {} · dot {}", state.line, state.dot),
        active: None,
        detail: None,
        blocks: vec![
            SectionBlock::Rows(vec![
                Row::value("line", state.line.to_string()),
                Row::value("dot", state.dot.to_string()),
            ]),
            SectionBlock::Rule,
            SectionBlock::Table(status_table(state.vdp_status)),
            SectionBlock::Rule,
            SectionBlock::Rows(registers),
        ],
    }
}

/// The VDP status register's three documented flags; the low five bits carry
/// the fifth-sprite number.
fn status_table(status: u8) -> BitTable {
    BitTable {
        columns: &["int", "ovr", "col"],
        corner: None,
        rows: vec![BitRow {
            name: "status",
            bits: vec![status & 0x80 != 0, status & 0x40 != 0, status & 0x20 != 0],
        }],
    }
}

/// Master System media is recognised by its `.sms` file extension.
pub fn is_sms_rom(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("sms"))
}

pub struct SmsSystem;

impl SteppingSystem for SmsSystem {
    type Core = Sms;
    type Frame = Frame;
    type InspectState = SmsInspectState;

    /// One NTSC frame: 262 lines × 228 T-states at 3.579545 MHz.
    const FRAME_INTERVAL: Duration = Duration::from_micros(16_688);
    const RUN_BUDGET: u32 = 400_000;
    const PIXEL_ASPECT: f32 = PIXEL_ASPECT;

    fn pc(sms: &Sms) -> u16 {
        sms.cpu.pc
    }

    fn step_instruction(sms: &mut Sms) {
        sms.step_instruction();
    }

    fn take_frame(sms: &mut Sms) -> Option<Frame> {
        sms.take_frame()
    }

    fn step_frame(sms: &mut Sms) -> Option<Frame> {
        sms.step_frame(FRAME_BUDGET)
    }

    fn power_cycle(sms: &mut Sms) {
        sms.power_cycle();
    }

    fn apply_control(sms: &mut Sms, control: ControlId, input: ControlInput) {
        apply_control(sms, control, input);
    }

    fn drain_audio_samples(sms: &mut Sms) -> Vec<(f32, f32)> {
        sms.drain_audio_samples()
    }

    fn indexed_frame(frame: &Frame) -> IndexedFrame {
        IndexedFrame {
            width: vdp::PIXELS_PER_LINE as u32,
            height: vdp::ACTIVE_LINES as u32,
            pixels: frame.pixels.clone().into(),
            palette: cram_palette(&frame.cram),
            pixel_aspect: PIXEL_ASPECT,
        }
    }

    fn blank_frame() -> IndexedFrame {
        IndexedFrame::blank(
            vdp::PIXELS_PER_LINE as u32,
            vdp::ACTIVE_LINES as u32,
            PIXEL_ASPECT,
            cram_palette(&[0; 32]),
        )
    }

    fn step_over_target(sms: &Sms) -> Option<u16> {
        // CALL and RST push a return path; run to the next address.
        let opcode = sms.peek(sms.cpu.pc);
        let is_call = opcode == 0xCD || (opcode & 0xC7) == 0xC4;
        is_call.then(|| sms.cpu.pc.wrapping_add(3))
    }

    fn inspect(sms: &Sms, frame_count: u64) -> SmsInspectState {
        let cpu = &sms.cpu;
        let mut code_window = Vec::with_capacity(CODE_WINDOW_ROWS);
        let mut address = cpu.pc;
        for _ in 0..CODE_WINDOW_ROWS {
            code_window.push((
                address,
                [
                    sms.peek(address),
                    sms.peek(address.wrapping_add(1)),
                    sms.peek(address.wrapping_add(2)),
                    sms.peek(address.wrapping_add(3)),
                ],
            ));
            address = address.wrapping_add(4);
        }
        // The mapper latches mirror into RAM, which inspection can read.
        let banks = [0, 1, 2].map(|slot| sms.peek(0xFFFD + slot as u16));
        SmsInspectState {
            a: cpu.a,
            f: cpu.f,
            bc: cpu.bc(),
            de: cpu.de(),
            hl: cpu.hl(),
            ix: cpu.ix,
            iy: cpu.iy,
            sp: cpu.sp,
            pc: cpu.pc,
            line: sms.vdp.line(),
            dot: sms.vdp.dot(),
            vdp_status: sms.vdp.peek_status(),
            vdp_registers: sms.vdp.registers,
            banks,
            code_window,
            frame: frame_count,
        }
    }

    fn register_groups(state: &SmsInspectState) -> Vec<RegisterGroup> {
        cpu_register_groups(state)
    }

    fn sidebar_sections(state: &SmsInspectState) -> Vec<Section> {
        sms_sidebar_sections(state)
    }

    fn snapshot(state: &SmsInspectState, frame: u64) -> DebugView {
        let mut state = state.clone();
        state.frame = frame;
        Box::new(SmsSnapshot::new(state))
    }

    fn running_status(state: &SmsInspectState, frame: u64) -> RunningStatus {
        RunningStatus {
            pc: state.pc.into(),
            sp: state.sp.into(),
            video_label: "VDP",
            video_summary: format!("line {} · dot {}", state.line, state.dot),
            frame,
        }
    }
}

/// Resolve a CRAM snapshot (6-bit --BBGGRR) to display RGB.
fn cram_palette(cram: &[u8; 32]) -> Arc<[RGB8]> {
    cram.iter()
        .map(|&entry| {
            let channel = |bits: u8| (bits & 0x03) * 85;
            RGB8::new(channel(entry), channel(entry >> 2), channel(entry >> 4))
        })
        .collect()
}

/// The family's reading of the shared control ids: the pad maps onto the
/// port lines, and Start works the console Pause button (an NMI).
fn apply_control(sms: &mut Sms, control: ControlId, input: ControlInput) {
    let ControlInput::Digital(pressed) = input else {
        return;
    };
    let line = match control.0 {
        0 => {
            if pressed {
                sms.cpu.trigger_nmi();
            }
            return;
        }
        2 => 0x10, // button 1
        3 => 0x20, // button 2
        4 => 0x01, // up
        5 => 0x02, // down
        6 => 0x04, // left
        7 => 0x08, // right
        _ => return,
    };
    if pressed {
        sms.port_dc &= !line;
    } else {
        sms.port_dc |= line;
    }
}
